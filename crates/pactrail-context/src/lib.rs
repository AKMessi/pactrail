//! Provenance-aware repository context for Pactrail.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use pactrail_core::TaskContract;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_INDEX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_INSTRUCTION_BYTES: u64 = 256 * 1024;
const MAX_SYMBOLS_PER_FILE: usize = 2_000;
const MAX_ANCHOR_PREVIEW_BYTES: usize = 2 * 1024;
const MAX_CONTEXT_FRAGMENTS: usize = 64;
const MAX_CONTEXT_FRAGMENT_BYTES: usize = 64 * 1024;
const MAX_CONTEXT_FRAGMENTS_BYTES: usize = 256 * 1024;
const DEFAULT_CONTEXT_PACK_BYTES: usize = 128 * 1024;
const MIN_CONTEXT_PACK_BYTES: usize = 4 * 1024;
const MAX_CONTEXT_PACK_BYTES: usize = 512 * 1024;
const ESTIMATED_BYTES_PER_TOKEN: u64 = 3;
const CONTEXT_PACK_INPUT_SHARE: u64 = 4;
const TRUNCATION_NOTICE: &str =
    "\n\n[Context pack reached its model-derived budget. Use tools to inspect omitted sources.]";

/// Coarse language classification used by the context compiler.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Java,
    Kotlin,
    C,
    Cpp,
    CSharp,
    Ruby,
    Php,
    Shell,
    Markdown,
    Toml,
    Json,
    Yaml,
    Other,
}

/// One symbol-like declaration with source provenance.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Symbol {
    pub name: String,
    pub kind: String,
    pub line: usize,
}

/// Deterministic metadata extracted from one repository file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IndexedFile {
    pub path: String,
    pub digest: String,
    pub bytes: u64,
    pub lines: usize,
    pub language: Language,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<String>,
    /// Bounded current content for conventional project overview files.
    #[serde(default)]
    pub anchor_preview: Option<String>,
}

/// Hierarchical repository instruction file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InstructionFile {
    pub path: String,
    /// Virtual directory subtree governed by this file. `.` is repository-wide.
    pub scope: String,
    pub digest: String,
    pub content: String,
}

/// Conservative byte budget for one provider-neutral repository context pack.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContextBudget {
    max_bytes: usize,
}

impl ContextBudget {
    /// Derives a pack budget from the model's declared input and output limits.
    ///
    /// Pactrail reserves most of the window for system instructions, tool
    /// schemas, conversation growth, and tool results. Bytes are used as a
    /// deterministic provider-neutral upper bound; providers still perform the
    /// authoritative token count.
    #[must_use]
    pub fn from_model_limits(context_tokens: u64, max_output_tokens: u64) -> Self {
        let input_tokens = context_tokens.saturating_sub(max_output_tokens);
        let allocated_tokens = input_tokens / CONTEXT_PACK_INPUT_SHARE;
        let estimated_bytes = allocated_tokens.saturating_mul(ESTIMATED_BYTES_PER_TOKEN);
        Self {
            max_bytes: usize::try_from(estimated_bytes)
                .unwrap_or(MAX_CONTEXT_PACK_BYTES)
                .clamp(MIN_CONTEXT_PACK_BYTES, MAX_CONTEXT_PACK_BYTES),
        }
    }

    /// Creates an explicit bounded budget, primarily for embedders and tests.
    ///
    /// # Errors
    ///
    /// Returns an error outside Pactrail's supported deterministic bounds.
    pub fn new(max_bytes: usize) -> Result<Self, ContextError> {
        if !(MIN_CONTEXT_PACK_BYTES..=MAX_CONTEXT_PACK_BYTES).contains(&max_bytes) {
            return Err(ContextError::InvalidBudget {
                minimum: MIN_CONTEXT_PACK_BYTES,
                maximum: MAX_CONTEXT_PACK_BYTES,
                actual: max_bytes,
            });
        }
        Ok(Self { max_bytes })
    }

    /// Returns the deterministic rendered byte ceiling.
    #[must_use]
    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_CONTEXT_PACK_BYTES,
        }
    }
}

/// Bounded context supplied by another provenance-aware subsystem.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContextFragment {
    /// Stable source label shown to the model, such as a memory identifier.
    pub source: String,
    /// Advisory content. Fragments never override the task contract or repository instructions.
    pub content: String,
}

/// Stable repository topology and provenance index.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RepositoryIndex {
    pub files: BTreeMap<String, IndexedFile>,
    pub instructions: Vec<InstructionFile>,
    pub languages: BTreeMap<Language, usize>,
    pub digest: String,
}

impl RepositoryIndex {
    /// Builds an index without invoking a compiler, language server, or model.
    ///
    /// # Errors
    ///
    /// Returns an error when traversal or required file access fails. Oversized
    /// and non-UTF-8 files remain in topology but are not semantically scanned.
    pub fn build(root: &Path) -> Result<Self, ContextError> {
        let mut files = BTreeMap::new();
        let mut instructions = Vec::new();
        let mut languages = BTreeMap::new();
        for item in WalkBuilder::new(root)
            .hidden(false)
            .git_ignore(true)
            .git_global(false)
            .filter_entry(|entry| {
                let name = entry.file_name().to_string_lossy();
                name != ".git" && name != ".pactrail"
            })
            .build()
        {
            let entry = item.map_err(ContextError::Walk)?;
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let path = entry.path();
            let relative = portable_relative(root, path)?;
            let (digest, size, retained) = fingerprint_and_retain(path)?;
            let language = detect_language(path);
            *languages.entry(language).or_insert(0) += 1;
            let text = retained
                .as_deref()
                .and_then(|bytes| std::str::from_utf8(bytes).ok());
            let (lines, symbols, imports) = text.map_or((0, Vec::new(), Vec::new()), |text| {
                let lines = text.lines().count();
                let (symbols, imports) = extract_structure(text, language);
                (lines, symbols, imports)
            });
            let anchor_preview = text
                .filter(|_| repository_anchor_rank(&relative).is_some())
                .map(|text| utf8_prefix(text, MAX_ANCHOR_PREVIEW_BYTES).to_owned());
            if path.file_name().is_some_and(|name| name == "AGENTS.md")
                && size <= MAX_INSTRUCTION_BYTES
                && let Some(content) = text
            {
                instructions.push(InstructionFile {
                    path: relative.clone(),
                    scope: instruction_scope(&relative),
                    digest: digest.clone(),
                    content: content.to_owned(),
                });
            }
            files.insert(
                relative.clone(),
                IndexedFile {
                    path: relative,
                    digest,
                    bytes: size,
                    lines,
                    language,
                    symbols,
                    imports,
                    anchor_preview,
                },
            );
        }
        instructions.sort_by(|left, right| left.path.cmp(&right.path));
        let digest = index_digest(&files, &instructions);
        Ok(Self {
            files,
            instructions,
            languages,
            digest,
        })
    }

    /// Retrieves files by lexical overlap with a task, with stable tie-breaking.
    ///
    /// When task wording does not name any repository concepts (for example,
    /// "what is this directory about?"), the result falls back to a small set
    /// of conventional project anchors. This keeps broad discovery useful
    /// without flooding the model context with arbitrary paths.
    #[must_use]
    pub fn retrieve(&self, query: &str, limit: usize) -> Vec<&IndexedFile> {
        if limit == 0 {
            return Vec::new();
        }
        let tokens = query_tokens(query);
        let mut scored = self
            .files
            .values()
            .map(|file| {
                let path = file.path.to_lowercase();
                let symbol_text = file
                    .symbols
                    .iter()
                    .map(|symbol| symbol.name.to_lowercase())
                    .collect::<Vec<_>>()
                    .join(" ");
                let score = tokens
                    .iter()
                    .map(|token| {
                        usize::from(path.contains(token)) * 4
                            + usize::from(symbol_text.contains(token)) * 3
                    })
                    .sum::<usize>();
                (score, file)
            })
            .collect::<Vec<_>>();
        scored.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.path.cmp(&right.path))
        });
        let mut retrieved = scored
            .into_iter()
            .filter(|(score, _)| *score > 0)
            .take(limit)
            .map(|(_, file)| file)
            .collect::<Vec<_>>();
        if retrieved.is_empty() {
            let mut anchors = self
                .files
                .values()
                .filter_map(|file| repository_anchor_rank(&file.path).map(|rank| (rank, file)))
                .collect::<Vec<_>>();
            anchors.sort_by(|(left_rank, left), (right_rank, right)| {
                left_rank
                    .cmp(right_rank)
                    .then_with(|| left.path.cmp(&right.path))
            });
            retrieved.extend(anchors.into_iter().take(limit.min(8)).map(|(_, file)| file));
        }
        retrieved
    }
}

fn repository_anchor_rank(path: &str) -> Option<u8> {
    let normalized = path.to_ascii_lowercase();
    let file_name = normalized.rsplit('/').next().unwrap_or(&normalized);
    match file_name {
        "readme" | "readme.md" | "readme.mdx" | "readme.rst" | "readme.txt" => Some(0),
        "cargo.toml" | "package.json" | "pyproject.toml" | "go.mod" | "pom.xml"
        | "build.gradle" | "build.gradle.kts" | "mix.exs" | "composer.json" => Some(1),
        _ => match normalized.as_str() {
            "src/main.rs" | "src/lib.rs" | "main.py" | "app.py" | "src/index.ts"
            | "src/index.js" | "cmd/main.go" => Some(2),
            _ => None,
        },
    }
}

fn fingerprint_and_retain(path: &Path) -> Result<(String, u64, Option<Vec<u8>>), ContextError> {
    let metadata = fs::metadata(path).map_err(|source| ContextError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let size = metadata.len();
    let mut retained = (size <= MAX_INDEX_FILE_BYTES)
        .then(|| Vec::with_capacity(usize::try_from(size).unwrap_or_default()));
    let mut file = fs::File::open(path).map_err(|source| ContextError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let count = file.read(&mut buffer).map_err(|source| ContextError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        if let Some(bytes) = &mut retained {
            bytes.extend_from_slice(&buffer[..count]);
        }
    }
    Ok((hasher.finalize().to_hex().to_string(), size, retained))
}

fn utf8_prefix(value: &str, max_bytes: usize) -> &str {
    let mut boundary = value.len().min(max_bytes);
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &value[..boundary]
}

/// Bounded provider-neutral context compiled for one task.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextPack {
    pub repository_digest: String,
    pub rendered: String,
    pub cited_files: Vec<String>,
    pub included_instructions: Vec<String>,
    pub included_fragments: Vec<String>,
    pub rendered_bytes: usize,
    pub budget_bytes: usize,
    pub truncated: bool,
}

impl ContextPack {
    /// Compiles task contract, repository topology, instructions, and retrieved symbols.
    ///
    /// # Errors
    ///
    /// Returns an error if the contract cannot be serialized.
    pub fn compile(contract: &TaskContract, index: &RepositoryIndex) -> Result<Self, ContextError> {
        Self::compile_with_budget(contract, index, &[], ContextBudget::default())
    }

    /// Compiles task and repository context with bounded, provenance-labelled fragments.
    ///
    /// # Errors
    ///
    /// Returns an error if the contract cannot be serialized or a fragment
    /// exceeds the count, per-fragment, or aggregate safety bound.
    pub fn compile_with_fragments(
        contract: &TaskContract,
        index: &RepositoryIndex,
        fragments: &[ContextFragment],
    ) -> Result<Self, ContextError> {
        Self::compile_with_budget(contract, index, fragments, ContextBudget::default())
    }

    /// Compiles context under an explicit deterministic byte budget.
    ///
    /// The task contract and root `AGENTS.md` are required and fail closed when
    /// they cannot fit. Scoped instructions, memories, and topology are added
    /// as complete provenance-labelled entries in priority order; Pactrail
    /// never cuts an instruction in the middle.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid fragments or when authoritative context
    /// cannot fit in the supplied budget.
    pub fn compile_with_budget(
        contract: &TaskContract,
        index: &RepositoryIndex,
        fragments: &[ContextFragment],
        budget: ContextBudget,
    ) -> Result<Self, ContextError> {
        validate_fragments(fragments)?;
        let mut model_contract = contract.clone();
        ".".clone_into(&mut model_contract.workspace_root);
        let contract_json =
            serde_json::to_string_pretty(&model_contract).map_err(ContextError::Serialization)?;
        let retrieved = index.retrieve(&contract.goal, 40);
        let language_summary = index
            .languages
            .iter()
            .map(|(language, count)| format!("{language:?}: {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        let root_instruction = index
            .instructions
            .iter()
            .find(|instruction| instruction.scope == ".");
        let global_instructions = root_instruction.map_or_else(
            || "No repository-wide AGENTS.md file was found.".to_owned(),
            |instruction| {
                format!(
                    "### {} [scope: repository root]\n{}",
                    instruction.path,
                    instruction.content.trim()
                )
            },
        );
        let required = format!(
            "# Task contract\n{contract_json}\n\nThe model-visible workspace root is `.`. Every tool path must be relative to it; host paths are unavailable.\n\n# Repository\nDigest: {}\nFiles: {}\nLanguages: {language_summary}\n\n# Repository-wide instructions\n{global_instructions}\n\nNested AGENTS.md files apply only to files beneath their declared virtual directory scope.\n",
            index.digest,
            index.files.len(),
        );
        let mut writer = PackWriter::new(budget.max_bytes);
        writer.push_required(&required)?;
        let mut included_instructions = root_instruction
            .map(|instruction| vec![instruction.path.clone()])
            .unwrap_or_default();
        append_scoped_instructions(&mut writer, index, &mut included_instructions);
        let mut included_fragments = Vec::new();
        append_fragments(&mut writer, fragments, &mut included_fragments);
        let mut cited_files = Vec::new();
        append_topology(&mut writer, &retrieved, &mut cited_files);
        let (rendered, truncated) = writer.finish();
        let rendered_bytes = rendered.len();
        Ok(Self {
            repository_digest: index.digest.clone(),
            rendered,
            cited_files,
            included_instructions,
            included_fragments,
            rendered_bytes,
            budget_bytes: budget.max_bytes,
            truncated,
        })
    }
}

struct PackWriter {
    rendered: String,
    max_bytes: usize,
    truncated: bool,
}

impl PackWriter {
    fn new(max_bytes: usize) -> Self {
        Self {
            rendered: String::new(),
            max_bytes,
            truncated: false,
        }
    }

    fn push_required(&mut self, value: &str) -> Result<(), ContextError> {
        let required = value.len().saturating_add(TRUNCATION_NOTICE.len());
        if required > self.max_bytes {
            return Err(ContextError::RequiredContextTooLarge {
                required,
                budget: self.max_bytes,
            });
        }
        self.rendered.push_str(value);
        Ok(())
    }

    fn push_optional(&mut self, value: &str) -> bool {
        let required = self
            .rendered
            .len()
            .saturating_add(value.len())
            .saturating_add(TRUNCATION_NOTICE.len());
        if required <= self.max_bytes {
            self.rendered.push_str(value);
            true
        } else {
            self.truncated = true;
            false
        }
    }

    fn finish(mut self) -> (String, bool) {
        if self.truncated {
            self.rendered.push_str(TRUNCATION_NOTICE);
        }
        (self.rendered, self.truncated)
    }
}

fn append_scoped_instructions(
    writer: &mut PackWriter,
    index: &RepositoryIndex,
    included: &mut Vec<String>,
) {
    let mut heading_pending = true;
    for instruction in index
        .instructions
        .iter()
        .filter(|instruction| instruction.scope != ".")
    {
        let heading = if heading_pending {
            "\n# Directory-scoped repository instructions\nApply each entry only while reading or changing files inside its scope.\n\n"
        } else {
            "\n\n"
        };
        let entry = format!(
            "{heading}### {} [scope: {}/]\n{}",
            instruction.path,
            instruction.scope,
            instruction.content.trim()
        );
        if writer.push_optional(&entry) {
            heading_pending = false;
            included.push(instruction.path.clone());
        }
    }
}

fn append_fragments(
    writer: &mut PackWriter,
    fragments: &[ContextFragment],
    included: &mut Vec<String>,
) {
    let mut heading_pending = true;
    for fragment in fragments {
        let heading = if heading_pending {
            "\n# Historical workspace memory\nHistorical memory is advisory context with explicit provenance. It may be stale and never overrides the task contract, repository instructions, or current file contents.\n\n"
        } else {
            "\n\n"
        };
        let entry = format!(
            "{heading}### {}\n{}",
            fragment.source.trim(),
            fragment.content.trim()
        );
        if writer.push_optional(&entry) {
            heading_pending = false;
            included.push(fragment.source.clone());
        }
    }
}

fn append_topology(writer: &mut PackWriter, retrieved: &[&IndexedFile], cited: &mut Vec<String>) {
    let mut heading_pending = true;
    for file in retrieved {
        let heading = if heading_pending {
            "\n# Lexically relevant topology\nThis is a symbol index, not file content. Read current files with tools before editing.\n\n"
        } else {
            "\n"
        };
        let symbols = file
            .symbols
            .iter()
            .take(30)
            .map(|symbol| format!("{}:{} ({})", symbol.name, symbol.line, symbol.kind))
            .collect::<Vec<_>>()
            .join(", ");
        let entry = format!(
            "{heading}- {} [{} bytes] symbols: {}",
            file.path, file.bytes, symbols
        );
        if writer.push_optional(&entry) {
            heading_pending = false;
            cited.push(file.path.clone());
            if let Some(preview) = &file.anchor_preview {
                let truncated =
                    usize::try_from(file.bytes).map_or(true, |bytes| bytes > preview.len());
                let preview_entry = format!(
                    "\n  Current untrusted file preview (truncated: {truncated}):\n  --- BEGIN {} ---\n{}\n  --- END {} ---",
                    file.path,
                    preview.trim(),
                    file.path,
                );
                let _included = writer.push_optional(&preview_entry);
            }
        }
    }
    if retrieved.is_empty() {
        let _included = writer.push_optional(
            "\n# Lexically relevant topology\nNo files ranked from the task text; use list_files and search to investigate.",
        );
    }
}

fn validate_fragments(fragments: &[ContextFragment]) -> Result<(), ContextError> {
    if fragments.len() > MAX_CONTEXT_FRAGMENTS {
        return Err(ContextError::InvalidFragment(format!(
            "context accepts at most {MAX_CONTEXT_FRAGMENTS} supplemental fragments"
        )));
    }
    let mut total = 0_usize;
    for fragment in fragments {
        if fragment.source.trim().is_empty() || fragment.source.chars().any(char::is_control) {
            return Err(ContextError::InvalidFragment(
                "fragment sources must be non-empty and contain no control characters".to_owned(),
            ));
        }
        if fragment.content.len() > MAX_CONTEXT_FRAGMENT_BYTES {
            return Err(ContextError::InvalidFragment(format!(
                "one context fragment exceeds {MAX_CONTEXT_FRAGMENT_BYTES} bytes"
            )));
        }
        total = total
            .checked_add(fragment.content.len())
            .ok_or_else(|| ContextError::InvalidFragment("fragment size overflowed".to_owned()))?;
    }
    if total > MAX_CONTEXT_FRAGMENTS_BYTES {
        return Err(ContextError::InvalidFragment(format!(
            "supplemental context exceeds {MAX_CONTEXT_FRAGMENTS_BYTES} bytes"
        )));
    }
    Ok(())
}

fn extract_structure(text: &str, language: Language) -> (Vec<Symbol>, Vec<String>) {
    let mut symbols = Vec::new();
    let mut imports = Vec::new();
    for (index, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if let Some(import) = extract_import(line, language) {
            imports.push(import);
        }
        if symbols.len() < MAX_SYMBOLS_PER_FILE
            && let Some((kind, name)) = extract_symbol(line, language)
        {
            symbols.push(Symbol {
                name,
                kind,
                line: index + 1,
            });
        }
    }
    imports.sort();
    imports.dedup();
    (symbols, imports)
}

fn extract_symbol(line: &str, language: Language) -> Option<(String, String)> {
    let prefixes: &[(&str, &str)] = match language {
        Language::Rust => &[
            ("pub fn ", "function"),
            ("fn ", "function"),
            ("pub struct ", "struct"),
            ("struct ", "struct"),
            ("pub enum ", "enum"),
            ("enum ", "enum"),
            ("trait ", "trait"),
        ],
        Language::Python => &[
            ("def ", "function"),
            ("async def ", "function"),
            ("class ", "class"),
        ],
        Language::Go => &[("func ", "function"), ("type ", "type")],
        Language::TypeScript | Language::JavaScript => &[
            ("function ", "function"),
            ("export function ", "function"),
            ("class ", "class"),
            ("export class ", "class"),
            ("interface ", "interface"),
            ("export interface ", "interface"),
        ],
        Language::Java | Language::Kotlin | Language::CSharp => &[
            ("class ", "class"),
            ("public class ", "class"),
            ("interface ", "interface"),
            ("public interface ", "interface"),
        ],
        _ => &[],
    };
    for (prefix, kind) in prefixes {
        if let Some(rest) = line.strip_prefix(prefix) {
            let name = rest
                .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
                .next()
                .unwrap_or_default();
            if !name.is_empty() {
                return Some(((*kind).to_owned(), name.to_owned()));
            }
        }
    }
    None
}

fn extract_import(line: &str, language: Language) -> Option<String> {
    let is_import = match language {
        Language::Rust => line.starts_with("use ") || line.starts_with("mod "),
        Language::Python => line.starts_with("import ") || line.starts_with("from "),
        Language::TypeScript | Language::JavaScript => {
            line.starts_with("import ") || line.contains("require(")
        }
        Language::Go | Language::Java | Language::Kotlin => line.starts_with("import "),
        _ => false,
    };
    is_import.then(|| line.chars().take(500).collect())
}

fn query_tokens(query: &str) -> BTreeSet<String> {
    query
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .map(str::to_lowercase)
        .filter(|token| token.len() >= 3)
        .collect()
}

fn index_digest(files: &BTreeMap<String, IndexedFile>, instructions: &[InstructionFile]) -> String {
    let mut hasher = blake3::Hasher::new();
    for file in files.values() {
        hasher.update(file.path.as_bytes());
        hasher.update(file.digest.as_bytes());
    }
    for instruction in instructions {
        hasher.update(instruction.path.as_bytes());
        hasher.update(instruction.digest.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn instruction_scope(relative: &str) -> String {
    relative
        .rsplit_once('/')
        .map_or_else(|| ".".to_owned(), |(directory, _)| directory.to_owned())
}

fn portable_relative(root: &Path, path: &Path) -> Result<String, ContextError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| ContextError::EscapedRoot(path.to_path_buf()))?;
    relative
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| ContextError::NonUnicodePath(path.to_path_buf()))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|components| components.join("/"))
}

fn detect_language(path: &Path) -> Language {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => Language::Rust,
        "ts" | "tsx" => Language::TypeScript,
        "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
        "py" | "pyi" => Language::Python,
        "go" => Language::Go,
        "java" => Language::Java,
        "kt" | "kts" => Language::Kotlin,
        "c" | "h" => Language::C,
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => Language::Cpp,
        "cs" => Language::CSharp,
        "rb" => Language::Ruby,
        "php" => Language::Php,
        "sh" | "bash" | "zsh" | "ps1" => Language::Shell,
        "md" | "mdx" => Language::Markdown,
        "toml" => Language::Toml,
        "json" | "jsonc" => Language::Json,
        "yaml" | "yml" => Language::Yaml,
        _ => Language::Other,
    }
}

/// Repository context construction failure.
#[derive(Debug, Error)]
pub enum ContextError {
    #[error("repository traversal failed: {0}")]
    Walk(ignore::Error),
    #[error("repository I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("repository path escaped the root: {0}")]
    EscapedRoot(PathBuf),
    #[error("non-Unicode repository path is unsupported: {0}")]
    NonUnicodePath(PathBuf),
    #[error("context serialization failed: {0}")]
    Serialization(serde_json::Error),
    #[error("context budget must be between {minimum} and {maximum} bytes, got {actual}")]
    InvalidBudget {
        minimum: usize,
        maximum: usize,
        actual: usize,
    },
    #[error(
        "authoritative task and root instructions require {required} bytes, exceeding the {budget}-byte context budget"
    )]
    RequiredContextTooLarge { required: usize, budget: usize },
    #[error("supplemental context is invalid: {0}")]
    InvalidFragment(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_symbols_in_stable_order() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        fs::create_dir(root.path().join("src"))
            .unwrap_or_else(|error| unreachable!("src: {error}"));
        fs::write(
            root.path().join("src/lib.rs"),
            "use std::fmt;\npub struct Receipt;\npub fn verify() {}\n",
        )
        .unwrap_or_else(|error| unreachable!("fixture: {error}"));
        let index = RepositoryIndex::build(root.path())
            .unwrap_or_else(|error| unreachable!("index: {error}"));
        let file = &index.files["src/lib.rs"];
        assert_eq!(file.language, Language::Rust);
        assert_eq!(file.symbols[0].name, "Receipt");
        assert_eq!(file.symbols[1].name, "verify");
        assert_eq!(file.imports, vec!["use std::fmt;"]);
    }

    #[test]
    fn retrieved_files_are_task_relevant() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        fs::write(root.path().join("receipt.rs"), "pub fn verify_receipt() {}")
            .unwrap_or_else(|error| unreachable!("receipt: {error}"));
        fs::write(root.path().join("unrelated.rs"), "pub fn other() {}")
            .unwrap_or_else(|error| unreachable!("other: {error}"));
        let index = RepositoryIndex::build(root.path())
            .unwrap_or_else(|error| unreachable!("index: {error}"));
        let retrieved = index.retrieve("fix receipt verification", 10);
        assert_eq!(
            retrieved.first().map(|file| file.path.as_str()),
            Some("receipt.rs")
        );
    }

    #[test]
    fn broad_repository_questions_fall_back_to_project_anchors() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        fs::create_dir(root.path().join("src"))
            .unwrap_or_else(|error| unreachable!("src: {error}"));
        fs::write(root.path().join("README.md"), "# Example\n")
            .unwrap_or_else(|error| unreachable!("readme: {error}"));
        fs::write(
            root.path().join("Cargo.toml"),
            "[package]\nname = \"example\"\n",
        )
        .unwrap_or_else(|error| unreachable!("manifest: {error}"));
        fs::write(root.path().join("src/lib.rs"), "pub fn example() {}\n")
            .unwrap_or_else(|error| unreachable!("library: {error}"));
        fs::write(root.path().join("notes.txt"), "unrelated\n")
            .unwrap_or_else(|error| unreachable!("notes: {error}"));

        let index = RepositoryIndex::build(root.path())
            .unwrap_or_else(|error| unreachable!("index: {error}"));
        let paths = index
            .retrieve("whats this directory about", 10)
            .into_iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>();

        assert_eq!(paths, vec!["README.md", "Cargo.toml", "src/lib.rs"]);
    }

    #[test]
    fn broad_context_contains_bounded_current_anchor_evidence() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        fs::create_dir(root.path().join("src"))
            .unwrap_or_else(|error| unreachable!("src: {error}"));
        fs::write(
            root.path().join("Cargo.toml"),
            "[package]\nname = \"grounded-example\"\n",
        )
        .unwrap_or_else(|error| unreachable!("manifest: {error}"));
        fs::write(root.path().join("src/lib.rs"), "pub fn grounded() {}\n")
            .unwrap_or_else(|error| unreachable!("library: {error}"));
        let index = RepositoryIndex::build(root.path())
            .unwrap_or_else(|error| unreachable!("index: {error}"));
        let contract = TaskContract::new("whats this directory about", ".");
        let budget =
            ContextBudget::new(4 * 1024).unwrap_or_else(|error| unreachable!("budget: {error}"));

        let context = ContextPack::compile_with_budget(&contract, &index, &[], budget)
            .unwrap_or_else(|error| unreachable!("context: {error}"));

        assert!(context.rendered.contains("Current untrusted file preview"));
        assert!(context.rendered.contains("name = \"grounded-example\""));
        assert!(context.rendered.contains("pub fn grounded()"));
        assert!(context.rendered_bytes <= budget.max_bytes());
    }

    #[test]
    fn oversized_files_are_hashed_without_semantic_retention() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        let path = root.path().join("large.rs");
        let bytes = vec![b'x'; usize::try_from(MAX_INDEX_FILE_BYTES).unwrap_or_default() + 1];
        fs::write(&path, &bytes).unwrap_or_else(|error| unreachable!("large fixture: {error}"));

        let index = RepositoryIndex::build(root.path())
            .unwrap_or_else(|error| unreachable!("index: {error}"));
        let file = &index.files["large.rs"];
        assert_eq!(file.bytes, MAX_INDEX_FILE_BYTES + 1);
        assert_eq!(file.lines, 0);
        assert!(file.symbols.is_empty());
        assert_eq!(file.digest, blake3::hash(&bytes).to_hex().to_string());
    }

    #[test]
    fn model_context_virtualizes_the_workspace_root() {
        let host_root = r"C:\Users\private\project";
        let contract = TaskContract::new("Create a file", host_root);
        let context = ContextPack::compile(&contract, &RepositoryIndex::default())
            .unwrap_or_else(|error| unreachable!("context: {error}"));

        assert!(!context.rendered.contains(host_root));
        assert!(context.rendered.contains(r#""workspace_root": ".""#));
        assert!(
            context
                .rendered
                .contains("Every tool path must be relative")
        );
    }

    #[test]
    fn supplemental_context_is_provenance_labelled_and_bounded() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        fs::write(root.path().join("lib.rs"), "pub fn run() {}\n")
            .unwrap_or_else(|error| unreachable!("fixture: {error}"));
        let index = RepositoryIndex::build(root.path())
            .unwrap_or_else(|error| unreachable!("index: {error}"));
        let contract = TaskContract::new("change run", root.path().display().to_string());
        let context = ContextPack::compile_with_fragments(
            &contract,
            &index,
            &[ContextFragment {
                source: "memory:123 [decision]".to_owned(),
                content: "Keep this API synchronous.".to_owned(),
            }],
        )
        .unwrap_or_else(|error| unreachable!("context: {error}"));
        assert!(context.rendered.contains("# Historical workspace memory"));
        assert!(context.rendered.contains("memory:123 [decision]"));
        assert!(context.rendered.contains("advisory context"));

        let oversized = ContextPack::compile_with_fragments(
            &contract,
            &index,
            &[ContextFragment {
                source: "memory:oversized".to_owned(),
                content: "x".repeat(MAX_CONTEXT_FRAGMENT_BYTES + 1),
            }],
        );
        assert!(matches!(oversized, Err(ContextError::InvalidFragment(_))));
    }

    #[test]
    fn nested_instructions_are_explicitly_directory_scoped() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        fs::create_dir_all(root.path().join("crates/api"))
            .unwrap_or_else(|error| unreachable!("directories: {error}"));
        fs::write(root.path().join("AGENTS.md"), "Global rule.")
            .unwrap_or_else(|error| unreachable!("root instructions: {error}"));
        fs::write(
            root.path().join("crates/api/AGENTS.md"),
            "API subtree rule.",
        )
        .unwrap_or_else(|error| unreachable!("scoped instructions: {error}"));
        let index = RepositoryIndex::build(root.path())
            .unwrap_or_else(|error| unreachable!("index: {error}"));
        let contract = TaskContract::new("change api", root.path().display().to_string());
        let context = ContextPack::compile(&contract, &index)
            .unwrap_or_else(|error| unreachable!("context: {error}"));

        assert_eq!(index.instructions[0].scope, ".");
        assert_eq!(index.instructions[1].scope, "crates/api");
        assert!(
            context
                .rendered
                .contains("AGENTS.md [scope: repository root]")
        );
        assert!(context.rendered.contains("[scope: crates/api/]"));
        assert!(context.rendered.contains("apply only"));
    }

    #[test]
    fn model_budget_omits_whole_optional_entries_with_a_visible_notice() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        for index in 0..30 {
            fs::write(
                root.path().join(format!("memory_{index}.rs")),
                format!("pub fn memory_{index}() {{}}\n"),
            )
            .unwrap_or_else(|error| unreachable!("fixture: {error}"));
        }
        let index = RepositoryIndex::build(root.path())
            .unwrap_or_else(|error| unreachable!("index: {error}"));
        let contract = TaskContract::new("change memory", root.path().display().to_string());
        let fragments = (0..20)
            .map(|index| ContextFragment {
                source: format!("memory:{index}"),
                content: "advisory evidence ".repeat(40),
            })
            .collect::<Vec<_>>();
        let budget = ContextBudget::new(MIN_CONTEXT_PACK_BYTES)
            .unwrap_or_else(|error| unreachable!("budget: {error}"));
        let context = ContextPack::compile_with_budget(&contract, &index, &fragments, budget)
            .unwrap_or_else(|error| unreachable!("context: {error}"));

        assert!(context.truncated);
        assert!(context.rendered_bytes <= budget.max_bytes);
        assert!(
            context
                .rendered
                .contains("reached its model-derived budget")
        );
        assert!(context.included_fragments.len() < fragments.len());
    }

    #[test]
    fn oversized_authoritative_instructions_fail_closed() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        fs::write(root.path().join("AGENTS.md"), "x".repeat(8 * 1024))
            .unwrap_or_else(|error| unreachable!("instructions: {error}"));
        let index = RepositoryIndex::build(root.path())
            .unwrap_or_else(|error| unreachable!("index: {error}"));
        let contract = TaskContract::new("change file", root.path().display().to_string());
        let budget = ContextBudget::new(MIN_CONTEXT_PACK_BYTES)
            .unwrap_or_else(|error| unreachable!("budget: {error}"));
        let context = ContextPack::compile_with_budget(&contract, &index, &[], budget);

        assert!(matches!(
            context,
            Err(ContextError::RequiredContextTooLarge { .. })
        ));
    }
}
