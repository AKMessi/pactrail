use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::Path;

use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};

use crate::TransactionError;

/// Digest and size of one regular workspace file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FileFingerprint {
    pub digest: String,
    pub bytes: u64,
    /// Unix-style permission bits where the platform exposes them.
    pub unix_mode: Option<u32>,
}

/// Stable, sorted snapshot of a workspace's regular files.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspaceManifest {
    pub files: BTreeMap<String, FileFingerprint>,
    pub digest: String,
}

impl WorkspaceManifest {
    /// Captures regular, non-ignored files below `root`.
    ///
    /// # Errors
    ///
    /// Returns an error when traversal, metadata access, reading, or path
    /// normalization fails. Symbolic links are rejected conservatively.
    pub fn capture(root: &Path) -> Result<Self, TransactionError> {
        let mut files = BTreeMap::new();
        let walker = WalkBuilder::new(root)
            .hidden(false)
            .git_ignore(true)
            .git_global(false)
            .git_exclude(true)
            .parents(true)
            .filter_entry(|entry| {
                let name = entry.file_name().to_string_lossy();
                name != ".git" && name != ".pactrail"
            })
            .build();

        for result in walker {
            let entry = result.map_err(|source| TransactionError::Walk { source })?;
            if entry.path() == root {
                continue;
            }
            let file_type = entry
                .file_type()
                .ok_or_else(|| TransactionError::UnsupportedFile(entry.path().to_path_buf()))?;
            if file_type.is_symlink() {
                return Err(TransactionError::SymbolicLink(entry.path().to_path_buf()));
            }
            if file_type.is_dir() {
                continue;
            }
            if !file_type.is_file() {
                return Err(TransactionError::UnsupportedFile(
                    entry.path().to_path_buf(),
                ));
            }
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|_| TransactionError::EscapedRoot(entry.path().to_path_buf()))?;
            let portable = portable(relative)?;
            let fingerprint = fingerprint(entry.path())?;
            files.insert(portable, fingerprint);
        }
        let digest = manifest_digest(&files);
        Ok(Self { files, digest })
    }
}

pub(crate) fn fingerprint(path: &Path) -> Result<FileFingerprint, TransactionError> {
    let file = File::open(path).map_err(|source| TransactionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let metadata = file.metadata().map_err(|source| TransactionError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut reader = BufReader::new(file);
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|source| TransactionError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(FileFingerprint {
        digest: hasher.finalize().to_hex().to_string(),
        bytes: metadata.len(),
        unix_mode: unix_mode(&metadata),
    })
}

pub(crate) fn portable(path: &Path) -> Result<String, TransactionError> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(value) => {
                let value = value
                    .to_str()
                    .ok_or_else(|| TransactionError::NonUnicodePath(path.to_path_buf()))?;
                components.push(value);
            }
            _ => return Err(TransactionError::EscapedRoot(path.to_path_buf())),
        }
    }
    Ok(components.join("/"))
}

pub(crate) fn manifest_digest(files: &BTreeMap<String, FileFingerprint>) -> String {
    let mut hasher = blake3::Hasher::new();
    for (path, fingerprint) in files {
        hasher.update(path.as_bytes());
        hasher.update(&[0]);
        hasher.update(fingerprint.digest.as_bytes());
        hasher.update(&fingerprint.bytes.to_le_bytes());
        hasher.update(&fingerprint.unix_mode.unwrap_or_default().to_le_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(unix)]
// The manifest format is shared with non-Unix hosts, where mode is absent.
// Keeping one return type prevents platform-specific serialization behavior.
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn unix_mode(metadata: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode())
}

#[cfg(not(unix))]
pub(crate) fn unix_mode(_metadata: &fs::Metadata) -> Option<u32> {
    None
}

// The shared result type is required because Unix permission updates can fail,
// even though the non-Unix implementation is intentionally a no-op.
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn apply_mode(path: &Path, mode: Option<u32>) -> Result<(), TransactionError> {
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|source| {
            TransactionError::Io {
                path: path.to_path_buf(),
                source,
            }
        })?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}
