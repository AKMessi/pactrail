//! Workspace-scoped, explicit composer persistence and completion matching.
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;

const MAX_BYTES: usize = 64 * 1024;

pub(crate) struct ComposerStore {
    directory: PathBuf,
    workspace_key: String,
}

impl ComposerStore {
    pub(crate) fn new(config: &Path, workspace: &Path) -> Self {
        Self {
            directory: config.join("composer"),
            workspace_key: blake3::hash(workspace.as_os_str().as_encoded_bytes())
                .to_hex()
                .to_string(),
        }
    }

    fn path(&self, kind: &str) -> PathBuf {
        self.directory
            .join(format!("{}-{kind}.txt", self.workspace_key))
    }

    pub(crate) fn load(&self, kind: &str) -> io::Result<Option<String>> {
        let path = self.path(kind);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if !metadata.is_file() || metadata.len() > MAX_BYTES as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "saved composer text is not a bounded regular file",
            ));
        }
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "saved composer text is too large",
            ));
        }
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    pub(crate) fn save(&self, kind: &str, text: &str) -> io::Result<()> {
        if text.len() > MAX_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "composer text exceeds 64 KiB",
            ));
        }
        fs::create_dir_all(&self.directory)?;
        if !fs::symlink_metadata(&self.directory)?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "composer directory must not be a symbolic link",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.directory, fs::Permissions::from_mode(0o700))?;
        }
        let mut file = NamedTempFile::new_in(&self.directory)?;
        file.write_all(text.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(self.path(kind)).map_err(|e| e.error)?;
        Ok(())
    }

    pub(crate) fn clear(&self, kind: &str) -> io::Result<()> {
        match fs::remove_file(self.path(kind)) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
}

/// Prefixes win; ordered character matches allow compact command/goal queries.
/// Never applied to ordinary task input, only explicit completion surfaces.
pub(crate) fn match_score(candidate: &str, query: &str) -> Option<usize> {
    let candidate = candidate.to_lowercase();
    let query = query.to_lowercase();
    if candidate.starts_with(&query) {
        return Some(0);
    }
    if let Some(offset) = candidate.find(&query) {
        return Some(1 + offset);
    }
    let mut remaining = candidate.chars().enumerate();
    let mut score = 100;
    for wanted in query.chars() {
        let (index, _) = remaining.find(|(_, value)| *value == wanted)?;
        score += index;
    }
    Some(score)
}

/// One directory, bounded work, no content reads or recursive workspace scan.
pub(crate) fn path_completions(workspace: &Path, argument: &str) -> Vec<(String, bool)> {
    let parsed = shell_words::split(argument).ok();
    if parsed.as_ref().is_some_and(|words| words.len() > 1) {
        return Vec::new();
    }
    let prefix = parsed
        .as_ref()
        .and_then(|words| words.first())
        .map_or_else(|| argument.trim_start_matches(['\'', '"']), String::as_str);
    let (parent, leaf) = prefix
        .rsplit_once('/')
        .map_or(("", prefix), |(parent, leaf)| (parent, leaf));
    let directory = if prefix.starts_with('/') {
        PathBuf::from(format!("{parent}/"))
    } else {
        workspace.join(parent)
    };
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut matches = Vec::new();
    for entry in entries.take(2048).flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_file() && !kind.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.starts_with(leaf) || (name.starts_with('.') && !leaf.starts_with('.')) {
            continue;
        }
        let path = if parent.is_empty() && !prefix.starts_with('/') {
            name
        } else {
            format!("{parent}/{name}")
        };
        let path = if kind.is_dir() {
            format!("{path}/")
        } else {
            path
        };
        // Reject terminal controls instead of displaying a different pathname.
        if crate::output::sanitize_terminal_text(&path) != path {
            continue;
        }
        matches.push((shell_words::quote(&path).into_owned(), kind.is_dir()));
    }
    matches.sort();
    matches.truncate(100);
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_atomic_unicode_drafts_and_retry_survive_restart() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let first = ComposerStore::new(directory.path(), Path::new("/first"));
        let second = ComposerStore::new(directory.path(), Path::new("/second"));
        first.save("draft", "Read 日本語.rs\nThen explain.")?;
        first.save("last-task", "Prior task")?;
        let restarted = ComposerStore::new(directory.path(), Path::new("/first"));
        assert_eq!(
            restarted.load("draft")?.as_deref(),
            Some("Read 日本語.rs\nThen explain.")
        );
        assert_eq!(restarted.load("last-task")?.as_deref(), Some("Prior task"));
        assert_eq!(second.load("draft")?, None);
        first.save("draft", "Replacement")?;
        assert_eq!(first.load("draft")?.as_deref(), Some("Replacement"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(first.path("draft"))?.permissions().mode() & 0o777,
                0o600
            );
        }
        first.clear("draft")?;
        first.clear("draft")?;
        assert_eq!(first.load("draft")?, None);
        assert!(first.save("draft", &"x".repeat(MAX_BYTES + 1)).is_err());
        fs::write(first.path("draft"), [0xff])?;
        assert!(first.load("draft").is_err());
        Ok(())
    }

    #[test]
    fn path_completion_quotes_spaces_and_preserves_unicode() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        fs::write(directory.path().join("task with spaces.md"), "private body")?;
        fs::write(directory.path().join("日本語.md"), "body")?;
        fs::create_dir(directory.path().join("tasks"))?;
        let matches = path_completions(directory.path(), "task");
        assert_eq!(matches.len(), 2);
        for (value, _) in &matches {
            assert_eq!(
                shell_words::split(value).map_err(io::Error::other)?.len(),
                1
            );
        }
        assert_eq!(path_completions(directory.path(), "日本")[0].0, "日本語.md");
        assert!(path_completions(directory.path(), "a b").is_empty());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn saved_text_and_directory_symlinks_are_rejected() -> io::Result<()> {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir()?;
        let store = ComposerStore::new(directory.path(), Path::new("/workspace"));
        store.save("draft", "draft")?;
        store.clear("draft")?;
        let outside = directory.path().join("outside");
        fs::write(&outside, "outside")?;
        symlink(&outside, store.path("draft"))?;
        assert!(store.load("draft").is_err());
        // Atomic replacement doesn't write through a preexisting file symlink.
        store.save("draft", "replacement")?;
        assert_eq!(fs::read_to_string(&outside)?, "outside");
        fs::remove_dir_all(&store.directory)?;
        let target = directory.path().join("target");
        fs::create_dir(&target)?;
        symlink(&target, &store.directory)?;
        assert!(store.save("draft", "denied").is_err());
        Ok(())
    }

    #[test]
    fn matching_is_case_insensitive_and_ranked() {
        assert_eq!(match_score("/evidence", "/ev"), Some(0));
        assert!(match_score("/evidence", "/evd").is_some());
        assert!(match_score("Awaiting review · Fix 日本語 parser", "日本語").is_some());
        assert!(match_score("fixture-edit", "EDIT").is_some());
        assert_eq!(match_score("/apply", "discard"), None);
    }
}
