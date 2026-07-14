use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;

use thiserror::Error;

const MAX_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_COMPRESSED_BYTES: u64 = 128 * 1024 * 1024;

/// Metadata for content persisted by an [`ArtifactStore`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredArtifact {
    pub digest: String,
    pub uncompressed_bytes: u64,
    pub compressed_bytes: u64,
}

/// Content-addressed, compressed artifact storage.
#[derive(Clone, Debug)]
pub struct ArtifactStore {
    root: PathBuf,
}

impl ArtifactStore {
    /// Opens or creates an artifact store below `root`.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the artifact directory cannot be created.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, ArtifactError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|source| ArtifactError::Io {
            path: root.clone(),
            source,
        })?;
        Ok(Self { root })
    }

    /// Stores bytes once under their BLAKE3 digest.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when compression or atomic persistence fails.
    pub fn put(&self, content: &[u8]) -> Result<StoredArtifact, ArtifactError> {
        let uncompressed_bytes = u64::try_from(content.len()).unwrap_or(u64::MAX);
        if uncompressed_bytes > MAX_ARTIFACT_BYTES {
            return Err(ArtifactError::TooLarge {
                actual: uncompressed_bytes,
                limit: MAX_ARTIFACT_BYTES,
            });
        }
        let digest = blake3::hash(content).to_hex().to_string();
        let destination = self.path_for(&digest)?;
        if destination.exists() {
            self.get(&digest)?;
        } else {
            let parent = destination
                .parent()
                .ok_or_else(|| ArtifactError::InvalidDigest(digest.clone()))?;
            fs::create_dir_all(parent).map_err(|source| ArtifactError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
            let mut temporary =
                tempfile::NamedTempFile::new_in(parent).map_err(|source| ArtifactError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            {
                let mut encoder = zstd::stream::write::Encoder::new(temporary.as_file_mut(), 3)
                    .map_err(|source| ArtifactError::Io {
                        path: destination.clone(),
                        source,
                    })?;
                encoder
                    .write_all(content)
                    .and_then(|()| encoder.finish().map(|_| ()))
                    .map_err(|source| ArtifactError::Io {
                        path: destination.clone(),
                        source,
                    })?;
            }
            temporary
                .as_file()
                .sync_all()
                .map_err(|source| ArtifactError::Io {
                    path: destination.clone(),
                    source,
                })?;
            match temporary.persist_noclobber(&destination) {
                Ok(_) => {}
                Err(_error) if destination.exists() => {}
                Err(error) => {
                    return Err(ArtifactError::Io {
                        path: destination,
                        source: error.error,
                    });
                }
            }
        }
        let compressed_bytes = fs::metadata(&destination)
            .map_err(|source| ArtifactError::Io {
                path: destination,
                source,
            })?
            .len();
        Ok(StoredArtifact {
            digest,
            uncompressed_bytes,
            compressed_bytes,
        })
    }

    /// Loads and integrity-checks one artifact.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid digest, failed I/O, decompression failure,
    /// or content that no longer matches its address.
    pub fn get(&self, digest: &str) -> Result<Vec<u8>, ArtifactError> {
        let path = self.path_for(digest)?;
        let compressed_bytes = fs::metadata(&path)
            .map_err(|source| ArtifactError::Io {
                path: path.clone(),
                source,
            })?
            .len();
        if compressed_bytes > MAX_COMPRESSED_BYTES {
            return Err(ArtifactError::TooLarge {
                actual: compressed_bytes,
                limit: MAX_COMPRESSED_BYTES,
            });
        }
        let file = File::open(&path).map_err(|source| ArtifactError::Io {
            path: path.clone(),
            source,
        })?;
        let decoder =
            zstd::stream::read::Decoder::new(file).map_err(|source| ArtifactError::Io {
                path: path.clone(),
                source,
            })?;
        let mut content = Vec::new();
        decoder
            .take(MAX_ARTIFACT_BYTES + 1)
            .read_to_end(&mut content)
            .map_err(|source| ArtifactError::Io {
                path: path.clone(),
                source,
            })?;
        let uncompressed_bytes = u64::try_from(content.len()).unwrap_or(u64::MAX);
        if uncompressed_bytes > MAX_ARTIFACT_BYTES {
            return Err(ArtifactError::TooLarge {
                actual: uncompressed_bytes,
                limit: MAX_ARTIFACT_BYTES,
            });
        }
        let actual = blake3::hash(&content).to_hex().to_string();
        if actual != digest {
            return Err(ArtifactError::Integrity {
                expected: digest.to_owned(),
                actual,
            });
        }
        Ok(content)
    }

    /// Returns whether an artifact exists without decoding it.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactError::InvalidDigest`] when `digest` is not a BLAKE3 hex digest.
    pub fn contains(&self, digest: &str) -> Result<bool, ArtifactError> {
        self.path_for(digest).map(|path| path.is_file())
    }

    fn path_for(&self, digest: &str) -> Result<PathBuf, ArtifactError> {
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ArtifactError::InvalidDigest(digest.to_owned()));
        }
        Ok(self.root.join(&digest[..2]).join(format!("{digest}.zst")))
    }
}

/// Artifact persistence or integrity failure.
#[derive(Debug, Error)]
pub enum ArtifactError {
    #[error("invalid BLAKE3 digest {0:?}")]
    InvalidDigest(String),
    #[error("artifact is {actual} bytes, exceeding the {limit}-byte limit")]
    TooLarge { actual: u64, limit: u64 },
    #[error("artifact I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("artifact integrity failed: expected {expected}, got {actual}")]
    Integrity { expected: String, actual: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifacts_round_trip_and_deduplicate() {
        let directory = tempfile::tempdir()
            .unwrap_or_else(|error| unreachable!("temporary directory: {error}"));
        let store = ArtifactStore::open(directory.path())
            .unwrap_or_else(|error| unreachable!("artifact store: {error}"));
        let first = store
            .put(b"important evidence")
            .unwrap_or_else(|error| unreachable!("store artifact: {error}"));
        let second = store
            .put(b"important evidence")
            .unwrap_or_else(|error| unreachable!("store artifact again: {error}"));
        assert_eq!(first.digest, second.digest);
        assert_eq!(
            store.get(&first.digest).ok(),
            Some(b"important evidence".to_vec())
        );
    }

    #[test]
    fn rejects_path_like_digests() {
        let directory = tempfile::tempdir()
            .unwrap_or_else(|error| unreachable!("temporary directory: {error}"));
        let store = ArtifactStore::open(directory.path())
            .unwrap_or_else(|error| unreachable!("artifact store: {error}"));
        assert!(matches!(
            store.get("../secret"),
            Err(ArtifactError::InvalidDigest(_))
        ));
        assert!(matches!(
            store.contains(&"A".repeat(64)),
            Err(ArtifactError::InvalidDigest(_))
        ));
    }

    #[test]
    fn existing_corrupt_artifact_is_never_reported_as_stored() {
        let directory = tempfile::tempdir()
            .unwrap_or_else(|error| unreachable!("temporary directory: {error}"));
        let store = ArtifactStore::open(directory.path())
            .unwrap_or_else(|error| unreachable!("artifact store: {error}"));
        let artifact = store
            .put(b"durable evidence")
            .unwrap_or_else(|error| unreachable!("initial artifact: {error}"));
        let path = store
            .path_for(&artifact.digest)
            .unwrap_or_else(|error| unreachable!("artifact path: {error}"));
        fs::write(&path, b"corrupt")
            .unwrap_or_else(|error| unreachable!("corrupt fixture: {error}"));

        assert!(store.put(b"durable evidence").is_err());
    }
}
