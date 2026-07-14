//! Provenance-aware repository context for Pactrail.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use pactrail_core::TaskContract;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_INDEX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_INSTRUCTION_BYTES: u64 = 256 * 1024;
const MAX_SYMBOLS_PER_FILE: usize = 2_000;

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
}

/// Hierarchical repository instruction file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InstructionFile {
    pub path: String,
    pub digest: String,
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
            let bytes = fs::read(path).map_err(|source| ContextError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            let digest = blake3::hash(&bytes).to_hex().to_string();
            let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            let language = detect_language(path);
            *languages.entry(language).or_insert(0) += 1;
            let text = if size <= MAX_INDEX_FILE_BYTES {
                std::str::from_utf8(&bytes).ok()
            } else {
                None
            };
            let (lines, symbols, imports) = text.map_or((0, Vec::new(), Vec::new()), |text| {
                let lines = text.lines().count();
                let (symbols, imports) = extract_structure(text, language);
                (lines, symbols, imports)
            });
            if path.file_name().is_some_and(|name| name == "AGENTS.md")
                && size <= MAX_INSTRUCTION_BYTES
                && let Some(content) = text
            {
                instructions.push(InstructionFile {
                    path: relative.clone(),
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
    #[must_use]
    pub fn retrieve(&self, query: &str, limit: usize) -> Vec<&IndexedFile> {
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
        scored
            .into_iter()
            .filter(|(score, _)| *score > 0)
            .take(limit)
            .map(|(_, file)| file)
            .collect()
    }
}

/// Bounded provider-neutral context compiled for one task.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextPack {
    pub repository_digest: String,
    pub rendered: String,
    pub cited_files: Vec<String>,
}

impl ContextPack {
    /// Compiles task contract, repository topology, instructions, and retrieved symbols.
    ///
    /// # Errors
    ///
    /// Returns an error if the contract cannot be serialized.
    pub fn compile(contract: &TaskContract, index: &RepositoryIndex) -> Result<Self, ContextError> {
        let contract_json =
            serde_json::to_string_pretty(contract).map_err(ContextError::Serialization)?;
        let retrieved = index.retrieve(&contract.goal, 40);
        let cited_files = retrieved
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        let language_summary = index
            .languages
            .iter()
            .map(|(language, count)| format!("{language:?}: {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        let relevant = retrieved
            .iter()
            .map(|file| {
                let symbols = file
                    .symbols
                    .iter()
                    .take(30)
                    .map(|symbol| format!("{}:{} ({})", symbol.name, symbol.line, symbol.kind))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "- {} [{} bytes] symbols: {}",
                    file.path, file.bytes, symbols
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let instructions = index
            .instructions
            .iter()
            .map(|instruction| {
                format!("### {}\n{}\n", instruction.path, instruction.content.trim())
            })
            .collect::<Vec<_>>()
            .join("\n");
        let rendered = format!(
            "# Task contract\n{contract_json}\n\n# Repository\nDigest: {}\nFiles: {}\nLanguages: {language_summary}\n\n# Applicable repository instructions\n{}\n# Lexically relevant topology\n{}",
            index.digest,
            index.files.len(),
            if instructions.is_empty() {
                "No AGENTS.md files were found.\n".to_owned()
            } else {
                instructions
            },
            if relevant.is_empty() {
                "No files ranked from the task text; use list_files and search to investigate."
                    .to_owned()
            } else {
                relevant
            }
        );
        Ok(Self {
            repository_digest: index.digest.clone(),
            rendered,
            cited_files,
        })
    }
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
}
