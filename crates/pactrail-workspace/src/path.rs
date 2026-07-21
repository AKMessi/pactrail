use std::path::{Component, Path, PathBuf};

use thiserror::Error;

/// A non-empty relative path without traversal or platform prefixes.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SafeRelativePath(PathBuf);

impl SafeRelativePath {
    /// Validates a path for use below a workspace root.
    ///
    /// # Errors
    ///
    /// Returns [`PathError`] for empty, absolute, prefixed, or traversing paths.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, PathError> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Err(PathError::Empty);
        }
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                Component::Normal(value) => normalized.push(value),
                Component::CurDir => {}
                Component::ParentDir => return Err(PathError::Traversal(path.to_path_buf())),
                Component::RootDir | Component::Prefix(_) => {
                    return Err(PathError::Absolute(path.to_path_buf()));
                }
            }
        }
        if normalized.as_os_str().is_empty() {
            return Err(PathError::Empty);
        }
        Ok(Self(normalized))
    }

    /// Returns the normalized path.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// Returns a portable forward-slash representation.
    #[must_use]
    pub fn portable(&self) -> String {
        self.0
            .components()
            .filter_map(|component| match component {
                Component::Normal(value) => Some(value.to_string_lossy()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/")
    }
}

/// A path cannot safely be resolved below a workspace.
#[derive(Debug, Error)]
pub enum PathError {
    #[error("workspace-relative path cannot be empty")]
    Empty,
    #[error("absolute or prefixed path is forbidden: {0}")]
    Absolute(PathBuf),
    #[error("parent traversal is forbidden: {0}")]
    Traversal(PathBuf),
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn normalizes_current_directory_segments() {
        let path = SafeRelativePath::new("src/./lib.rs")
            .unwrap_or_else(|error| unreachable!("safe path: {error}"));
        assert_eq!(path.portable(), "src/lib.rs");
    }

    #[test]
    fn rejects_parent_traversal() {
        assert!(matches!(
            SafeRelativePath::new("src/../../secret"),
            Err(PathError::Traversal(_))
        ));
    }

    proptest! {
        #[test]
        fn accepted_components_round_trip_portably(
            components in prop::collection::vec("[A-Za-z0-9_-]{1,12}", 1..16)
        ) {
            let expected = components.join("/");
            let decorated = format!("./{}", components.join("/./"));
            let actual = SafeRelativePath::new(&decorated).ok().map(|path| path.portable());
            prop_assert_eq!(actual, Some(expected));
        }

        #[test]
        fn traversal_is_rejected_at_every_component_boundary(
            prefix in prop::collection::vec("[A-Za-z0-9_-]{1,12}", 0..8),
            suffix in prop::collection::vec("[A-Za-z0-9_-]{1,12}", 0..8)
        ) {
            let mut components = prefix;
            components.push("..".to_owned());
            components.extend(suffix);
            let hostile = components.join("/");
            prop_assert!(matches!(
                SafeRelativePath::new(hostile),
                Err(PathError::Traversal(_))
            ));
        }
    }
}
