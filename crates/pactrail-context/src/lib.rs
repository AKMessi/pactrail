//! Provenance-aware repository context for Pactrail.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use pactrail_core::TaskContract;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_INDEX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_INSTRUCTION_BYTES: u64 = 256 * 1024;
const MAX_SYMBOLS_PER_FILE: usize = 2_000;
const MAX_IMPORTS_PER_FILE: usize = 4_096;
const MAX_IDENTIFIER_OCCURRENCES_PER_FILE: usize = 100_000;
const MAX_GRAPH_DEFINITIONS: usize = 200_000;
const MAX_GRAPH_REFERENCES: usize = 500_000;
const MAX_REFERENCES_PER_SYMBOL: usize = 256;
const MAX_IDENTIFIER_BYTES: usize = 128;
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
const INDEX_CACHE_SCHEMA_VERSION: u32 = 1;
const MAX_INDEX_CACHE_ENTRY_BYTES: u64 = 16 * 1024 * 1024;

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

/// One project-defined symbol location in the repository evidence graph.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SymbolLocation {
    pub path: String,
    pub line: usize,
    pub kind: String,
}

/// One bounded lexical reference to a project-defined symbol.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SymbolReference {
    pub path: String,
    pub line: usize,
}

/// Deterministic repository-wide symbol and reference index.
///
/// References are deliberately labelled lexical evidence. They are navigation
/// hints derived from identifier occurrences, not claims about runtime call
/// flow or type resolution.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RepositoryGraph {
    pub definitions: BTreeMap<String, Vec<SymbolLocation>>,
    pub references: BTreeMap<String, Vec<SymbolReference>>,
    pub truncated: bool,
}

/// Bounded evidence for one graph-matched project symbol.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphSymbolEvidence {
    pub name: String,
    pub definitions: Vec<SymbolLocation>,
    pub references: Vec<SymbolReference>,
    pub total_references: usize,
    pub references_truncated: bool,
}

/// Stable result of a repository evidence graph query.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GraphQueryResult {
    pub query: String,
    pub symbols: Vec<GraphSymbolEvidence>,
    pub total_matching_symbols: usize,
    pub result_truncated: bool,
    pub graph_truncated: bool,
}

/// One bounded path related to task-matched seed files by lexical graph evidence.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ImpactPathEvidence {
    pub path: String,
    pub score: usize,
    pub reasons: Vec<String>,
}

/// Deterministic one-hop change-impact query result.
///
/// This is navigation evidence. It does not claim that changing a seed will
/// alter runtime behavior in every related file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ImpactQueryResult {
    pub query: String,
    pub seeds: Vec<String>,
    pub related: Vec<ImpactPathEvidence>,
    pub total_related_files: usize,
    pub result_truncated: bool,
    pub graph_truncated: bool,
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

/// Measured work performed while constructing a repository index.
///
/// Cache hits never bypass content hashing: `bytes_hashed` always describes
/// current workspace bytes read during this build. The cache only reuses
/// bounded, derived structure after its content digest has been established.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct IndexBuildTelemetry {
    pub files_hashed: usize,
    pub bytes_hashed: u64,
    pub cache_eligible_files: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub cache_writes: usize,
    pub rejected_cache_entries: usize,
}

/// A repository index together with deterministic cache telemetry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryIndexBuild {
    pub index: RepositoryIndex,
    pub telemetry: IndexBuildTelemetry,
}

/// Deterministic retrieval measurements derived by Pactrail, not by a model.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RetrievalTelemetry {
    pub query_terms: usize,
    pub retrieved_files: usize,
    pub cited_files: usize,
    pub graph_symbols: usize,
    pub graph_locations: usize,
    pub impact_files: usize,
    /// Share of retrieved files that fit in the rendered pack, from 0 to 10,000.
    pub citation_coverage_basis_points: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CachedFileAnalysis {
    schema_version: u32,
    content_digest: String,
    payload_digest: String,
    language: Language,
    lines: usize,
    symbols: Vec<Symbol>,
    imports: Vec<String>,
    identifier_lines: BTreeMap<String, Vec<usize>>,
    identifiers_truncated: bool,
}

/// Stable repository topology and provenance index.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RepositoryIndex {
    pub files: BTreeMap<String, IndexedFile>,
    pub instructions: Vec<InstructionFile>,
    pub languages: BTreeMap<Language, usize>,
    #[serde(default)]
    pub graph: RepositoryGraph,
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
        Self::build_internal(root, None).map(|build| build.index)
    }

    /// Builds an index while reusing content-addressed derived structure.
    ///
    /// Every current file is still read and hashed before a cache entry can be
    /// used. Cache entries never supply instruction contents or source
    /// previews. Cache I/O is best effort: an unavailable, malformed, or stale
    /// entry is measured and recomputed instead of failing repository context.
    ///
    /// # Errors
    ///
    /// Returns an error when traversal or required workspace file access fails.
    pub fn build_with_cache(
        root: &Path,
        cache_root: &Path,
    ) -> Result<RepositoryIndexBuild, ContextError> {
        Self::build_internal(root, Some(cache_root))
    }

    fn build_internal(
        root: &Path,
        cache_root: Option<&Path>,
    ) -> Result<RepositoryIndexBuild, ContextError> {
        let mut indexed_by_path = BTreeMap::new();
        let mut directives = Vec::new();
        let mut language_counts = BTreeMap::new();
        let mut derived_by_path = BTreeMap::new();
        let mut telemetry = IndexBuildTelemetry::default();
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
            let built = build_indexed_file(root, entry.path(), cache_root, &mut telemetry)?;
            *language_counts.entry(built.indexed.language).or_insert(0) += 1;
            if let Some(directive) = built.instruction {
                directives.push(directive);
            }
            derived_by_path.insert(built.relative.clone(), built.derived);
            indexed_by_path.insert(built.relative, built.indexed);
        }
        directives.sort_by(|left, right| left.path.cmp(&right.path));
        let graph = build_repository_graph(&indexed_by_path, &derived_by_path);
        let digest = index_digest(&indexed_by_path, &directives);
        Ok(RepositoryIndexBuild {
            index: Self {
                files: indexed_by_path,
                instructions: directives,
                languages: language_counts,
                graph,
                digest,
            },
            telemetry,
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
                let direct_score = direct_file_score(file, &tokens);
                (direct_score, file)
            })
            .collect::<Vec<_>>();
        let graph_evidence = self.graph.query(query, 16, 64);
        for (score, file) in &mut scored {
            for symbol in &graph_evidence.symbols {
                *score = score.saturating_add(
                    symbol
                        .definitions
                        .iter()
                        .filter(|location| location.path == file.path)
                        .count()
                        .saturating_mul(6),
                );
                *score = score.saturating_add(
                    symbol
                        .references
                        .iter()
                        .filter(|reference| reference.path == file.path)
                        .count()
                        .saturating_mul(2),
                );
            }
        }
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

    /// Finds files plausibly affected by task-matched seed files.
    ///
    /// Relationships come from bounded project-definition and lexical-reference
    /// evidence. Scores and reasons are deterministic navigation hints, not
    /// type-resolved dependency or runtime-impact claims.
    #[must_use]
    pub fn query_change_impact(
        &self,
        query: &str,
        max_seeds: usize,
        max_related: usize,
    ) -> ImpactQueryResult {
        let tokens = query_tokens(query);
        let mut seed_candidates = self
            .files
            .values()
            .filter_map(|file| {
                let score = direct_file_score(file, &tokens);
                (score > 0).then_some((score, file))
            })
            .collect::<Vec<_>>();
        seed_candidates.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.path.cmp(&right.path))
        });
        let seeds = seed_candidates
            .into_iter()
            .take(max_seeds)
            .map(|(_, file)| file.path.clone())
            .collect::<BTreeSet<_>>();
        let mut related = BTreeMap::<String, (usize, BTreeSet<String>)>::new();
        for (symbol, definitions) in &self.graph.definitions {
            let references = self
                .graph
                .references
                .get(symbol)
                .map_or(&[][..], Vec::as_slice);
            let definition_is_seed = definitions
                .iter()
                .any(|location| seeds.contains(&location.path));
            let reference_is_seed = references
                .iter()
                .any(|location| seeds.contains(&location.path));
            if definition_is_seed {
                for reference in references
                    .iter()
                    .filter(|location| !seeds.contains(&location.path))
                {
                    add_impact_reason(
                        &mut related,
                        &reference.path,
                        3,
                        format!("references {symbol} defined by a seed"),
                    );
                }
            }
            if reference_is_seed {
                for definition in definitions
                    .iter()
                    .filter(|location| !seeds.contains(&location.path))
                {
                    add_impact_reason(
                        &mut related,
                        &definition.path,
                        4,
                        format!("defines {symbol} referenced by a seed"),
                    );
                }
            }
        }
        let total_related_files = related.len();
        let mut related = related
            .into_iter()
            .map(|(path, (score, reasons))| ImpactPathEvidence {
                path,
                score,
                reasons: reasons.into_iter().take(8).collect(),
            })
            .collect::<Vec<_>>();
        related.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.path.cmp(&right.path))
        });
        related.truncate(max_related);
        ImpactQueryResult {
            query: query.to_owned(),
            seeds: seeds.into_iter().collect(),
            result_truncated: related.len() < total_related_files,
            related,
            total_related_files,
            graph_truncated: self.graph.truncated,
        }
    }
}

fn direct_file_score(file: &IndexedFile, tokens: &BTreeSet<String>) -> usize {
    let path = file.path.to_lowercase();
    let symbol_text = file
        .symbols
        .iter()
        .map(|symbol| symbol.name.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    tokens
        .iter()
        .map(|token| {
            usize::from(path.contains(token)) * 4 + usize::from(symbol_text.contains(token)) * 3
        })
        .sum()
}

fn add_impact_reason(
    impact: &mut BTreeMap<String, (usize, BTreeSet<String>)>,
    path: &str,
    weight: usize,
    reason: String,
) {
    let entry = impact.entry(path.to_owned()).or_default();
    entry.0 = entry.0.saturating_add(weight);
    if entry.1.len() < 8 {
        entry.1.insert(reason);
    }
}

impl RepositoryGraph {
    /// Finds project-defined symbols and their bounded lexical references.
    ///
    /// Results are ranked by exact or partial query-token overlap, then by a
    /// stable case-insensitive symbol name. Zero limits return an empty result.
    #[must_use]
    pub fn query(
        &self,
        query: &str,
        max_symbols: usize,
        max_references_per_symbol: usize,
    ) -> GraphQueryResult {
        let normalized_query = query.trim().to_ascii_lowercase();
        let tokens = query_tokens(query);
        let mut matches = self
            .definitions
            .keys()
            .filter_map(|name| {
                let normalized_name = name.to_ascii_lowercase();
                graph_match_score(&normalized_query, &tokens, &normalized_name)
                    .map(|score| (score, normalized_name, name))
            })
            .collect::<Vec<_>>();
        matches.sort_by(
            |(left_score, left_normalized, left), (right_score, right_normalized, right)| {
                right_score
                    .cmp(left_score)
                    .then_with(|| left_normalized.cmp(right_normalized))
                    .then_with(|| left.cmp(right))
            },
        );
        let total_matching_symbols = matches.len();
        let symbols = matches
            .into_iter()
            .take(max_symbols)
            .filter_map(|(_, _, name)| {
                let definitions = self.definitions.get(name)?.clone();
                let all_references = self.references.get(name).cloned().unwrap_or_default();
                let total_references = all_references.len();
                let references = all_references
                    .into_iter()
                    .take(max_references_per_symbol)
                    .collect::<Vec<_>>();
                Some(GraphSymbolEvidence {
                    name: name.clone(),
                    definitions,
                    references_truncated: references.len() < total_references,
                    references,
                    total_references,
                })
            })
            .collect::<Vec<_>>();
        GraphQueryResult {
            query: query.to_owned(),
            result_truncated: symbols.len() < total_matching_symbols,
            symbols,
            total_matching_symbols,
            graph_truncated: self.truncated,
        }
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

struct IndexedFileBuild {
    relative: String,
    indexed: IndexedFile,
    instruction: Option<InstructionFile>,
    derived: CachedFileAnalysis,
}

fn build_indexed_file(
    root: &Path,
    path: &Path,
    cache_root: Option<&Path>,
    telemetry: &mut IndexBuildTelemetry,
) -> Result<IndexedFileBuild, ContextError> {
    let relative = portable_relative(root, path)?;
    let (digest, size, retained) = fingerprint_and_retain(path)?;
    telemetry.files_hashed = telemetry.files_hashed.saturating_add(1);
    telemetry.bytes_hashed = telemetry.bytes_hashed.saturating_add(size);
    let language = detect_language(path);
    let text = retained
        .as_deref()
        .and_then(|bytes| std::str::from_utf8(bytes).ok());
    let derived = text.map_or_else(
        || empty_file_analysis(&digest, language),
        |text| resolve_file_analysis(text, &digest, size, language, cache_root, telemetry),
    );
    let anchor_preview = text
        .filter(|_| repository_anchor_rank(&relative).is_some())
        .map(|text| utf8_prefix(text, MAX_ANCHOR_PREVIEW_BYTES).to_owned());
    let instruction = if path.file_name().is_some_and(|name| name == "AGENTS.md")
        && size <= MAX_INSTRUCTION_BYTES
    {
        text.map(|content| InstructionFile {
            path: relative.clone(),
            scope: instruction_scope(&relative),
            digest: digest.clone(),
            content: content.to_owned(),
        })
    } else {
        None
    };
    let indexed = IndexedFile {
        path: relative.clone(),
        digest,
        bytes: size,
        lines: derived.lines,
        language,
        symbols: derived.symbols.clone(),
        imports: derived.imports.clone(),
        anchor_preview,
    };
    Ok(IndexedFileBuild {
        relative,
        indexed,
        instruction,
        derived,
    })
}

fn empty_file_analysis(content_digest: &str, language: Language) -> CachedFileAnalysis {
    CachedFileAnalysis {
        schema_version: INDEX_CACHE_SCHEMA_VERSION,
        content_digest: content_digest.to_owned(),
        payload_digest: String::new(),
        language,
        lines: 0,
        symbols: Vec::new(),
        imports: Vec::new(),
        identifier_lines: BTreeMap::new(),
        identifiers_truncated: false,
    }
}

fn resolve_file_analysis(
    text: &str,
    content_digest: &str,
    file_bytes: u64,
    language: Language,
    cache_root: Option<&Path>,
    telemetry: &mut IndexBuildTelemetry,
) -> CachedFileAnalysis {
    telemetry.cache_eligible_files = telemetry.cache_eligible_files.saturating_add(1);
    let cached = cache_root.and_then(|root| {
        match load_cached_analysis(root, content_digest, language, file_bytes) {
            CacheLookup::Hit(entry) => {
                telemetry.cache_hits = telemetry.cache_hits.saturating_add(1);
                Some(entry)
            }
            CacheLookup::Miss => {
                telemetry.cache_misses = telemetry.cache_misses.saturating_add(1);
                None
            }
            CacheLookup::Rejected => {
                telemetry.cache_misses = telemetry.cache_misses.saturating_add(1);
                telemetry.rejected_cache_entries =
                    telemetry.rejected_cache_entries.saturating_add(1);
                None
            }
        }
    });
    if cache_root.is_none() {
        telemetry.cache_misses = telemetry.cache_misses.saturating_add(1);
    }
    cached.unwrap_or_else(|| {
        let entry = analyze_file(text, language, content_digest);
        if let Some(root) = cache_root
            && store_cached_analysis(root, &entry)
        {
            telemetry.cache_writes = telemetry.cache_writes.saturating_add(1);
        }
        entry
    })
}

enum CacheLookup {
    Hit(CachedFileAnalysis),
    Miss,
    Rejected,
}

fn analyze_file(text: &str, language: Language, content_digest: &str) -> CachedFileAnalysis {
    let lines = text.lines().count();
    let (symbols, imports) = extract_structure(text, language);
    let mut identifier_lines = BTreeMap::<String, Vec<usize>>::new();
    let mut occurrence_count = 0_usize;
    let mut identifiers_truncated = false;
    if is_code_language(language) {
        'source: for (line_index, raw_line) in text.lines().enumerate() {
            if is_comment_only(raw_line, language) {
                continue;
            }
            let line = line_index.saturating_add(1);
            for identifier in identifiers(raw_line) {
                if occurrence_count == MAX_IDENTIFIER_OCCURRENCES_PER_FILE {
                    identifiers_truncated = true;
                    break 'source;
                }
                identifier_lines
                    .entry(identifier.to_owned())
                    .or_default()
                    .push(line);
                occurrence_count = occurrence_count.saturating_add(1);
            }
        }
    }
    let mut analysis = CachedFileAnalysis {
        schema_version: INDEX_CACHE_SCHEMA_VERSION,
        content_digest: content_digest.to_owned(),
        payload_digest: String::new(),
        language,
        lines,
        symbols,
        imports,
        identifier_lines,
        identifiers_truncated,
    };
    analysis.payload_digest = analysis_payload_digest(&analysis);
    analysis
}

fn analysis_payload_digest(analysis: &CachedFileAnalysis) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&analysis.schema_version.to_le_bytes());
    hasher.update(analysis.content_digest.as_bytes());
    hasher.update(format!("{:?}", analysis.language).as_bytes());
    hasher.update(&analysis.lines.to_le_bytes());
    for symbol in &analysis.symbols {
        hasher.update(symbol.name.as_bytes());
        hasher.update(symbol.kind.as_bytes());
        hasher.update(&symbol.line.to_le_bytes());
    }
    for import in &analysis.imports {
        hasher.update(import.as_bytes());
    }
    for (identifier, lines) in &analysis.identifier_lines {
        hasher.update(identifier.as_bytes());
        for line in lines {
            hasher.update(&line.to_le_bytes());
        }
    }
    hasher.update(&[u8::from(analysis.identifiers_truncated)]);
    hasher.finalize().to_hex().to_string()
}

fn analysis_cache_path(cache_root: &Path, content_digest: &str, language: Language) -> PathBuf {
    let key = blake3::hash(
        format!("{INDEX_CACHE_SCHEMA_VERSION}\0{content_digest}\0{language:?}").as_bytes(),
    )
    .to_hex()
    .to_string();
    cache_root
        .join(format!("v{INDEX_CACHE_SCHEMA_VERSION}"))
        .join(&key[..2])
        .join(format!("{key}.json"))
}

fn load_cached_analysis(
    cache_root: &Path,
    content_digest: &str,
    language: Language,
    file_bytes: u64,
) -> CacheLookup {
    let path = analysis_cache_path(cache_root, content_digest, language);
    let file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return CacheLookup::Miss,
        Err(_) => return CacheLookup::Rejected,
    };
    let size = match file.metadata() {
        Ok(metadata) => metadata.len(),
        Err(_) => return CacheLookup::Rejected,
    };
    if size > MAX_INDEX_CACHE_ENTRY_BYTES {
        return CacheLookup::Rejected;
    }
    let capacity = usize::try_from(size).unwrap_or_default();
    let mut encoded = Vec::with_capacity(capacity);
    if file
        .take(MAX_INDEX_CACHE_ENTRY_BYTES.saturating_add(1))
        .read_to_end(&mut encoded)
        .is_err()
        || u64::try_from(encoded.len()).unwrap_or(u64::MAX) > MAX_INDEX_CACHE_ENTRY_BYTES
    {
        return CacheLookup::Rejected;
    }
    let Ok(analysis) = serde_json::from_slice::<CachedFileAnalysis>(&encoded) else {
        return CacheLookup::Rejected;
    };
    if validate_cached_analysis(&analysis, content_digest, language, file_bytes) {
        CacheLookup::Hit(analysis)
    } else {
        CacheLookup::Rejected
    }
}

fn validate_cached_analysis(
    analysis: &CachedFileAnalysis,
    content_digest: &str,
    language: Language,
    file_bytes: u64,
) -> bool {
    let max_lines = usize::try_from(file_bytes)
        .unwrap_or(usize::MAX)
        .saturating_add(1);
    if analysis.schema_version != INDEX_CACHE_SCHEMA_VERSION
        || analysis.content_digest != content_digest
        || analysis.payload_digest != analysis_payload_digest(analysis)
        || analysis.language != language
        || analysis.lines > max_lines
        || analysis.symbols.len() > MAX_SYMBOLS_PER_FILE
        || analysis.imports.len() > MAX_IMPORTS_PER_FILE
    {
        return false;
    }
    if analysis.symbols.iter().any(|symbol| {
        !valid_identifier(&symbol.name)
            || !matches!(
                symbol.kind.as_str(),
                "function" | "struct" | "enum" | "trait" | "class" | "type" | "interface"
            )
            || symbol.line == 0
            || symbol.line > analysis.lines
    }) || analysis.imports.iter().any(|import| {
        import.len() > 500
            || import
                .chars()
                .any(|value| matches!(value, '\0' | '\r' | '\n'))
    }) {
        return false;
    }
    let mut occurrences = 0_usize;
    for (identifier, lines) in &analysis.identifier_lines {
        if !valid_identifier(identifier)
            || lines
                .windows(2)
                .any(|window| window.first() >= window.get(1))
            || lines
                .iter()
                .any(|line| *line == 0 || *line > analysis.lines)
        {
            return false;
        }
        occurrences = occurrences.saturating_add(lines.len());
        if occurrences > MAX_IDENTIFIER_OCCURRENCES_PER_FILE {
            return false;
        }
    }
    true
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_BYTES
        && value.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
        })
}

fn store_cached_analysis(cache_root: &Path, analysis: &CachedFileAnalysis) -> bool {
    let path = analysis_cache_path(cache_root, &analysis.content_digest, analysis.language);
    let Some(parent) = path.parent() else {
        return false;
    };
    if fs::create_dir_all(parent).is_err() {
        return false;
    }
    let Ok(canonical_root) = fs::canonicalize(cache_root) else {
        return false;
    };
    let Ok(canonical_parent) = fs::canonicalize(parent) else {
        return false;
    };
    if !canonical_parent.starts_with(&canonical_root) {
        return false;
    }
    let Ok(mut temporary) = tempfile::NamedTempFile::new_in(parent) else {
        return false;
    };
    if serde_json::to_writer(temporary.as_file_mut(), analysis).is_err()
        || temporary.as_file_mut().flush().is_err()
        || temporary.as_file().sync_all().is_err()
    {
        return false;
    }
    temporary.persist(&path).is_ok()
}

fn build_repository_graph(
    files: &BTreeMap<String, IndexedFile>,
    derived_by_path: &BTreeMap<String, CachedFileAnalysis>,
) -> RepositoryGraph {
    let mut graph = RepositoryGraph::default();
    let mut definition_count = 0_usize;
    for file in files.values() {
        for symbol in &file.symbols {
            if definition_count == MAX_GRAPH_DEFINITIONS {
                graph.truncated = true;
                break;
            }
            graph
                .definitions
                .entry(symbol.name.clone())
                .or_default()
                .push(SymbolLocation {
                    path: file.path.clone(),
                    line: symbol.line,
                    kind: symbol.kind.clone(),
                });
            definition_count = definition_count.saturating_add(1);
        }
        if definition_count == MAX_GRAPH_DEFINITIONS {
            break;
        }
    }

    let definition_names = graph.definitions.keys().cloned().collect::<BTreeSet<_>>();
    let mut reference_count = 0_usize;
    'files: for file in files.values() {
        if !is_code_language(file.language) || file.bytes > MAX_INDEX_FILE_BYTES {
            continue;
        }
        let Some(derived) = derived_by_path.get(&file.path) else {
            continue;
        };
        graph.truncated |= derived.identifiers_truncated;
        for (identifier, lines) in &derived.identifier_lines {
            if !definition_names.contains(identifier) {
                continue;
            }
            for line in lines {
                let is_definition = graph.definitions.get(identifier).is_some_and(|locations| {
                    locations
                        .iter()
                        .any(|location| location.path == file.path && location.line == *line)
                });
                if is_definition {
                    continue;
                }
                let references = graph.references.entry(identifier.to_owned()).or_default();
                if references.len() == MAX_REFERENCES_PER_SYMBOL {
                    graph.truncated = true;
                    continue;
                }
                references.push(SymbolReference {
                    path: file.path.clone(),
                    line: *line,
                });
                reference_count = reference_count.saturating_add(1);
                if reference_count == MAX_GRAPH_REFERENCES {
                    graph.truncated = true;
                    break 'files;
                }
            }
        }
    }
    graph
}

fn is_code_language(language: Language) -> bool {
    matches!(
        language,
        Language::Rust
            | Language::TypeScript
            | Language::JavaScript
            | Language::Python
            | Language::Go
            | Language::Java
            | Language::Kotlin
            | Language::C
            | Language::Cpp
            | Language::CSharp
            | Language::Ruby
            | Language::Php
    )
}

fn is_comment_only(line: &str, language: Language) -> bool {
    let trimmed = line.trim_start();
    match language {
        Language::Python | Language::Ruby => trimmed.starts_with('#'),
        Language::Php => trimmed.starts_with("//") || trimmed.starts_with('#'),
        _ => trimmed.starts_with("//") || trimmed.starts_with('*'),
    }
}

fn identifiers(line: &str) -> BTreeSet<&str> {
    let mut identifiers = BTreeSet::new();
    let bytes = line.as_bytes();
    let mut start = None;
    for (index, byte) in bytes.iter().copied().enumerate() {
        let valid = byte.is_ascii_alphanumeric() || byte == b'_';
        match (start, valid) {
            (None, true) if byte.is_ascii_alphabetic() || byte == b'_' => start = Some(index),
            (Some(begin), false) => {
                if index.saturating_sub(begin) <= MAX_IDENTIFIER_BYTES {
                    identifiers.insert(&line[begin..index]);
                }
                start = None;
            }
            _ => {}
        }
    }
    if let Some(begin) = start
        && bytes.len().saturating_sub(begin) <= MAX_IDENTIFIER_BYTES
    {
        identifiers.insert(&line[begin..]);
    }
    identifiers
}

fn graph_match_score(
    normalized_query: &str,
    tokens: &BTreeSet<String>,
    normalized_name: &str,
) -> Option<usize> {
    if normalized_query.is_empty() || normalized_name.is_empty() {
        return None;
    }
    if normalized_query == normalized_name {
        return Some(1_000);
    }
    if normalized_name.len() < 3 {
        return None;
    }
    tokens
        .iter()
        .filter_map(|token| {
            if token == normalized_name {
                Some(600)
            } else if normalized_name.contains(token) {
                Some(300_usize.saturating_add(token.len()))
            } else if normalized_name.len() >= 4 && token.contains(normalized_name) {
                Some(200_usize.saturating_add(normalized_name.len()))
            } else {
                None
            }
        })
        .max()
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
    pub project_profile: String,
    pub rendered: String,
    pub cited_files: Vec<String>,
    pub included_instructions: Vec<String>,
    pub included_fragments: Vec<String>,
    pub rendered_bytes: usize,
    pub budget_bytes: usize,
    pub truncated: bool,
    pub retrieval: RetrievalTelemetry,
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
        let graph_evidence = index.graph.query(&contract.goal, 8, 16);
        let impact_evidence = index.query_change_impact(&contract.goal, 8, 16);
        let language_summary = index
            .languages
            .iter()
            .map(|(language, count)| format!("{language:?}: {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        let project_profile = deterministic_project_profile(index);
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
            "# Task contract\n{contract_json}\n\nThe model-visible workspace root is `.`. Every tool path must be relative to it; host paths are unavailable.\n\n# Repository\nDigest: {}\nFiles: {}\nLanguages: {language_summary}\nDeterministic project profile: {project_profile}\n\n# Repository-wide instructions\n{global_instructions}\n\nNested AGENTS.md files apply only to files beneath their declared virtual directory scope.\n",
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
        append_graph_evidence(&mut writer, &graph_evidence, &mut cited_files);
        append_impact_evidence(&mut writer, &impact_evidence, &mut cited_files);
        append_topology(&mut writer, &retrieved, &mut cited_files);
        cited_files.sort();
        cited_files.dedup();
        let (rendered, truncated) = writer.finish();
        let rendered_bytes = rendered.len();
        let retrieval = retrieval_telemetry(
            &contract.goal,
            &retrieved,
            &cited_files,
            &graph_evidence,
            &impact_evidence,
        );
        Ok(Self {
            repository_digest: index.digest.clone(),
            project_profile,
            rendered,
            cited_files,
            included_instructions,
            included_fragments,
            rendered_bytes,
            budget_bytes: budget.max_bytes,
            truncated,
            retrieval,
        })
    }
}

fn retrieval_telemetry(
    query: &str,
    retrieved: &[&IndexedFile],
    cited_files: &[String],
    graph_evidence: &GraphQueryResult,
    impact_evidence: &ImpactQueryResult,
) -> RetrievalTelemetry {
    let retrieved_paths = retrieved
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    let cited_retrieved_files = cited_files
        .iter()
        .filter(|path| retrieved_paths.contains(path.as_str()))
        .count();
    let citation_coverage_basis_points = if retrieved.is_empty() {
        10_000
    } else {
        u16::try_from(
            cited_retrieved_files
                .saturating_mul(10_000)
                .checked_div(retrieved.len())
                .unwrap_or_default()
                .min(10_000),
        )
        .unwrap_or(10_000)
    };
    let graph_locations = graph_evidence
        .symbols
        .iter()
        .map(|symbol| {
            symbol
                .definitions
                .len()
                .saturating_add(symbol.references.len())
        })
        .sum();
    RetrievalTelemetry {
        query_terms: query_tokens(query).len(),
        retrieved_files: retrieved.len(),
        cited_files: cited_files.len(),
        graph_symbols: graph_evidence.symbols.len(),
        graph_locations,
        impact_files: impact_evidence.related.len(),
        citation_coverage_basis_points,
    }
}

fn deterministic_project_profile(index: &RepositoryIndex) -> String {
    let ecosystems = [
        ("Cargo.toml", "Rust/Cargo"),
        ("package.json", "JavaScript or TypeScript/npm"),
        ("pyproject.toml", "Python"),
        ("go.mod", "Go"),
        ("pom.xml", "Java/Maven"),
        ("build.gradle", "JVM/Gradle"),
        ("build.gradle.kts", "JVM/Gradle"),
        ("mix.exs", "Elixir/Mix"),
        ("composer.json", "PHP/Composer"),
    ]
    .into_iter()
    .filter(|(path, _)| index.files.contains_key(*path))
    .map(|(path, ecosystem)| format!("{ecosystem} ({path})"))
    .collect::<Vec<_>>();
    let entrypoints = [
        ("src/lib.rs", "Rust library"),
        ("src/main.rs", "Rust binary"),
        ("src/index.ts", "TypeScript entry point"),
        ("src/index.js", "JavaScript entry point"),
        ("main.py", "Python entry point"),
        ("app.py", "Python application"),
        ("cmd/main.go", "Go command"),
    ]
    .into_iter()
    .filter(|(path, _)| index.files.contains_key(*path))
    .map(|(path, role)| format!("{role} ({path})"))
    .collect::<Vec<_>>();
    let ecosystem_text = if ecosystems.is_empty() {
        "no conventional root manifest detected".to_owned()
    } else {
        format!("ecosystem evidence: {}", ecosystems.join(", "))
    };
    let entrypoint_text = if entrypoints.is_empty() {
        "no conventional entry point detected".to_owned()
    } else {
        format!("entry points: {}", entrypoints.join(", "))
    };
    let readme = if index.files.contains_key("README.md") {
        "README.md is present"
    } else {
        "no root README.md is present"
    };
    format!("{ecosystem_text}; {entrypoint_text}; {readme}")
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

fn append_graph_evidence(
    writer: &mut PackWriter,
    result: &GraphQueryResult,
    cited: &mut Vec<String>,
) {
    let mut heading_pending = true;
    for symbol in &result.symbols {
        let heading = if heading_pending {
            "\n# Repository evidence graph\nProject-defined symbols are linked to bounded lexical identifier references. Use these as navigation hints, not proof of runtime call flow; read current source before editing.\n\n"
        } else {
            "\n"
        };
        let definitions = symbol
            .definitions
            .iter()
            .map(|location| format!("{}:{} ({})", location.path, location.line, location.kind))
            .collect::<Vec<_>>()
            .join(", ");
        let references = symbol
            .references
            .iter()
            .map(|reference| format!("{}:{}", reference.path, reference.line))
            .collect::<Vec<_>>()
            .join(", ");
        let entry = format!(
            "{heading}- {}\n  definitions: {definitions}\n  lexical references: {references}",
            symbol.name
        );
        if writer.push_optional(&entry) {
            heading_pending = false;
            cited.extend(
                symbol
                    .definitions
                    .iter()
                    .map(|location| location.path.clone()),
            );
            cited.extend(
                symbol
                    .references
                    .iter()
                    .map(|reference| reference.path.clone()),
            );
        }
    }
}

fn append_impact_evidence(
    writer: &mut PackWriter,
    result: &ImpactQueryResult,
    cited: &mut Vec<String>,
) {
    let mut heading_pending = true;
    for related in &result.related {
        let heading = if heading_pending {
            "\n# Change-impact evidence\nThese are one-hop lexical relationships from task-matched seed files. Use them to choose what to inspect; they are not proof of runtime impact.\n\n"
        } else {
            "\n"
        };
        let entry = format!(
            "{heading}- {} [score {}]: {}",
            related.path,
            related.score,
            related.reasons.join("; ")
        );
        if writer.push_optional(&entry) {
            heading_pending = false;
            cited.push(related.path.clone());
        }
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
        if imports.len() < MAX_IMPORTS_PER_FILE
            && let Some(import) = extract_import(line, language)
        {
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
    #[error("repository file changed while its evidence graph was being built: {0}")]
    ChangedDuringIndex(PathBuf),
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
    fn content_addressed_cache_reuses_only_unchanged_file_analysis() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        let cache = tempfile::tempdir().unwrap_or_else(|error| unreachable!("cache: {error}"));
        fs::write(root.path().join("one.rs"), "pub fn one() {}\n")
            .unwrap_or_else(|error| unreachable!("one: {error}"));
        fs::write(root.path().join("two.rs"), "pub fn two() {}\n")
            .unwrap_or_else(|error| unreachable!("two: {error}"));

        let cold = RepositoryIndex::build_with_cache(root.path(), cache.path())
            .unwrap_or_else(|error| unreachable!("cold index: {error}"));
        assert_eq!(cold.telemetry.files_hashed, 2);
        assert_eq!(cold.telemetry.cache_hits, 0);
        assert_eq!(cold.telemetry.cache_misses, 2);
        assert_eq!(cold.telemetry.cache_writes, 2);

        let warm = RepositoryIndex::build_with_cache(root.path(), cache.path())
            .unwrap_or_else(|error| unreachable!("warm index: {error}"));
        assert_eq!(warm.index, cold.index);
        assert_eq!(warm.telemetry.cache_hits, 2);
        assert_eq!(warm.telemetry.cache_misses, 0);

        fs::write(root.path().join("two.rs"), "pub fn changed() {}\n")
            .unwrap_or_else(|error| unreachable!("changed: {error}"));
        let incremental = RepositoryIndex::build_with_cache(root.path(), cache.path())
            .unwrap_or_else(|error| unreachable!("incremental index: {error}"));
        assert_eq!(incremental.telemetry.cache_hits, 1);
        assert_eq!(incremental.telemetry.cache_misses, 1);
        assert_eq!(incremental.index.files["two.rs"].symbols[0].name, "changed");
    }

    #[test]
    fn malformed_cache_entry_is_rejected_and_repaired() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        let cache = tempfile::tempdir().unwrap_or_else(|error| unreachable!("cache: {error}"));
        let source = b"pub fn verified() {}\n";
        fs::write(root.path().join("lib.rs"), source)
            .unwrap_or_else(|error| unreachable!("source: {error}"));
        let digest = blake3::hash(source).to_hex().to_string();
        let cache_path = analysis_cache_path(cache.path(), &digest, Language::Rust);

        let _cold = RepositoryIndex::build_with_cache(root.path(), cache.path())
            .unwrap_or_else(|error| unreachable!("cold index: {error}"));
        fs::write(&cache_path, b"{not-json")
            .unwrap_or_else(|error| unreachable!("corrupt cache: {error}"));

        let repaired = RepositoryIndex::build_with_cache(root.path(), cache.path())
            .unwrap_or_else(|error| unreachable!("repair index: {error}"));
        assert_eq!(repaired.telemetry.rejected_cache_entries, 1);
        assert_eq!(repaired.telemetry.cache_writes, 1);
        assert_eq!(repaired.index.files["lib.rs"].symbols[0].name, "verified");

        let warm = RepositoryIndex::build_with_cache(root.path(), cache.path())
            .unwrap_or_else(|error| unreachable!("warm index: {error}"));
        assert_eq!(warm.telemetry.cache_hits, 1);
        assert_eq!(warm.telemetry.rejected_cache_entries, 0);
    }

    #[test]
    fn repository_graph_links_definitions_to_bounded_lexical_references() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        fs::create_dir(root.path().join("src"))
            .unwrap_or_else(|error| unreachable!("src: {error}"));
        fs::write(
            root.path().join("src/receipt.rs"),
            "pub struct Receipt;\nimpl Receipt {\n    pub fn verify(&self) {}\n}\n",
        )
        .unwrap_or_else(|error| unreachable!("definition: {error}"));
        fs::write(
            root.path().join("src/consumer.rs"),
            "use crate::receipt::Receipt;\npub fn consume(receipt: Receipt) { receipt.verify(); }\n",
        )
        .unwrap_or_else(|error| unreachable!("reference: {error}"));

        let index = RepositoryIndex::build(root.path())
            .unwrap_or_else(|error| unreachable!("index: {error}"));
        let result = index.graph.query("Receipt", 8, 8);

        assert_eq!(result.total_matching_symbols, 1);
        assert_eq!(result.symbols[0].name, "Receipt");
        assert_eq!(result.symbols[0].definitions[0].path, "src/receipt.rs");
        assert_eq!(result.symbols[0].definitions[0].line, 1);
        assert!(
            result.symbols[0]
                .references
                .iter()
                .any(|reference| reference.path == "src/consumer.rs" && reference.line == 1)
        );
        assert!(!result.graph_truncated);
    }

    #[test]
    fn change_impact_links_seed_definitions_and_seed_references() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        fs::create_dir(root.path().join("src"))
            .unwrap_or_else(|error| unreachable!("src: {error}"));
        fs::write(
            root.path().join("src/definition.rs"),
            "pub struct Receipt;\n",
        )
        .unwrap_or_else(|error| unreachable!("definition: {error}"));
        fs::write(
            root.path().join("src/consumer.rs"),
            "pub fn consume(value: Receipt) {}\n",
        )
        .unwrap_or_else(|error| unreachable!("consumer: {error}"));
        let index = RepositoryIndex::build(root.path())
            .unwrap_or_else(|error| unreachable!("index: {error}"));

        let downstream = index.query_change_impact("Receipt", 1, 8);
        assert_eq!(downstream.seeds, vec!["src/definition.rs"]);
        assert_eq!(downstream.related[0].path, "src/consumer.rs");
        assert!(downstream.related[0].reasons[0].contains("references Receipt"));

        let upstream = index.query_change_impact("consume", 1, 8);
        assert_eq!(upstream.seeds, vec!["src/consumer.rs"]);
        assert_eq!(upstream.related[0].path, "src/definition.rs");
        assert!(upstream.related[0].reasons[0].contains("defines Receipt"));
    }

    #[test]
    fn graph_evidence_expands_initial_retrieval_and_context() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        fs::create_dir(root.path().join("src"))
            .unwrap_or_else(|error| unreachable!("src: {error}"));
        fs::write(
            root.path().join("src/definition.rs"),
            "pub struct Receipt;\n",
        )
        .unwrap_or_else(|error| unreachable!("definition: {error}"));
        fs::write(
            root.path().join("src/consumer.rs"),
            "pub fn consume(value: Receipt) {}\n",
        )
        .unwrap_or_else(|error| unreachable!("reference: {error}"));
        let index = RepositoryIndex::build(root.path())
            .unwrap_or_else(|error| unreachable!("index: {error}"));

        let retrieved = index.retrieve("fix Receipt", 10);
        assert!(retrieved.iter().any(|file| file.path == "src/consumer.rs"));

        let context = ContextPack::compile(&TaskContract::new("fix Receipt", "."), &index)
            .unwrap_or_else(|error| unreachable!("context: {error}"));
        assert!(context.rendered.contains("# Repository evidence graph"));
        assert!(context.rendered.contains("# Change-impact evidence"));
        assert!(context.rendered.contains("src/consumer.rs:1"));
        assert!(context.cited_files.contains(&"src/consumer.rs".to_owned()));
        assert_eq!(context.retrieval.impact_files, 1);
    }

    #[test]
    fn graph_query_does_not_match_short_symbols_inside_unrelated_words() {
        let graph = RepositoryGraph {
            definitions: BTreeMap::from([(
                "get".to_owned(),
                vec![SymbolLocation {
                    path: "src/lib.rs".to_owned(),
                    line: 1,
                    kind: "function".to_owned(),
                }],
            )]),
            references: BTreeMap::new(),
            truncated: false,
        };

        assert!(graph.query("fix target selection", 8, 8).symbols.is_empty());
        assert_eq!(graph.query("fix get", 8, 8).symbols[0].name, "get");
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
        assert!(context.rendered.contains(
            "Deterministic project profile: ecosystem evidence: Rust/Cargo (Cargo.toml); entry points: Rust library (src/lib.rs)"
        ));
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
