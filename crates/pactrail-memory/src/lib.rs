//! Provenance-aware, workspace-local memory for Pactrail.
//!
//! Memory is deliberately separate from model conversation history. User
//! memories are explicit, and run memories are derived only from applied,
//! integrity-checked receipts. Models can recall memory but cannot silently
//! persist arbitrary claims.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Mutex, MutexGuard};

use pactrail_core::{ChangeReceipt, ReceiptOutcome, RunId};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;

/// Current on-disk schema for the provenance memory database.
pub const MEMORY_DATABASE_SCHEMA_VERSION: i64 = 1;

/// Oldest initialized memory database schema this binary can open.
pub const MIN_MEMORY_DATABASE_SCHEMA_VERSION: i64 = 1;
const MAX_TITLE_BYTES: usize = 512;
const MAX_CONTENT_BYTES: usize = 64 * 1024;
const MAX_TAGS: usize = 32;
const MAX_TAG_BYTES: usize = 64;
const MAX_QUERY_BYTES: usize = 4 * 1024;
const MAX_RESULTS: usize = 100;
const SEARCH_CANDIDATES: usize = 2_000;

/// Stable identity of one memory entry.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Hash, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct MemoryId(Uuid);

impl MemoryId {
    /// Creates a time-ordered memory identifier.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for MemoryId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MemoryId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for MemoryId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

/// Semantic class used for retrieval and UI grouping.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Convention,
    Decision,
    Warning,
    AppliedRun,
}

impl MemoryKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Convention => "convention",
            Self::Decision => "decision",
            Self::Warning => "warning",
            Self::AppliedRun => "applied_run",
        }
    }
}

impl std::fmt::Display for MemoryKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for MemoryKind {
    type Err = MemoryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "convention" => Ok(Self::Convention),
            "decision" => Ok(Self::Decision),
            "warning" => Ok(Self::Warning),
            "applied_run" => Ok(Self::AppliedRun),
            _ => Err(MemoryError::Corrupt(format!(
                "unknown memory kind {value:?}"
            ))),
        }
    }
}

/// Origin used to distinguish explicit human memory from receipt-derived memory.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    User,
    AppliedReceipt,
}

impl MemorySource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::AppliedReceipt => "applied_receipt",
        }
    }
}

impl std::fmt::Display for MemorySource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for MemorySource {
    type Err = MemoryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "user" => Ok(Self::User),
            "applied_receipt" => Ok(Self::AppliedReceipt),
            _ => Err(MemoryError::Corrupt(format!(
                "unknown memory source {value:?}"
            ))),
        }
    }
}

/// User-supplied memory input.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct MemoryDraft {
    pub kind: MemoryKind,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// One durable memory with provenance and usage metadata.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct MemoryRecord {
    pub id: MemoryId,
    pub kind: MemoryKind,
    pub source: MemorySource,
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub source_run_id: Option<RunId>,
    pub source_integrity_hash: Option<String>,
    #[schemars(with = "String")]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[schemars(with = "String")]
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub access_count: u64,
}

/// A retrieved memory and its deterministic lexical relevance score.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct MemoryMatch {
    pub score: u64,
    pub memory: MemoryRecord,
}

/// Thread-safe SQLite-backed workspace memory.
#[derive(Debug)]
pub struct MemoryStore {
    connection: Mutex<Connection>,
}

impl MemoryStore {
    /// Reads the on-disk database schema without creating or migrating state.
    ///
    /// # Errors
    ///
    /// Returns an error for inaccessible, non-regular, symlinked, or malformed
    /// database paths.
    pub fn database_schema_version(path: impl AsRef<Path>) -> Result<Option<i64>, MemoryError> {
        let path = path.as_ref();
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(MemoryError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(MemoryError::InvalidDatabasePath(path.to_path_buf()));
        }
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(MemoryError::Database)?;
        connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map(Some)
            .map_err(MemoryError::Database)
    }

    /// Opens a current memory database without changing pragmas or state.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing/malformed database or unsupported schema.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self, MemoryError> {
        let path = path.as_ref();
        let version = Self::database_schema_version(path)?
            .ok_or_else(|| MemoryError::InvalidDatabasePath(path.to_path_buf()))?;
        if version != MEMORY_DATABASE_SCHEMA_VERSION {
            return Err(MemoryError::UnsupportedDatabaseSchema(version));
        }
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(MemoryError::Database)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Verifies that a schema-zero `SQLite` file contains no user objects.
    ///
    /// # Errors
    ///
    /// Returns an error when the file is not schema zero or already contains
    /// objects that Pactrail cannot safely classify as a new memory store.
    pub fn validate_uninitialized(path: impl AsRef<Path>) -> Result<(), MemoryError> {
        let path = path.as_ref();
        if Self::database_schema_version(path)? != Some(0) {
            return Err(MemoryError::ExpectedUninitializedDatabase);
        }
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(MemoryError::Database)?;
        let objects = unversioned_object_count(&connection)?;
        if objects == 0 {
            Ok(())
        } else {
            Err(MemoryError::UnexpectedUnversionedObjects(objects))
        }
    }

    /// Decodes every memory row without updating access metadata.
    ///
    /// # Errors
    ///
    /// Returns an error for a malformed table or corrupt record.
    pub fn validate_all(&self) -> Result<usize, MemoryError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id, kind, source, title, content, tags_json, source_run_id,
                        source_integrity_hash, created_at, updated_at, access_count
                 FROM memories ORDER BY id ASC",
            )
            .map_err(MemoryError::Database)?;
        let rows = statement
            .query_map([], decode_row)
            .map_err(MemoryError::Database)?;
        let mut count = 0_usize;
        for row in rows {
            let _memory = row.map_err(MemoryError::Database)??;
            count = count.saturating_add(1);
        }
        Ok(count)
    }

    /// Opens a durable store and initializes the supported schema.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported schemas, database failures, or an
    /// inaccessible parent directory.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MemoryError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| MemoryError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        validate_database_target(path)?;
        let connection = Connection::open(path).map_err(MemoryError::Database)?;
        Self::initialize(connection)
    }

    /// Opens an ephemeral in-memory store for tests and embedding.
    ///
    /// # Errors
    ///
    /// Returns an error if `SQLite` initialization fails.
    pub fn open_in_memory() -> Result<Self, MemoryError> {
        Self::initialize(Connection::open_in_memory().map_err(MemoryError::Database)?)
    }

    fn initialize(mut connection: Connection) -> Result<Self, MemoryError> {
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .and_then(|()| connection.pragma_update(None, "journal_mode", "WAL"))
            .and_then(|()| connection.pragma_update(None, "synchronous", "FULL"))
            .map_err(MemoryError::Database)?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(MemoryError::Database)?;
        if !matches!(version, 0 | MEMORY_DATABASE_SCHEMA_VERSION) {
            return Err(MemoryError::UnsupportedDatabaseSchema(version));
        }
        if version == 0 {
            let objects = unversioned_object_count(&connection)?;
            if objects != 0 {
                return Err(MemoryError::UnexpectedUnversionedObjects(objects));
            }
        }
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MemoryError::Database)?;
        transaction
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS memories (
                    id TEXT PRIMARY KEY NOT NULL,
                    kind TEXT NOT NULL,
                    source TEXT NOT NULL,
                    title TEXT NOT NULL,
                    content TEXT NOT NULL,
                    tags_json TEXT NOT NULL,
                    source_run_id TEXT UNIQUE,
                    source_integrity_hash TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    last_accessed_at TEXT,
                    access_count INTEGER NOT NULL DEFAULT 0,
                    active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1))
                 ) STRICT;
                 CREATE INDEX IF NOT EXISTS memories_active_updated
                    ON memories (active, updated_at DESC);",
            )
            .map_err(MemoryError::Database)?;
        if version == 0 {
            transaction
                .pragma_update(None, "user_version", MEMORY_DATABASE_SCHEMA_VERSION)
                .map_err(MemoryError::Database)?;
        }
        transaction.commit().map_err(MemoryError::Database)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Persists an explicit user memory after strict validation.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid input or an atomic database failure.
    pub fn remember(&self, mut draft: MemoryDraft) -> Result<MemoryRecord, MemoryError> {
        validate_draft(&draft)?;
        draft.tags = normalize_tags(draft.tags)?;
        let id = MemoryId::new();
        let now = OffsetDateTime::now_utc();
        let timestamp = format_time(now)?;
        let tags_json = serde_json::to_string(&draft.tags).map_err(MemoryError::Serialization)?;
        self.connection()?
            .execute(
                "INSERT INTO memories
             (id, kind, source, title, content, tags_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![
                    id.to_string(),
                    draft.kind.as_str(),
                    MemorySource::User.as_str(),
                    draft.title.trim(),
                    draft.content.trim(),
                    tags_json,
                    timestamp,
                ],
            )
            .map_err(MemoryError::Database)?;
        Ok(MemoryRecord {
            id,
            kind: draft.kind,
            source: MemorySource::User,
            title: draft.title.trim().to_owned(),
            content: draft.content.trim().to_owned(),
            tags: draft.tags,
            source_run_id: None,
            source_integrity_hash: None,
            created_at: now,
            updated_at: now,
            access_count: 0,
        })
    }

    /// Records an applied run exactly once using only integrity-checked receipt data.
    ///
    /// # Errors
    ///
    /// Refuses non-applied or invalid receipts and detects a run whose recorded
    /// integrity hash changes across retries.
    pub fn remember_applied_run(
        &self,
        receipt: &ChangeReceipt,
    ) -> Result<MemoryRecord, MemoryError> {
        if receipt.outcome != ReceiptOutcome::Applied {
            return Err(MemoryError::Invalid(
                "only applied receipts can become run memory".to_owned(),
            ));
        }
        if !receipt.verify_integrity().map_err(MemoryError::Receipt)? {
            return Err(MemoryError::Invalid(
                "receipt integrity verification failed".to_owned(),
            ));
        }
        let run_id = receipt.run_id.to_string();
        if let Some(existing) = self.find_by_run_id(receipt.run_id)? {
            if existing.source_integrity_hash.as_deref() == Some(&receipt.integrity_hash) {
                return Ok(existing);
            }
            return Err(MemoryError::ProvenanceConflict(receipt.run_id));
        }

        let id = MemoryId::new();
        let now = OffsetDateTime::now_utc();
        let timestamp = format_time(now)?;
        let title = bounded_title(&format!("Applied: {}", receipt.contract.goal));
        let paths = receipt
            .changes
            .iter()
            .take(200)
            .map(|change| change.path.as_str())
            .collect::<Vec<_>>();
        let omitted_paths = receipt.changes.len().saturating_sub(paths.len());
        let raw_content = format!(
            "Goal: {}\nChanged files ({}): {}{}\nVerification: {} passed, {} failed, {} inconclusive\nOutstanding risks: {}",
            receipt.contract.goal,
            receipt.changes.len(),
            if paths.is_empty() {
                "none".to_owned()
            } else {
                paths.join(", ")
            },
            if omitted_paths == 0 {
                String::new()
            } else {
                format!(" (+{omitted_paths} omitted)")
            },
            receipt.verification.passed,
            receipt.verification.failed,
            receipt.verification.inconclusive,
            if receipt.unresolved_risks.is_empty() {
                "none".to_owned()
            } else {
                receipt.unresolved_risks.join("; ")
            },
        );
        let content = truncate_utf8(&raw_content, MAX_CONTENT_BYTES);
        let tags = applied_run_tags(receipt);
        let tags_json = serde_json::to_string(&tags).map_err(MemoryError::Serialization)?;
        self.connection()?
            .execute(
                "INSERT INTO memories
             (id, kind, source, title, content, tags_json, source_run_id,
              source_integrity_hash, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                params![
                    id.to_string(),
                    MemoryKind::AppliedRun.as_str(),
                    MemorySource::AppliedReceipt.as_str(),
                    title,
                    content,
                    tags_json,
                    run_id,
                    receipt.integrity_hash,
                    timestamp,
                ],
            )
            .map_err(MemoryError::Database)?;
        Ok(MemoryRecord {
            id,
            kind: MemoryKind::AppliedRun,
            source: MemorySource::AppliedReceipt,
            title,
            content,
            tags,
            source_run_id: Some(receipt.run_id),
            source_integrity_hash: Some(receipt.integrity_hash.clone()),
            created_at: now,
            updated_at: now,
            access_count: 0,
        })
    }

    /// Returns active memories ranked by deterministic lexical relevance.
    ///
    /// An empty query returns the most recently updated memories. Selected
    /// entries have their access metadata updated atomically.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid bounds, corrupt rows, or database failures.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryMatch>, MemoryError> {
        if query.len() > MAX_QUERY_BYTES {
            return Err(MemoryError::Invalid(format!(
                "memory query exceeds {MAX_QUERY_BYTES} bytes"
            )));
        }
        if limit == 0 || limit > MAX_RESULTS {
            return Err(MemoryError::Invalid(format!(
                "memory result limit must be between 1 and {MAX_RESULTS}"
            )));
        }
        let tokens = query_tokens(query);
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MemoryError::Database)?;
        let mut candidates = {
            let mut statement = transaction
                .prepare(
                    "SELECT id, kind, source, title, content, tags_json, source_run_id,
                            source_integrity_hash, created_at, updated_at, access_count
                     FROM memories WHERE active = 1
                     ORDER BY updated_at DESC, id ASC LIMIT ?1",
                )
                .map_err(MemoryError::Database)?;
            let rows = statement
                .query_map(
                    [i64::try_from(SEARCH_CANDIDATES).unwrap_or(i64::MAX)],
                    decode_row,
                )
                .map_err(MemoryError::Database)?;
            let mut records = Vec::new();
            for row in rows {
                records.push(row.map_err(MemoryError::Database)??);
            }
            records
        };
        let lowered_query = query.trim().to_lowercase();
        let query_empty = lowered_query.is_empty();
        let mut matches = candidates
            .drain(..)
            .filter_map(|memory| {
                let score = score_memory(&memory, &tokens, &lowered_query);
                (query_empty || score > 0).then_some(MemoryMatch { score, memory })
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| right.memory.updated_at.cmp(&left.memory.updated_at))
                .then_with(|| left.memory.id.cmp(&right.memory.id))
        });
        matches.truncate(limit);
        if !matches.is_empty() {
            let now = format_time(OffsetDateTime::now_utc())?;
            for item in &matches {
                transaction
                    .execute(
                        "UPDATE memories
                         SET access_count = access_count + 1, last_accessed_at = ?1
                         WHERE id = ?2 AND active = 1",
                        params![now, item.memory.id.to_string()],
                    )
                    .map_err(MemoryError::Database)?;
            }
        }
        transaction.commit().map_err(MemoryError::Database)?;
        for item in &mut matches {
            item.memory.access_count = item.memory.access_count.saturating_add(1);
        }
        Ok(matches)
    }

    /// Lists active memories by most recent update without changing usage counters.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid bounds, corrupt rows, or database failures.
    pub fn list(&self, limit: usize) -> Result<Vec<MemoryRecord>, MemoryError> {
        if limit == 0 || limit > MAX_RESULTS {
            return Err(MemoryError::Invalid(format!(
                "memory list limit must be between 1 and {MAX_RESULTS}"
            )));
        }
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id, kind, source, title, content, tags_json, source_run_id,
                        source_integrity_hash, created_at, updated_at, access_count
                 FROM memories WHERE active = 1
                 ORDER BY updated_at DESC, id ASC LIMIT ?1",
            )
            .map_err(MemoryError::Database)?;
        let rows = statement
            .query_map([i64::try_from(limit).unwrap_or(i64::MAX)], decode_row)
            .map_err(MemoryError::Database)?;
        let mut memories = Vec::new();
        for row in rows {
            memories.push(row.map_err(MemoryError::Database)??);
        }
        Ok(memories)
    }

    /// Soft-deletes one memory while retaining provenance for diagnostics.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError::NotFound`] when the entry is absent or inactive.
    pub fn forget(&self, id: MemoryId) -> Result<(), MemoryError> {
        let now = format_time(OffsetDateTime::now_utc())?;
        let updated = self
            .connection()?
            .execute(
                "UPDATE memories SET active = 0, updated_at = ?1
                 WHERE id = ?2 AND active = 1",
                params![now, id.to_string()],
            )
            .map_err(MemoryError::Database)?;
        if updated == 0 {
            return Err(MemoryError::NotFound(id));
        }
        Ok(())
    }

    fn find_by_run_id(&self, run_id: RunId) -> Result<Option<MemoryRecord>, MemoryError> {
        self.connection()?
            .query_row(
                "SELECT id, kind, source, title, content, tags_json, source_run_id,
                        source_integrity_hash, created_at, updated_at, access_count
                 FROM memories WHERE source_run_id = ?1",
                [run_id.to_string()],
                decode_row,
            )
            .optional()
            .map_err(MemoryError::Database)?
            .transpose()
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, MemoryError> {
        self.connection.lock().map_err(|_| MemoryError::Poisoned)
    }
}

fn decode_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Result<MemoryRecord, MemoryError>> {
    let id = row.get::<_, String>(0)?;
    let kind = row.get::<_, String>(1)?;
    let source = row.get::<_, String>(2)?;
    let tags = row.get::<_, String>(5)?;
    let source_run_id = row.get::<_, Option<String>>(6)?;
    let created_at = row.get::<_, String>(8)?;
    let updated_at = row.get::<_, String>(9)?;
    let access_count = row.get::<_, i64>(10)?;
    Ok((|| {
        Ok(MemoryRecord {
            id: MemoryId::from_str(&id).map_err(MemoryError::Id)?,
            kind: MemoryKind::from_str(&kind)?,
            source: MemorySource::from_str(&source)?,
            title: row.get(3).map_err(MemoryError::Database)?,
            content: row.get(4).map_err(MemoryError::Database)?,
            tags: serde_json::from_str(&tags).map_err(MemoryError::Serialization)?,
            source_run_id: source_run_id
                .map(|value| RunId::from_str(&value).map_err(MemoryError::Id))
                .transpose()?,
            source_integrity_hash: row.get(7).map_err(MemoryError::Database)?,
            created_at: parse_time(&created_at)?,
            updated_at: parse_time(&updated_at)?,
            access_count: u64::try_from(access_count)
                .map_err(|_| MemoryError::Corrupt("negative access count".to_owned()))?,
        })
    })())
}

fn unversioned_object_count(connection: &Connection) -> Result<i64, MemoryError> {
    connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )
        .map_err(MemoryError::Database)
}

fn validate_database_target(path: &Path) -> Result<(), MemoryError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(MemoryError::InvalidDatabasePath(path.to_path_buf()))
        }
        Ok(_) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(MemoryError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn validate_draft(draft: &MemoryDraft) -> Result<(), MemoryError> {
    let title = draft.title.trim();
    let content = draft.content.trim();
    if title.is_empty() || title.len() > MAX_TITLE_BYTES {
        return Err(MemoryError::Invalid(format!(
            "memory title must be 1..={MAX_TITLE_BYTES} bytes"
        )));
    }
    if content.is_empty() || content.len() > MAX_CONTENT_BYTES {
        return Err(MemoryError::Invalid(format!(
            "memory content must be 1..={MAX_CONTENT_BYTES} bytes"
        )));
    }
    if title.chars().any(char::is_control) || content.contains('\0') {
        return Err(MemoryError::Invalid(
            "memory contains forbidden control characters".to_owned(),
        ));
    }
    if draft.tags.len() > MAX_TAGS {
        return Err(MemoryError::Invalid(format!(
            "memory accepts at most {MAX_TAGS} tags"
        )));
    }
    Ok(())
}

fn normalize_tags(tags: Vec<String>) -> Result<Vec<String>, MemoryError> {
    let mut normalized = BTreeSet::new();
    for tag in tags {
        let tag = tag.trim().to_lowercase();
        if tag.is_empty()
            || tag.len() > MAX_TAG_BYTES
            || !tag
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(MemoryError::Invalid(format!(
                "memory tags must be 1..={MAX_TAG_BYTES} bytes and use letters, numbers, '.', '-', or '_'"
            )));
        }
        normalized.insert(tag);
    }
    Ok(normalized.into_iter().collect())
}

fn applied_run_tags(receipt: &ChangeReceipt) -> Vec<String> {
    let mut tags = BTreeSet::from(["applied".to_owned()]);
    for path in receipt.changes.iter().take(MAX_TAGS - 1) {
        if let Some(extension) = Path::new(&path.path)
            .extension()
            .and_then(|value| value.to_str())
        {
            let tag = format!("ext-{}", extension.to_ascii_lowercase());
            if tag.len() <= MAX_TAG_BYTES {
                tags.insert(tag);
            }
        }
    }
    tags.into_iter().take(MAX_TAGS).collect()
}

fn bounded_title(value: &str) -> String {
    truncate_utf8(value, MAX_TITLE_BYTES)
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value[..boundary].to_owned()
}

fn query_tokens(query: &str) -> BTreeSet<String> {
    query
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .map(str::to_lowercase)
        .filter(|token| token.len() >= 2)
        .collect()
}

fn score_memory(memory: &MemoryRecord, tokens: &BTreeSet<String>, query: &str) -> u64 {
    let title = memory.title.to_lowercase();
    let content = memory.content.to_lowercase();
    let tags = memory.tags.join(" ").to_lowercase();
    let mut score = 0_u64;
    for token in tokens {
        score = score
            .saturating_add(u64::from(title.contains(token)) * 8)
            .saturating_add(u64::from(tags.contains(token)) * 6)
            .saturating_add(u64::from(content.contains(token)) * 3);
    }
    if !query.is_empty() {
        score = score
            .saturating_add(u64::from(title.contains(query)) * 16)
            .saturating_add(u64::from(content.contains(query)) * 8);
    }
    score
}

fn format_time(value: OffsetDateTime) -> Result<String, MemoryError> {
    value
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(MemoryError::TimeFormat)
}

fn parse_time(value: &str) -> Result<OffsetDateTime, MemoryError> {
    OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .map_err(MemoryError::TimeParse)
}

/// Durable memory failure.
#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("memory database failed: {0}")]
    Database(rusqlite::Error),
    #[error("memory serialization failed: {0}")]
    Serialization(serde_json::Error),
    #[error("memory timestamp formatting failed: {0}")]
    TimeFormat(time::error::Format),
    #[error("memory timestamp parsing failed: {0}")]
    TimeParse(time::error::Parse),
    #[error("memory I/O failed at {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("memory input is invalid: {0}")]
    Invalid(String),
    #[error("memory data is corrupt: {0}")]
    Corrupt(String),
    #[error("memory id is invalid: {0}")]
    Id(uuid::Error),
    #[error("receipt validation failed: {0}")]
    Receipt(pactrail_core::ReceiptError),
    #[error("memory {0} was not found")]
    NotFound(MemoryId),
    #[error("applied run {0} already has memory with different provenance")]
    ProvenanceConflict(RunId),
    #[error("memory database schema version {0} is unsupported")]
    UnsupportedDatabaseSchema(i64),
    #[error("memory database path is not a regular, non-symlink file: {0}")]
    InvalidDatabasePath(std::path::PathBuf),
    #[error("schema-zero memory database contains {0} unrecognized user object(s)")]
    UnexpectedUnversionedObjects(i64),
    #[error("memory database is not an uninitialized schema-zero file")]
    ExpectedUninitializedDatabase,
    #[error("memory database lock was poisoned")]
    Poisoned,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_memory_round_trips_and_ranks_relevance() {
        let store = MemoryStore::open_in_memory()
            .unwrap_or_else(|error| unreachable!("memory store: {error}"));
        store
            .remember(MemoryDraft {
                kind: MemoryKind::Convention,
                title: "Parser testing".to_owned(),
                content: "Every parser fix needs a malformed-input regression test.".to_owned(),
                tags: vec!["Tests".to_owned(), "parser".to_owned()],
            })
            .unwrap_or_else(|error| unreachable!("remember: {error}"));
        store
            .remember(MemoryDraft {
                kind: MemoryKind::Decision,
                title: "UI palette".to_owned(),
                content: "Use cyan for active progress.".to_owned(),
                tags: vec!["ui".to_owned()],
            })
            .unwrap_or_else(|error| unreachable!("remember: {error}"));

        let matches = store
            .search("parser regression", 5)
            .unwrap_or_else(|error| unreachable!("search: {error}"));
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].memory.title, "Parser testing");
        assert_eq!(matches[0].memory.tags, vec!["parser", "tests"]);
        assert_eq!(matches[0].memory.access_count, 1);
    }

    #[test]
    fn forgetting_is_idempotently_rejected() {
        let store = MemoryStore::open_in_memory()
            .unwrap_or_else(|error| unreachable!("memory store: {error}"));
        let memory = store
            .remember(MemoryDraft {
                kind: MemoryKind::Warning,
                title: "Generated files".to_owned(),
                content: "Do not edit generated bindings.".to_owned(),
                tags: Vec::new(),
            })
            .unwrap_or_else(|error| unreachable!("remember: {error}"));
        store
            .forget(memory.id)
            .unwrap_or_else(|error| unreachable!("forget: {error}"));
        assert!(matches!(
            store.forget(memory.id),
            Err(MemoryError::NotFound(_))
        ));
        assert!(
            store
                .search("generated", 5)
                .unwrap_or_else(|error| unreachable!("search: {error}"))
                .is_empty()
        );
    }

    #[test]
    fn future_schema_fails_closed() {
        let directory = tempfile::tempdir()
            .unwrap_or_else(|error| unreachable!("temporary directory: {error}"));
        let path = directory.path().join("memory.sqlite3");
        let connection = Connection::open(&path)
            .unwrap_or_else(|error| unreachable!("fixture database: {error}"));
        connection
            .pragma_update(None, "user_version", 99)
            .unwrap_or_else(|error| unreachable!("fixture schema: {error}"));
        drop(connection);

        assert!(matches!(
            MemoryStore::open(&path),
            Err(MemoryError::UnsupportedDatabaseSchema(99))
        ));
        assert_eq!(
            MemoryStore::database_schema_version(path).ok(),
            Some(Some(99))
        );
    }

    #[test]
    fn schema_inspection_does_not_create_and_initialized_store_is_current() {
        let directory = tempfile::tempdir()
            .unwrap_or_else(|error| unreachable!("temporary directory: {error}"));
        let path = directory.path().join("memory.sqlite3");
        assert_eq!(MemoryStore::database_schema_version(&path).ok(), Some(None));
        drop(MemoryStore::open(&path).unwrap_or_else(|error| unreachable!("open: {error}")));
        assert_eq!(
            MemoryStore::database_schema_version(path).ok(),
            Some(Some(MEMORY_DATABASE_SCHEMA_VERSION))
        );
    }

    #[test]
    fn unversioned_user_objects_are_never_stamped_as_memory_state() {
        let directory = tempfile::tempdir()
            .unwrap_or_else(|error| unreachable!("temporary directory: {error}"));
        let path = directory.path().join("memory.sqlite3");
        let connection = Connection::open(&path)
            .unwrap_or_else(|error| unreachable!("fixture database: {error}"));
        connection
            .execute("CREATE TABLE foreign_data (value TEXT)", [])
            .unwrap_or_else(|error| unreachable!("fixture table: {error}"));
        drop(connection);

        assert!(matches!(
            MemoryStore::open(&path),
            Err(MemoryError::UnexpectedUnversionedObjects(1))
        ));
        assert!(matches!(
            MemoryStore::validate_uninitialized(&path),
            Err(MemoryError::UnexpectedUnversionedObjects(1))
        ));
        assert_eq!(
            MemoryStore::database_schema_version(path).ok(),
            Some(Some(0))
        );
    }

    #[test]
    fn open_rejects_non_file_database_targets() {
        let directory = tempfile::tempdir()
            .unwrap_or_else(|error| unreachable!("temporary directory: {error}"));
        let path = directory.path().join("memory.sqlite3");
        fs::create_dir(&path).unwrap_or_else(|error| unreachable!("fixture directory: {error}"));

        assert!(matches!(
            MemoryStore::open(&path),
            Err(MemoryError::InvalidDatabasePath(found)) if found == path
        ));
    }
}
