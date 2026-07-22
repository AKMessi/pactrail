use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use pactrail_core::FileChange;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::manifest::{apply_mode, fingerprint, manifest_digest, unix_mode};
use crate::{PathError, SafeRelativePath, WorkspaceManifest};

const METADATA_FILE: &str = "transaction.json";
const WORKSPACE_DIRECTORY: &str = "workspace";
const APPLY_JOURNAL_DIRECTORY: &str = "apply-journal";
const MAX_TRANSACTION_METADATA_BYTES: u64 = 64 * 1024 * 1024;

/// Current schema for isolated workspace transaction metadata.
pub const TRANSACTION_SCHEMA_VERSION: u32 = 1;

/// Oldest transaction metadata schema this binary can reopen.
pub const MIN_TRANSACTION_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TransactionMetadata {
    schema_version: u32,
    source_root: PathBuf,
    allowed_write_paths: Vec<String>,
    baseline: WorkspaceManifest,
}

/// Result of landing a workspace transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyOutcome {
    pub changed_files: usize,
    pub baseline_digest: String,
    pub resulting_digest: String,
}

/// Isolated copy of a source workspace with a durable baseline manifest.
#[derive(Clone, Debug)]
pub struct WorkspaceTransaction {
    control_root: PathBuf,
    workspace_root: PathBuf,
    metadata: TransactionMetadata,
}

impl WorkspaceTransaction {
    /// Creates a transaction below an empty `control_root`.
    ///
    /// # Errors
    ///
    /// Returns an error if roots cannot be canonicalized, the destination is
    /// non-empty, a path scope is unsafe, or the source cannot be copied.
    pub fn create(
        source_root: impl AsRef<Path>,
        control_root: impl AsRef<Path>,
        allowed_write_paths: &[String],
    ) -> Result<Self, TransactionError> {
        let source_root =
            fs::canonicalize(source_root.as_ref()).map_err(|source| TransactionError::Io {
                path: source_root.as_ref().to_path_buf(),
                source,
            })?;
        if !source_root.is_dir() {
            return Err(TransactionError::NotDirectory(source_root));
        }
        if allowed_write_paths.is_empty() {
            return Err(TransactionError::EmptyWriteScope);
        }
        for scope in allowed_write_paths {
            if scope != "." {
                SafeRelativePath::new(scope)?;
            }
        }
        let control_root = control_root.as_ref().to_path_buf();
        if control_root.exists()
            && fs::read_dir(&control_root)
                .map_err(|source| TransactionError::Io {
                    path: control_root.clone(),
                    source,
                })?
                .next()
                .is_some()
        {
            return Err(TransactionError::DestinationNotEmpty(control_root));
        }
        fs::create_dir_all(&control_root).map_err(|source| TransactionError::Io {
            path: control_root.clone(),
            source,
        })?;
        let workspace_root = control_root.join(WORKSPACE_DIRECTORY);
        fs::create_dir(&workspace_root).map_err(|source| TransactionError::Io {
            path: workspace_root.clone(),
            source,
        })?;
        let baseline = WorkspaceManifest::capture(&source_root)?;
        copy_manifest_files(&source_root, &workspace_root, &baseline)?;
        let copied = WorkspaceManifest::capture(&workspace_root)?;
        if copied != baseline {
            return Err(TransactionError::SnapshotCopyMismatch {
                expected: baseline.digest,
                actual: copied.digest,
            });
        }
        let source_after_copy = WorkspaceManifest::capture(&source_root)?;
        if source_after_copy != baseline {
            return Err(TransactionError::SourceChangedDuringSnapshot {
                expected: baseline.digest,
                actual: source_after_copy.digest,
            });
        }
        let metadata = TransactionMetadata {
            schema_version: TRANSACTION_SCHEMA_VERSION,
            source_root,
            allowed_write_paths: allowed_write_paths.to_vec(),
            baseline,
        };
        write_json_atomic(&control_root.join(METADATA_FILE), &metadata)?;
        Ok(Self {
            control_root,
            workspace_root,
            metadata,
        })
    }

    /// Reopens an existing transaction after validating its metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if transaction metadata is missing, malformed, or uses
    /// an unsupported schema, or if the workspace copy is missing.
    pub fn open(control_root: impl AsRef<Path>) -> Result<Self, TransactionError> {
        let requested_root = control_root.as_ref();
        let requested_metadata =
            fs::symlink_metadata(requested_root).map_err(|source| TransactionError::Io {
                path: requested_root.to_path_buf(),
                source,
            })?;
        if requested_metadata.file_type().is_symlink() {
            return Err(TransactionError::SymbolicLink(requested_root.to_path_buf()));
        }
        if !requested_metadata.is_dir() {
            return Err(TransactionError::NotDirectory(requested_root.to_path_buf()));
        }
        let control_root =
            fs::canonicalize(requested_root).map_err(|source| TransactionError::Io {
                path: requested_root.to_path_buf(),
                source,
            })?;
        let metadata_path = control_root.join(METADATA_FILE);
        let metadata: TransactionMetadata = read_json(&metadata_path)?;
        if metadata.schema_version != TRANSACTION_SCHEMA_VERSION {
            return Err(TransactionError::UnsupportedSchema(metadata.schema_version));
        }
        validate_metadata(&metadata)?;
        let workspace_root = control_root.join(WORKSPACE_DIRECTORY);
        let workspace_metadata =
            fs::symlink_metadata(&workspace_root).map_err(|source| TransactionError::Io {
                path: workspace_root.clone(),
                source,
            })?;
        if workspace_metadata.file_type().is_symlink() {
            return Err(TransactionError::SymbolicLink(workspace_root));
        }
        if !workspace_metadata.is_dir() {
            return Err(TransactionError::NotDirectory(workspace_root));
        }
        Ok(Self {
            control_root,
            workspace_root,
            metadata,
        })
    }

    /// Root visible to models and tools.
    #[must_use]
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Run-local control directory containing transaction metadata and the
    /// isolated workspace.
    #[must_use]
    pub fn control_root(&self) -> &Path {
        &self.control_root
    }

    /// Original workspace that may receive an explicit apply.
    #[must_use]
    pub fn source_root(&self) -> &Path {
        &self.metadata.source_root
    }

    /// Digest of the immutable source baseline.
    #[must_use]
    pub fn baseline_digest(&self) -> &str {
        &self.metadata.baseline.digest
    }

    /// Resolves a readable relative path below the transaction root.
    ///
    /// # Errors
    ///
    /// Returns an error for unsafe paths or symbolic-link traversal.
    pub fn resolve_read(&self, relative: impl AsRef<Path>) -> Result<PathBuf, TransactionError> {
        let relative = SafeRelativePath::new(relative)?;
        let candidate = self.workspace_root.join(relative.as_path());
        ensure_no_symlink_ancestors(&self.workspace_root, &candidate)?;
        Ok(candidate)
    }

    /// Resolves a writable relative path after enforcing the contract scope.
    ///
    /// # Errors
    ///
    /// Returns an error for unsafe paths, paths outside the write scope, or
    /// symbolic-link traversal.
    pub fn resolve_write(&self, relative: impl AsRef<Path>) -> Result<PathBuf, TransactionError> {
        let relative = SafeRelativePath::new(relative)?;
        if !self.is_write_allowed(&relative) {
            return Err(TransactionError::WriteOutsideScope(relative.portable()));
        }
        let candidate = self.workspace_root.join(relative.as_path());
        ensure_no_symlink_ancestors(&self.workspace_root, &candidate)?;
        Ok(candidate)
    }

    /// Returns the current BLAKE3 digest of one regular candidate file.
    ///
    /// Missing paths return `None`. Resolution remains workspace-relative and
    /// rejects symbolic-link traversal, non-regular files, and unsafe paths.
    ///
    /// # Errors
    ///
    /// Returns an error when path validation, metadata access, or streaming
    /// fingerprinting fails.
    pub fn current_file_digest(
        &self,
        relative: impl AsRef<Path>,
    ) -> Result<Option<String>, TransactionError> {
        let path = self.resolve_read(relative)?;
        fingerprint_optional(&self.workspace_root, &path).map(|value| value.digest)
    }

    /// Writes one file through the transaction's path policy.
    ///
    /// # Errors
    ///
    /// Returns an error when resolution, directory creation, or atomic writing fails.
    pub fn write_file(
        &self,
        relative: impl AsRef<Path>,
        content: &[u8],
    ) -> Result<(), TransactionError> {
        let destination = self.resolve_write(relative)?;
        let parent = destination
            .parent()
            .ok_or_else(|| TransactionError::EscapedRoot(destination.clone()))?;
        fs::create_dir_all(parent).map_err(|source| TransactionError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
        write_atomic(&destination, content, None)
    }

    /// Deletes one file through the transaction's path policy.
    ///
    /// # Errors
    ///
    /// Returns an error when resolution fails, the target is not a regular file,
    /// or deletion fails.
    pub fn remove_file(&self, relative: impl AsRef<Path>) -> Result<(), TransactionError> {
        let path = self.resolve_write(relative)?;
        let metadata = fs::symlink_metadata(&path).map_err(|source| TransactionError::Io {
            path: path.clone(),
            source,
        })?;
        if !metadata.file_type().is_file() {
            return Err(TransactionError::UnsupportedFile(path));
        }
        fs::remove_file(&path).map_err(|source| TransactionError::Io { path, source })
    }

    /// Returns a stable set of file-level changes from the baseline.
    ///
    /// # Errors
    ///
    /// Returns an error when the transaction cannot be rescanned.
    pub fn changes(&self) -> Result<Vec<FileChange>, TransactionError> {
        let current = WorkspaceManifest::capture(&self.workspace_root)?;
        let paths: BTreeSet<&String> = self
            .metadata
            .baseline
            .files
            .keys()
            .chain(current.files.keys())
            .collect();
        let mut changes = Vec::new();
        for path in paths {
            let before = self.metadata.baseline.files.get(path);
            let after = current.files.get(path);
            if before == after {
                continue;
            }
            changes.push(FileChange {
                path: path.clone(),
                before_digest: before.map(|item| item.digest.clone()),
                after_digest: after.map(|item| item.digest.clone()),
                before_unix_mode: before.and_then(|item| item.unix_mode),
                after_unix_mode: after.and_then(|item| item.unix_mode),
                bytes_added: after.map_or(0, |item| item.bytes),
                bytes_removed: before.map_or(0, |item| item.bytes),
            });
        }
        Ok(changes)
    }

    /// Lands all changes after proving touched source files still match the baseline.
    ///
    /// Apply is journaled. An I/O failure triggers best-effort rollback and leaves
    /// the journal available for diagnosis when rollback itself fails.
    ///
    /// # Errors
    ///
    /// Returns an error for baseline drift, unsafe paths, unsupported file types,
    /// or an apply/rollback I/O failure.
    pub fn apply(&self) -> Result<ApplyOutcome, TransactionError> {
        let changes = self.changes()?;
        self.apply_change_set(&changes)
    }

    /// Lands exactly the supplied, receipt-bound change set.
    ///
    /// # Errors
    ///
    /// Returns an error if the candidate has changed since the expected set was
    /// produced, or under the same conditions as [`Self::apply`].
    pub fn apply_expected(
        &self,
        expected: &[FileChange],
    ) -> Result<ApplyOutcome, TransactionError> {
        let current = self.changes()?;
        if current != expected {
            return Err(TransactionError::CandidateSetDrift);
        }
        self.apply_change_set(expected)
    }

    fn apply_change_set(&self, changes: &[FileChange]) -> Result<ApplyOutcome, TransactionError> {
        self.apply_change_set_with_faults(changes, &NoApplyFaults)
    }

    fn apply_change_set_with_faults(
        &self,
        changes: &[FileChange],
        faults: &impl ApplyFaultInjector,
    ) -> Result<ApplyOutcome, TransactionError> {
        let journal_root = self.control_root.join(APPLY_JOURNAL_DIRECTORY);
        if journal_root.exists() {
            match self.source_state(changes)? {
                SourceState::Applied => {
                    let outcome = self.current_apply_outcome(changes.len())?;
                    faults.check(ApplyFaultPoint::JournalCleanup)?;
                    fs::remove_dir_all(&journal_root).map_err(|source| TransactionError::Io {
                        path: journal_root,
                        source,
                    })?;
                    return Ok(outcome);
                }
                SourceState::Partial => rollback(
                    &self.metadata.source_root,
                    &journal_root,
                    &self.metadata.baseline,
                    changes,
                    faults,
                )?,
                SourceState::Baseline => {}
                SourceState::Drift {
                    path,
                    expected,
                    actual,
                    expected_mode,
                    actual_mode,
                } => {
                    return Err(baseline_drift(
                        path,
                        expected,
                        actual,
                        expected_mode,
                        actual_mode,
                    ));
                }
            }
            faults.check(ApplyFaultPoint::JournalCleanup)?;
            fs::remove_dir_all(&journal_root).map_err(|source| TransactionError::Io {
                path: journal_root.clone(),
                source,
            })?;
        }
        match self.source_state(changes)? {
            SourceState::Baseline => {}
            SourceState::Applied => return self.current_apply_outcome(changes.len()),
            SourceState::Partial => {
                return Err(TransactionError::Invariant(
                    "source contains a partial apply without a recovery journal".to_owned(),
                ));
            }
            SourceState::Drift {
                path,
                expected,
                actual,
                expected_mode,
                actual_mode,
            } => {
                return Err(baseline_drift(
                    path,
                    expected,
                    actual,
                    expected_mode,
                    actual_mode,
                ));
            }
        }
        faults.check(ApplyFaultPoint::JournalCreate)?;
        fs::create_dir_all(&journal_root).map_err(|source| TransactionError::Io {
            path: journal_root.clone(),
            source,
        })?;
        backup_changed_files(&self.metadata.source_root, &journal_root, changes, faults)?;

        if let Err(apply_error) = self.apply_changes(changes, faults) {
            if let Err(rollback_error) = rollback(
                &self.metadata.source_root,
                &journal_root,
                &self.metadata.baseline,
                changes,
                faults,
            ) {
                return Err(TransactionError::RollbackFailed {
                    apply: Box::new(apply_error),
                    rollback: Box::new(rollback_error),
                });
            }
            return Err(apply_error);
        }

        let outcome = self.current_apply_outcome(changes.len())?;
        faults.check(ApplyFaultPoint::JournalCleanup)?;
        fs::remove_dir_all(&journal_root).map_err(|source| TransactionError::Io {
            path: journal_root,
            source,
        })?;
        Ok(outcome)
    }

    fn current_apply_outcome(
        &self,
        changed_files: usize,
    ) -> Result<ApplyOutcome, TransactionError> {
        let resulting = WorkspaceManifest::capture(&self.metadata.source_root)?;
        Ok(ApplyOutcome {
            changed_files,
            baseline_digest: self.metadata.baseline.digest.clone(),
            resulting_digest: resulting.digest,
        })
    }

    fn is_write_allowed(&self, relative: &SafeRelativePath) -> bool {
        self.metadata.allowed_write_paths.iter().any(|scope| {
            if scope == "." {
                return true;
            }
            SafeRelativePath::new(scope)
                .is_ok_and(|safe| relative.as_path().starts_with(safe.as_path()))
        })
    }

    fn source_state(&self, changes: &[FileChange]) -> Result<SourceState, TransactionError> {
        let mut all_baseline = true;
        let mut all_applied = true;
        let mut all_known = true;
        let mut first_foreign_drift = None;
        for change in changes {
            let source = self.metadata.source_root.join(&change.path);
            let actual = fingerprint_optional(&self.metadata.source_root, &source)?;
            let baseline = ChangeFingerprint {
                digest: change.before_digest.clone(),
                unix_mode: change.before_unix_mode,
            };
            let applied = ChangeFingerprint {
                digest: change.after_digest.clone(),
                unix_mode: change.after_unix_mode,
            };
            all_baseline &= actual == baseline;
            all_applied &= actual == applied;
            if actual != baseline && actual != applied {
                all_known = false;
            }
            if actual != baseline && actual != applied && first_foreign_drift.is_none() {
                first_foreign_drift = Some(SourceState::Drift {
                    path: change.path.clone(),
                    expected: change.before_digest.clone(),
                    actual: actual.digest.clone(),
                    expected_mode: change.before_unix_mode,
                    actual_mode: actual.unix_mode,
                });
            }
        }
        if all_baseline {
            Ok(SourceState::Baseline)
        } else if all_applied {
            Ok(SourceState::Applied)
        } else if all_known {
            Ok(SourceState::Partial)
        } else {
            first_foreign_drift.ok_or_else(|| {
                TransactionError::Invariant(
                    "mixed source state had no non-baseline file".to_owned(),
                )
            })
        }
    }

    fn apply_changes(
        &self,
        changes: &[FileChange],
        faults: &impl ApplyFaultInjector,
    ) -> Result<(), TransactionError> {
        for (index, change) in changes.iter().enumerate() {
            faults.check(ApplyFaultPoint::ApplyChange(index))?;
            let source = self.metadata.source_root.join(&change.path);
            let candidate = self.workspace_root.join(&change.path);
            ensure_no_symlink_ancestors(&self.metadata.source_root, &source)?;
            ensure_no_symlink_ancestors(&self.workspace_root, &candidate)?;
            let source_fingerprint = fingerprint_optional(&self.metadata.source_root, &source)?;
            let expected_source = ChangeFingerprint {
                digest: change.before_digest.clone(),
                unix_mode: change.before_unix_mode,
            };
            if source_fingerprint != expected_source {
                return Err(TransactionError::BaselineDrift {
                    path: change.path.clone(),
                    expected: expected_source.digest,
                    actual: source_fingerprint.digest,
                    expected_mode: expected_source.unix_mode,
                    actual_mode: source_fingerprint.unix_mode,
                });
            }
            match &change.after_digest {
                Some(expected_digest) => {
                    let mut content = Vec::new();
                    let mut file =
                        File::open(&candidate).map_err(|error| TransactionError::Io {
                            path: candidate.clone(),
                            source: error,
                        })?;
                    file.read_to_end(&mut content)
                        .map_err(|error| TransactionError::Io {
                            path: candidate.clone(),
                            source: error,
                        })?;
                    let metadata = file.metadata().map_err(|error| TransactionError::Io {
                        path: candidate.clone(),
                        source: error,
                    })?;
                    let actual_digest = blake3::hash(&content).to_hex().to_string();
                    let actual_mode = unix_mode(&metadata);
                    if &actual_digest != expected_digest || actual_mode != change.after_unix_mode {
                        return Err(TransactionError::CandidateDrift {
                            path: change.path.clone(),
                            expected: expected_digest.clone(),
                            actual: actual_digest,
                            expected_mode: change.after_unix_mode,
                            actual_mode,
                        });
                    }
                    if let Some(parent) = source.parent() {
                        fs::create_dir_all(parent).map_err(|error| TransactionError::Io {
                            path: parent.to_path_buf(),
                            source: error,
                        })?;
                    }
                    write_atomic(&source, &content, change.after_unix_mode)?;
                }
                None => fs::remove_file(&source).map_err(|error| TransactionError::Io {
                    path: source,
                    source: error,
                })?,
            }
            faults.check(ApplyFaultPoint::ApplyChangeWritten(index))?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ApplyFaultPoint {
    JournalCreate,
    BackupChange(usize),
    BackupChangeWritten(usize),
    ApplyChange(usize),
    ApplyChangeWritten(usize),
    RollbackChange(usize),
    RollbackChangeWritten(usize),
    JournalCleanup,
}

trait ApplyFaultInjector {
    fn check(&self, point: ApplyFaultPoint) -> Result<(), TransactionError>;
}

struct NoApplyFaults;

impl ApplyFaultInjector for NoApplyFaults {
    fn check(&self, _point: ApplyFaultPoint) -> Result<(), TransactionError> {
        Ok(())
    }
}

#[derive(Debug, Eq, PartialEq)]
enum SourceState {
    Baseline,
    Applied,
    Partial,
    Drift {
        path: String,
        expected: Option<String>,
        actual: Option<String>,
        expected_mode: Option<u32>,
        actual_mode: Option<u32>,
    },
}

#[derive(Debug, Eq, PartialEq)]
struct ChangeFingerprint {
    digest: Option<String>,
    unix_mode: Option<u32>,
}

fn baseline_drift(
    path: String,
    expected: Option<String>,
    actual: Option<String>,
    expected_mode: Option<u32>,
    actual_mode: Option<u32>,
) -> TransactionError {
    TransactionError::BaselineDrift {
        path,
        expected,
        actual,
        expected_mode,
        actual_mode,
    }
}

fn fingerprint_optional(root: &Path, path: &Path) -> Result<ChangeFingerprint, TransactionError> {
    ensure_no_symlink_ancestors(root, path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            let fingerprint = fingerprint(path)?;
            Ok(ChangeFingerprint {
                digest: Some(fingerprint.digest),
                unix_mode: fingerprint.unix_mode,
            })
        }
        Ok(_) => Err(TransactionError::UnsupportedFile(path.to_path_buf())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ChangeFingerprint {
            digest: None,
            unix_mode: None,
        }),
        Err(source) => Err(TransactionError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn copy_manifest_files(
    source: &Path,
    destination: &Path,
    manifest: &WorkspaceManifest,
) -> Result<(), TransactionError> {
    for (relative, file) in &manifest.files {
        let from = source.join(relative);
        let to = destination.join(relative);
        let parent = to
            .parent()
            .ok_or_else(|| TransactionError::EscapedRoot(to.clone()))?;
        fs::create_dir_all(parent).map_err(|error| TransactionError::Io {
            path: parent.to_path_buf(),
            source: error,
        })?;
        fs::copy(&from, &to).map_err(|error| TransactionError::Io {
            path: from,
            source: error,
        })?;
        apply_mode(&to, file.unix_mode)?;
    }
    Ok(())
}

fn backup_changed_files(
    source: &Path,
    journal: &Path,
    changes: &[FileChange],
    faults: &impl ApplyFaultInjector,
) -> Result<(), TransactionError> {
    for (index, change) in changes.iter().enumerate() {
        if change.before_digest.is_none() {
            continue;
        }
        faults.check(ApplyFaultPoint::BackupChange(index))?;
        let from = source.join(&change.path);
        let to = journal.join(&change.path);
        let parent = to
            .parent()
            .ok_or_else(|| TransactionError::EscapedRoot(to.clone()))?;
        fs::create_dir_all(parent).map_err(|error| TransactionError::Io {
            path: parent.to_path_buf(),
            source: error,
        })?;
        fs::copy(&from, &to).map_err(|error| TransactionError::Io {
            path: from,
            source: error,
        })?;
        faults.check(ApplyFaultPoint::BackupChangeWritten(index))?;
    }
    Ok(())
}

fn rollback(
    source: &Path,
    journal: &Path,
    baseline: &WorkspaceManifest,
    changes: &[FileChange],
    faults: &impl ApplyFaultInjector,
) -> Result<(), TransactionError> {
    for (index, change) in changes.iter().enumerate() {
        faults.check(ApplyFaultPoint::RollbackChange(index))?;
        let destination = source.join(&change.path);
        if change.before_digest.is_some() {
            let backup = journal.join(&change.path);
            let mut content = Vec::new();
            File::open(&backup)
                .and_then(|mut file| file.read_to_end(&mut content))
                .map_err(|error| TransactionError::Io {
                    path: backup,
                    source: error,
                })?;
            let mode = baseline
                .files
                .get(&change.path)
                .and_then(|file| file.unix_mode);
            write_atomic(&destination, &content, mode)?;
        } else if destination.exists() {
            fs::remove_file(&destination).map_err(|error| TransactionError::Io {
                path: destination,
                source: error,
            })?;
        }
        faults.check(ApplyFaultPoint::RollbackChangeWritten(index))?;
    }
    Ok(())
}

fn ensure_no_symlink_ancestors(root: &Path, candidate: &Path) -> Result<(), TransactionError> {
    let relative = candidate
        .strip_prefix(root)
        .map_err(|_| TransactionError::EscapedRoot(candidate.to_path_buf()))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(TransactionError::SymbolicLink(current));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(source) => {
                return Err(TransactionError::Io {
                    path: current,
                    source,
                });
            }
        }
    }
    Ok(())
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), TransactionError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(TransactionError::Serialization)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_TRANSACTION_METADATA_BYTES {
        return Err(TransactionError::ArtifactTooLarge {
            path: path.to_path_buf(),
            limit: MAX_TRANSACTION_METADATA_BYTES,
        });
    }
    write_atomic(path, &bytes, None)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, TransactionError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| TransactionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(TransactionError::SymbolicLink(path.to_path_buf()));
    }
    if !metadata.is_file() {
        return Err(TransactionError::UnsupportedFile(path.to_path_buf()));
    }
    if metadata.len() > MAX_TRANSACTION_METADATA_BYTES {
        return Err(TransactionError::ArtifactTooLarge {
            path: path.to_path_buf(),
            limit: MAX_TRANSACTION_METADATA_BYTES,
        });
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    File::open(path)
        .and_then(|file| {
            file.take(MAX_TRANSACTION_METADATA_BYTES.saturating_add(1))
                .read_to_end(&mut bytes)
        })
        .map_err(|source| TransactionError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_TRANSACTION_METADATA_BYTES {
        return Err(TransactionError::ArtifactTooLarge {
            path: path.to_path_buf(),
            limit: MAX_TRANSACTION_METADATA_BYTES,
        });
    }
    serde_json::from_slice(&bytes).map_err(TransactionError::Serialization)
}

fn validate_metadata(metadata: &TransactionMetadata) -> Result<(), TransactionError> {
    if metadata.allowed_write_paths.is_empty() {
        return Err(TransactionError::EmptyWriteScope);
    }
    for scope in &metadata.allowed_write_paths {
        if scope != "." {
            let _safe = SafeRelativePath::new(scope)?;
        }
    }
    if !metadata.source_root.is_absolute() {
        return Err(TransactionError::Invariant(
            "transaction source root is not absolute".to_owned(),
        ));
    }
    for (path, fingerprint) in &metadata.baseline.files {
        let safe = SafeRelativePath::new(path)?;
        if safe.portable() != *path || !valid_digest(&fingerprint.digest) {
            return Err(TransactionError::Invariant(format!(
                "transaction baseline entry {path:?} is malformed"
            )));
        }
    }
    let expected_digest = manifest_digest(&metadata.baseline.files);
    if metadata.baseline.digest != expected_digest {
        return Err(TransactionError::Invariant(
            "transaction baseline manifest digest is invalid".to_owned(),
        ));
    }
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn write_atomic(
    destination: &Path,
    content: &[u8],
    mode: Option<u32>,
) -> Result<(), TransactionError> {
    let parent = destination
        .parent()
        .ok_or_else(|| TransactionError::EscapedRoot(destination.to_path_buf()))?;
    fs::create_dir_all(parent).map_err(|source| TransactionError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let mut temporary = NamedTempFile::new_in(parent).map_err(|source| TransactionError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    temporary
        .write_all(content)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|source| TransactionError::Io {
            path: temporary.path().to_path_buf(),
            source,
        })?;
    apply_mode(temporary.path(), mode)?;

    temporary
        .persist(destination)
        .map_err(|error| TransactionError::Io {
            path: destination.to_path_buf(),
            source: error.error,
        })?;
    Ok(())
}

/// Failure while creating, mutating, diffing, or applying a transaction.
#[derive(Debug, Error)]
pub enum TransactionError {
    #[error("workspace I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("workspace traversal failed: {source}")]
    Walk { source: ignore::Error },
    #[error("unsafe workspace path: {0}")]
    Path(#[from] PathError),
    #[error("path escaped workspace root: {0}")]
    EscapedRoot(PathBuf),
    #[error("non-Unicode workspace path is unsupported: {0}")]
    NonUnicodePath(PathBuf),
    #[error("symbolic links are conservatively rejected in v1 transactions: {0}")]
    SymbolicLink(PathBuf),
    #[error("unsupported special file: {0}")]
    UnsupportedFile(PathBuf),
    #[error("workspace root is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("transaction destination is not empty: {0}")]
    DestinationNotEmpty(PathBuf),
    #[error(
        "source changed while its transaction snapshot was being created (expected manifest {expected}, actual {actual})"
    )]
    SourceChangedDuringSnapshot { expected: String, actual: String },
    #[error(
        "transaction copy does not match its captured manifest (expected {expected}, actual {actual})"
    )]
    SnapshotCopyMismatch { expected: String, actual: String },
    #[error("at least one write scope is required")]
    EmptyWriteScope,
    #[error("write is outside the task contract scope: {0}")]
    WriteOutsideScope(String),
    #[error("transaction metadata schema {0} is unsupported")]
    UnsupportedSchema(u32),
    #[error("transaction metadata is invalid: {0}")]
    Serialization(serde_json::Error),
    #[error("transaction artifact {path} exceeds the {limit}-byte safety limit")]
    ArtifactTooLarge { path: PathBuf, limit: u64 },
    #[error("transaction invariant failed: {0}")]
    Invariant(String),
    #[error(
        "source file {path} changed after the run started (expected digest {expected:?} mode {expected_mode:?}, actual digest {actual:?} mode {actual_mode:?})"
    )]
    BaselineDrift {
        path: String,
        expected: Option<String>,
        actual: Option<String>,
        expected_mode: Option<u32>,
        actual_mode: Option<u32>,
    },
    #[error(
        "candidate file {path} changed after receipt construction (expected digest {expected} mode {expected_mode:?}, actual digest {actual} mode {actual_mode:?})"
    )]
    CandidateDrift {
        path: String,
        expected: String,
        actual: String,
        expected_mode: Option<u32>,
        actual_mode: Option<u32>,
    },
    #[error("transaction candidate no longer matches the receipt-bound change set")]
    CandidateSetDrift,
    #[error("apply failed and rollback also failed; apply: {apply}; rollback: {rollback}")]
    RollbackFailed {
        apply: Box<TransactionError>,
        rollback: Box<TransactionError>,
    },
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::io::ErrorKind;

    use proptest::prelude::*;

    use super::*;

    #[derive(Default)]
    struct TestFaults {
        points: Vec<ApplyFaultPoint>,
        io_kind: Option<ErrorKind>,
    }

    impl TestFaults {
        fn invariant(point: ApplyFaultPoint) -> Self {
            Self {
                points: vec![point],
                io_kind: None,
            }
        }

        fn invariants(points: Vec<ApplyFaultPoint>) -> Self {
            Self {
                points,
                io_kind: None,
            }
        }

        fn io(points: Vec<ApplyFaultPoint>, kind: ErrorKind) -> Self {
            Self {
                points,
                io_kind: Some(kind),
            }
        }
    }

    impl ApplyFaultInjector for TestFaults {
        fn check(&self, point: ApplyFaultPoint) -> Result<(), TransactionError> {
            if !self.points.contains(&point) {
                return Ok(());
            }
            self.io_kind.map_or_else(
                || {
                    Err(TransactionError::Invariant(format!(
                        "injected transaction fault at {point:?}"
                    )))
                },
                |kind| {
                    Err(TransactionError::Io {
                        path: PathBuf::from(format!("injected-{point:?}")),
                        source: std::io::Error::from(kind),
                    })
                },
            )
        }
    }

    fn contains_io_kind(error: &TransactionError, expected: ErrorKind) -> bool {
        match error {
            TransactionError::Io { source, .. } => source.kind() == expected,
            TransactionError::RollbackFailed { apply, rollback } => {
                contains_io_kind(apply, expected) || contains_io_kind(rollback, expected)
            }
            _ => false,
        }
    }

    fn assert_two_file_source(source: &Path, first: &str, second: &str) {
        assert_eq!(
            fs::read_to_string(source.join("src/first.rs")).ok(),
            Some(first.to_owned())
        );
        assert_eq!(
            fs::read_to_string(source.join("src/second.rs")).ok(),
            Some(second.to_owned())
        );
    }

    fn fixture() -> (tempfile::TempDir, tempfile::TempDir, WorkspaceTransaction) {
        let source =
            tempfile::tempdir().unwrap_or_else(|error| unreachable!("source tempdir: {error}"));
        fs::create_dir(source.path().join("src"))
            .unwrap_or_else(|error| unreachable!("source directory: {error}"));
        fs::write(source.path().join("src/lib.rs"), "pub fn old() {}\n")
            .unwrap_or_else(|error| unreachable!("fixture file: {error}"));
        let control =
            tempfile::tempdir().unwrap_or_else(|error| unreachable!("control tempdir: {error}"));
        let control_path = control.path().join("run");
        let transaction =
            WorkspaceTransaction::create(source.path(), &control_path, &["src".to_owned()])
                .unwrap_or_else(|error| unreachable!("create transaction: {error}"));
        (source, control, transaction)
    }

    fn two_file_fixture() -> (tempfile::TempDir, tempfile::TempDir, WorkspaceTransaction) {
        let source =
            tempfile::tempdir().unwrap_or_else(|error| unreachable!("source tempdir: {error}"));
        fs::create_dir(source.path().join("src"))
            .unwrap_or_else(|error| unreachable!("source directory: {error}"));
        fs::write(source.path().join("src/first.rs"), "old first\n")
            .unwrap_or_else(|error| unreachable!("first fixture: {error}"));
        fs::write(source.path().join("src/second.rs"), "old second\n")
            .unwrap_or_else(|error| unreachable!("second fixture: {error}"));
        let control =
            tempfile::tempdir().unwrap_or_else(|error| unreachable!("control tempdir: {error}"));
        let transaction = WorkspaceTransaction::create(
            source.path(),
            control.path().join("run"),
            &["src".to_owned()],
        )
        .unwrap_or_else(|error| unreachable!("create transaction: {error}"));
        transaction
            .write_file("src/first.rs", b"new first\n")
            .unwrap_or_else(|error| unreachable!("first candidate: {error}"));
        transaction
            .write_file("src/second.rs", b"new second\n")
            .unwrap_or_else(|error| unreachable!("second candidate: {error}"));
        (source, control, transaction)
    }

    #[test]
    fn write_is_isolated_until_apply() {
        let (source, _control, transaction) = fixture();
        transaction
            .write_file("src/lib.rs", b"pub fn new() {}\n")
            .unwrap_or_else(|error| unreachable!("transaction write: {error}"));
        assert_eq!(
            fs::read_to_string(source.path().join("src/lib.rs")).ok(),
            Some("pub fn old() {}\n".to_owned())
        );
        assert_eq!(transaction.changes().map(|items| items.len()).ok(), Some(1));

        let outcome = transaction
            .apply()
            .unwrap_or_else(|error| unreachable!("apply transaction: {error}"));
        assert_eq!(outcome.changed_files, 1);
        assert_eq!(
            fs::read_to_string(source.path().join("src/lib.rs")).ok(),
            Some("pub fn new() {}\n".to_owned())
        );
        let repeated = transaction
            .apply()
            .unwrap_or_else(|error| unreachable!("idempotent apply: {error}"));
        assert_eq!(repeated.changed_files, 1);
    }

    #[test]
    fn refuses_to_overwrite_concurrent_user_change() {
        let (source, _control, transaction) = fixture();
        transaction
            .write_file("src/lib.rs", b"pub fn agent() {}\n")
            .unwrap_or_else(|error| unreachable!("transaction write: {error}"));
        fs::write(source.path().join("src/lib.rs"), "pub fn user() {}\n")
            .unwrap_or_else(|error| unreachable!("user edit: {error}"));
        assert!(matches!(
            transaction.apply(),
            Err(TransactionError::BaselineDrift { .. })
        ));
        assert_eq!(
            fs::read_to_string(source.path().join("src/lib.rs")).ok(),
            Some("pub fn user() {}\n".to_owned())
        );
    }

    #[test]
    fn write_scope_is_enforced() {
        let (_source, _control, transaction) = fixture();
        assert!(matches!(
            transaction.write_file("README.md", b"no"),
            Err(TransactionError::WriteOutsideScope(_))
        ));
    }

    #[test]
    fn transaction_reopens_from_durable_metadata() {
        let (_source, _control, transaction) = fixture();
        let reopened = WorkspaceTransaction::open(&transaction.control_root)
            .unwrap_or_else(|error| unreachable!("open transaction: {error}"));
        assert_eq!(reopened.baseline_digest(), transaction.baseline_digest());
    }

    #[test]
    fn apply_failure_rolls_back_and_retry_completes_from_the_journal() {
        let (source, _control, transaction) = two_file_fixture();
        let changes = transaction
            .changes()
            .unwrap_or_else(|error| unreachable!("changes: {error}"));
        let faults = TestFaults::invariant(ApplyFaultPoint::ApplyChange(1));
        assert!(matches!(
            transaction.apply_change_set_with_faults(&changes, &faults),
            Err(TransactionError::Invariant(message)) if message.contains("ApplyChange(1)")
        ));
        assert_eq!(
            fs::read_to_string(source.path().join("src/first.rs")).ok(),
            Some("old first\n".to_owned())
        );
        assert_eq!(
            fs::read_to_string(source.path().join("src/second.rs")).ok(),
            Some("old second\n".to_owned())
        );
        assert!(
            transaction
                .control_root
                .join(APPLY_JOURNAL_DIRECTORY)
                .is_dir()
        );

        transaction
            .apply_expected(&changes)
            .unwrap_or_else(|error| unreachable!("journal retry: {error}"));
        assert_eq!(
            fs::read_to_string(source.path().join("src/first.rs")).ok(),
            Some("new first\n".to_owned())
        );
        assert_eq!(
            fs::read_to_string(source.path().join("src/second.rs")).ok(),
            Some("new second\n".to_owned())
        );
        assert!(
            !transaction
                .control_root
                .join(APPLY_JOURNAL_DIRECTORY)
                .exists()
        );
    }

    #[test]
    fn rollback_failure_is_diagnosable_and_a_clean_retry_recovers() {
        let (source, _control, transaction) = two_file_fixture();
        let changes = transaction
            .changes()
            .unwrap_or_else(|error| unreachable!("changes: {error}"));
        let faults = TestFaults::invariants(vec![
            ApplyFaultPoint::ApplyChange(1),
            ApplyFaultPoint::RollbackChange(0),
        ]);
        assert!(matches!(
            transaction.apply_change_set_with_faults(&changes, &faults),
            Err(TransactionError::RollbackFailed { .. })
        ));
        assert_eq!(
            fs::read_to_string(source.path().join("src/first.rs")).ok(),
            Some("new first\n".to_owned())
        );
        assert_eq!(
            fs::read_to_string(source.path().join("src/second.rs")).ok(),
            Some("old second\n".to_owned())
        );

        transaction
            .apply_expected(&changes)
            .unwrap_or_else(|error| unreachable!("recovery retry: {error}"));
        assert_eq!(
            fs::read_to_string(source.path().join("src/first.rs")).ok(),
            Some("new first\n".to_owned())
        );
        assert_eq!(
            fs::read_to_string(source.path().join("src/second.rs")).ok(),
            Some("new second\n".to_owned())
        );
    }

    #[test]
    fn cleanup_failure_leaves_an_idempotently_recoverable_applied_journal() {
        let (source, _control, transaction) = two_file_fixture();
        let changes = transaction
            .changes()
            .unwrap_or_else(|error| unreachable!("changes: {error}"));
        let faults = TestFaults::invariant(ApplyFaultPoint::JournalCleanup);
        assert!(matches!(
            transaction.apply_change_set_with_faults(&changes, &faults),
            Err(TransactionError::Invariant(message)) if message.contains("JournalCleanup")
        ));
        assert_eq!(
            fs::read_to_string(source.path().join("src/first.rs")).ok(),
            Some("new first\n".to_owned())
        );
        assert!(
            transaction
                .control_root
                .join(APPLY_JOURNAL_DIRECTORY)
                .is_dir()
        );

        let recovered = transaction
            .apply_expected(&changes)
            .unwrap_or_else(|error| unreachable!("cleanup recovery: {error}"));
        assert_eq!(recovered.changed_files, 2);
        assert!(
            !transaction
                .control_root
                .join(APPLY_JOURNAL_DIRECTORY)
                .exists()
        );
    }

    #[test]
    fn platform_io_failure_matrix_preserves_or_recovers_every_apply_boundary() {
        let failure_kinds = [ErrorKind::PermissionDenied, ErrorKind::StorageFull];
        let points = [
            ApplyFaultPoint::JournalCreate,
            ApplyFaultPoint::BackupChange(0),
            ApplyFaultPoint::BackupChangeWritten(0),
            ApplyFaultPoint::BackupChange(1),
            ApplyFaultPoint::BackupChangeWritten(1),
            ApplyFaultPoint::ApplyChange(0),
            ApplyFaultPoint::ApplyChangeWritten(0),
            ApplyFaultPoint::ApplyChange(1),
            ApplyFaultPoint::ApplyChangeWritten(1),
            ApplyFaultPoint::JournalCleanup,
        ];

        for kind in failure_kinds {
            for point in points {
                let (source, _control, transaction) = two_file_fixture();
                let changes = transaction
                    .changes()
                    .unwrap_or_else(|error| unreachable!("changes: {error}"));
                let faults = TestFaults::io(vec![point], kind);
                let Err(error) = transaction.apply_change_set_with_faults(&changes, &faults) else {
                    unreachable!("fault {point:?} with {kind:?} unexpectedly succeeded")
                };
                assert!(
                    contains_io_kind(&error, kind),
                    "fault {point:?} did not preserve {kind:?}: {error}"
                );

                if point == ApplyFaultPoint::JournalCleanup {
                    assert_two_file_source(source.path(), "new first\n", "new second\n");
                } else {
                    assert_two_file_source(source.path(), "old first\n", "old second\n");
                }

                transaction
                    .apply_expected(&changes)
                    .unwrap_or_else(|error| {
                        unreachable!("recovery after {point:?} with {kind:?}: {error}")
                    });
                assert_two_file_source(source.path(), "new first\n", "new second\n");
                assert!(
                    !transaction
                        .control_root
                        .join(APPLY_JOURNAL_DIRECTORY)
                        .exists()
                );
            }
        }
    }

    #[test]
    fn rollback_io_failure_matrix_remains_diagnosable_and_retryable() {
        let failure_kinds = [ErrorKind::PermissionDenied, ErrorKind::StorageFull];
        let rollback_points = [
            ApplyFaultPoint::RollbackChange(0),
            ApplyFaultPoint::RollbackChangeWritten(0),
            ApplyFaultPoint::RollbackChange(1),
            ApplyFaultPoint::RollbackChangeWritten(1),
        ];

        for kind in failure_kinds {
            for rollback_point in rollback_points {
                let (source, _control, transaction) = two_file_fixture();
                let changes = transaction
                    .changes()
                    .unwrap_or_else(|error| unreachable!("changes: {error}"));
                let faults =
                    TestFaults::io(vec![ApplyFaultPoint::ApplyChange(1), rollback_point], kind);
                let Err(error) = transaction.apply_change_set_with_faults(&changes, &faults) else {
                    unreachable!(
                        "rollback fault {rollback_point:?} with {kind:?} unexpectedly succeeded"
                    )
                };
                assert!(matches!(error, TransactionError::RollbackFailed { .. }));
                assert!(contains_io_kind(&error, kind));
                if rollback_point == ApplyFaultPoint::RollbackChange(0) {
                    assert_two_file_source(source.path(), "new first\n", "old second\n");
                } else {
                    assert_two_file_source(source.path(), "old first\n", "old second\n");
                }

                transaction
                    .apply_expected(&changes)
                    .unwrap_or_else(|error| {
                        unreachable!(
                            "recovery after rollback {rollback_point:?} with {kind:?}: {error}"
                        )
                    });
                assert_two_file_source(source.path(), "new first\n", "new second\n");
                assert!(
                    !transaction
                        .control_root
                        .join(APPLY_JOURNAL_DIRECTORY)
                        .exists()
                );
            }
        }
    }

    #[test]
    fn oversized_and_extended_transaction_metadata_fail_closed() {
        let (_source, _control, transaction) = fixture();
        let metadata_path = transaction.control_root.join(METADATA_FILE);
        let original =
            fs::read(&metadata_path).unwrap_or_else(|error| unreachable!("read metadata: {error}"));
        let mut value: serde_json::Value = serde_json::from_slice(&original)
            .unwrap_or_else(|error| unreachable!("decode metadata: {error}"));
        value
            .as_object_mut()
            .unwrap_or_else(|| unreachable!("metadata object"))
            .insert("unexpected".to_owned(), serde_json::Value::Bool(true));
        fs::write(
            &metadata_path,
            serde_json::to_vec(&value)
                .unwrap_or_else(|error| unreachable!("encode metadata: {error}")),
        )
        .unwrap_or_else(|error| unreachable!("write extended metadata: {error}"));
        assert!(matches!(
            WorkspaceTransaction::open(&transaction.control_root),
            Err(TransactionError::Serialization(_))
        ));

        let file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&metadata_path)
            .unwrap_or_else(|error| unreachable!("open metadata: {error}"));
        file.set_len(MAX_TRANSACTION_METADATA_BYTES + 1)
            .unwrap_or_else(|error| unreachable!("extend metadata: {error}"));
        assert!(matches!(
            WorkspaceTransaction::open(&transaction.control_root),
            Err(TransactionError::ArtifactTooLarge { .. })
        ));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn arbitrary_candidate_bytes_land_exactly(content in prop::collection::vec(any::<u8>(), 0..8192)) {
            let (source, _control, transaction) = fixture();
            let mut expected = content;
            expected.push(0xff);
            transaction
                .write_file("src/lib.rs", &expected)
                .unwrap_or_else(|error| unreachable!("candidate: {error}"));
            let changes = transaction
                .changes()
                .unwrap_or_else(|error| unreachable!("changes: {error}"));
            prop_assert_eq!(changes.len(), 1);
            prop_assert!(transaction.apply_expected(&changes).is_ok());
            let landed = fs::read(source.path().join("src/lib.rs"))
                .unwrap_or_else(|error| unreachable!("landed content: {error}"));
            prop_assert_eq!(landed, expected);
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_mode_changes_are_receipted_and_applied() {
        use std::os::unix::fs::PermissionsExt;

        let (source, _control, transaction) = fixture();
        let candidate = transaction.workspace_root().join("src/lib.rs");
        let baseline_mode = fs::metadata(&candidate)
            .unwrap_or_else(|error| unreachable!("candidate metadata: {error}"))
            .permissions()
            .mode();
        let changed_mode = baseline_mode ^ 0o100;
        fs::set_permissions(&candidate, fs::Permissions::from_mode(changed_mode))
            .unwrap_or_else(|error| unreachable!("candidate chmod: {error}"));

        let changes = transaction
            .changes()
            .unwrap_or_else(|error| unreachable!("mode changes: {error}"));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].before_digest, changes[0].after_digest);
        assert_eq!(changes[0].before_unix_mode, Some(baseline_mode));
        assert_eq!(changes[0].after_unix_mode, Some(changed_mode));
        transaction
            .apply_expected(&changes)
            .unwrap_or_else(|error| unreachable!("mode apply: {error}"));
        let source_mode = fs::metadata(source.path().join("src/lib.rs"))
            .unwrap_or_else(|error| unreachable!("source metadata: {error}"))
            .permissions()
            .mode();
        assert_eq!(source_mode, changed_mode);
    }
}
