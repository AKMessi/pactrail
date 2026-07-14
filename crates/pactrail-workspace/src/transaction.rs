use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use pactrail_core::FileChange;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::manifest::{apply_mode, fingerprint};
use crate::{PathError, SafeRelativePath, WorkspaceManifest};

const METADATA_FILE: &str = "transaction.json";
const WORKSPACE_DIRECTORY: &str = "workspace";
const APPLY_JOURNAL_DIRECTORY: &str = "apply-journal";

#[derive(Clone, Debug, Deserialize, Serialize)]
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
        let metadata = TransactionMetadata {
            schema_version: 1,
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
        let control_root =
            fs::canonicalize(control_root.as_ref()).map_err(|source| TransactionError::Io {
                path: control_root.as_ref().to_path_buf(),
                source,
            })?;
        let metadata_path = control_root.join(METADATA_FILE);
        let metadata: TransactionMetadata = read_json(&metadata_path)?;
        if metadata.schema_version != 1 {
            return Err(TransactionError::UnsupportedSchema(metadata.schema_version));
        }
        let workspace_root = control_root.join(WORKSPACE_DIRECTORY);
        if !workspace_root.is_dir() {
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
        self.check_source_drift(&changes)?;
        let journal_root = self.control_root.join(APPLY_JOURNAL_DIRECTORY);
        if journal_root.exists() {
            fs::remove_dir_all(&journal_root).map_err(|source| TransactionError::Io {
                path: journal_root.clone(),
                source,
            })?;
        }
        fs::create_dir_all(&journal_root).map_err(|source| TransactionError::Io {
            path: journal_root.clone(),
            source,
        })?;
        backup_changed_files(&self.metadata.source_root, &journal_root, &changes)?;

        if let Err(apply_error) = self.apply_changes(&changes) {
            if let Err(rollback_error) = rollback(
                &self.metadata.source_root,
                &journal_root,
                &self.metadata.baseline,
                &changes,
            ) {
                return Err(TransactionError::RollbackFailed {
                    apply: Box::new(apply_error),
                    rollback: Box::new(rollback_error),
                });
            }
            return Err(apply_error);
        }

        let resulting = WorkspaceManifest::capture(&self.metadata.source_root)?;
        fs::remove_dir_all(&journal_root).map_err(|source| TransactionError::Io {
            path: journal_root,
            source,
        })?;
        Ok(ApplyOutcome {
            changed_files: changes.len(),
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

    fn check_source_drift(&self, changes: &[FileChange]) -> Result<(), TransactionError> {
        for change in changes {
            let source = self.metadata.source_root.join(&change.path);
            let actual = if source.exists() {
                Some(fingerprint(&source)?.digest)
            } else {
                None
            };
            if actual != change.before_digest {
                return Err(TransactionError::BaselineDrift {
                    path: change.path.clone(),
                    expected: change.before_digest.clone(),
                    actual,
                });
            }
        }
        Ok(())
    }

    fn apply_changes(&self, changes: &[FileChange]) -> Result<(), TransactionError> {
        for change in changes {
            let source = self.metadata.source_root.join(&change.path);
            let candidate = self.workspace_root.join(&change.path);
            match &change.after_digest {
                Some(_) => {
                    let mut content = Vec::new();
                    File::open(&candidate)
                        .and_then(|mut file| file.read_to_end(&mut content))
                        .map_err(|error| TransactionError::Io {
                            path: candidate.clone(),
                            source: error,
                        })?;
                    if let Some(parent) = source.parent() {
                        fs::create_dir_all(parent).map_err(|error| TransactionError::Io {
                            path: parent.to_path_buf(),
                            source: error,
                        })?;
                    }
                    let mode = fingerprint(&candidate)?.unix_mode;
                    write_atomic(&source, &content, mode)?;
                }
                None => fs::remove_file(&source).map_err(|error| TransactionError::Io {
                    path: source,
                    source: error,
                })?,
            }
        }
        Ok(())
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
) -> Result<(), TransactionError> {
    for change in changes {
        if change.before_digest.is_none() {
            continue;
        }
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
    }
    Ok(())
}

fn rollback(
    source: &Path,
    journal: &Path,
    baseline: &WorkspaceManifest,
    changes: &[FileChange],
) -> Result<(), TransactionError> {
    for change in changes {
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
    write_atomic(path, &bytes, None)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, TransactionError> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|source| TransactionError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    serde_json::from_slice(&bytes).map_err(TransactionError::Serialization)
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

    if destination.exists() {
        let mut output = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(destination)
            .map_err(|source| TransactionError::Io {
                path: destination.to_path_buf(),
                source,
            })?;
        output
            .write_all(content)
            .and_then(|()| output.sync_all())
            .map_err(|source| TransactionError::Io {
                path: destination.to_path_buf(),
                source,
            })?;
        apply_mode(destination, mode)?;
        return Ok(());
    }
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
    #[error("at least one write scope is required")]
    EmptyWriteScope,
    #[error("write is outside the task contract scope: {0}")]
    WriteOutsideScope(String),
    #[error("transaction metadata schema {0} is unsupported")]
    UnsupportedSchema(u32),
    #[error("transaction metadata is invalid: {0}")]
    Serialization(serde_json::Error),
    #[error(
        "source file {path} changed after the run started (expected {expected:?}, actual {actual:?})"
    )]
    BaselineDrift {
        path: String,
        expected: Option<String>,
        actual: Option<String>,
    },
    #[error("apply failed and rollback also failed; apply: {apply}; rollback: {rollback}")]
    RollbackFailed {
        apply: Box<TransactionError>,
        rollback: Box<TransactionError>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
