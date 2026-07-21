//! Bounded, process-free Git evidence for Pactrail.
//!
//! This crate enables none of Gitoxide's command, network-client, credential,
//! status, or filter-pipeline features. Its private implementation exposes and
//! calls only repository-open, object, index, and revision-read operations.
//! Repository data is opened with isolated configuration permissions and every
//! worktree path is validated through Pactrail's workspace path boundary before
//! it is read.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use gix::bstr::{BStr, BString, ByteSlice, ByteVec};
use ignore::WalkBuilder;
use pactrail_workspace::SafeRelativePath;
use serde::{Deserialize, Serialize};
use similar::TextDiff;
use thiserror::Error;

const MAX_OBJECT_BYTES: usize = 16 * 1024 * 1024;
const MAX_HEAD_ENTRIES: usize = 200_000;
const MAX_INDEX_ENTRIES: usize = 200_000;
const MAX_WORKTREE_ENTRIES: usize = 200_000;
const MAX_STATUS_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_STATUS_HASH_BYTES: u64 = 256 * 1024 * 1024;
const MAX_DIFF_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_DIFF_FILES: usize = 64;
const MAX_DIFF_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_HISTORY_COMMITS: usize = 100;
const MAX_COMMIT_SUMMARY_BYTES: usize = 512;
const MAX_AUTHOR_BYTES: usize = 256;

/// One side of a Git status entry.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GitChangeKind {
    Added,
    Modified,
    Deleted,
    TypeChanged,
    Conflicted,
    Unscanned,
}

/// Bounded status for one repository-relative path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GitPathStatus {
    pub path: String,
    pub staged: Option<GitChangeKind>,
    pub worktree: Option<GitChangeKind>,
    pub detail: Option<String>,
}

/// Resource-use and completeness telemetry for a status scan.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct GitScanTelemetry {
    pub head_entries: usize,
    pub index_entries: usize,
    pub worktree_entries: usize,
    pub hashed_files: usize,
    pub hashed_bytes: u64,
    pub unscanned_files: usize,
    pub traversal_truncated: bool,
}

/// Process-free repository status evidence.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GitStatus {
    pub branch: Option<String>,
    pub detached: bool,
    pub unborn: bool,
    pub head: Option<String>,
    pub entries: Vec<GitPathStatus>,
    pub total_entries: usize,
    pub result_truncated: bool,
    pub comparison: String,
    pub telemetry: GitScanTelemetry,
    pub warnings: Vec<String>,
}

/// One bounded commit-history record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GitCommitRecord {
    pub id: String,
    pub parent_ids: Vec<String>,
    pub author: String,
    pub committed_at_unix: i64,
    pub summary: String,
}

/// Bounded history rooted at the current `HEAD`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GitHistory {
    pub commits: Vec<GitCommitRecord>,
    pub requested: usize,
    pub result_truncated: bool,
    pub shallow_repository: bool,
}

/// A bounded raw HEAD-to-worktree diff.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GitDiff {
    pub patch: String,
    pub files: usize,
    pub total_changed_files: usize,
    pub binary_files: Vec<String>,
    pub omitted_files: Vec<String>,
    pub result_truncated: bool,
    pub comparison: String,
}

/// Read-only handle to a Git repository whose worktree is exactly the Pactrail
/// source workspace.
pub struct GitInspector {
    root: PathBuf,
    repository: gix::Repository,
}

#[derive(Clone)]
struct HeadEntry {
    id: gix::ObjectId,
    mode: gix::object::tree::EntryMode,
}

#[derive(Clone)]
struct IndexEntry {
    id: gix::ObjectId,
    mode: Option<gix::object::tree::EntryMode>,
    stage: u32,
}

struct BoundedHeadVisitor {
    entries: Vec<(BString, gix::ObjectId, gix::object::tree::EntryMode)>,
    limit: usize,
    path: BString,
    path_deque: VecDeque<BString>,
    truncated: bool,
}

impl BoundedHeadVisitor {
    fn new(limit: usize) -> Self {
        Self {
            entries: Vec::with_capacity(limit.min(4_096)),
            limit,
            path: BString::default(),
            path_deque: VecDeque::new(),
            truncated: false,
        }
    }

    fn push_element(&mut self, component: &BStr) {
        if component.is_empty() {
            return;
        }
        if !self.path.is_empty() {
            self.path.push(b'/');
        }
        self.path.push_str(component);
    }

    fn pop_element(&mut self) {
        if let Some(position) = self.path.rfind_byte(b'/') {
            self.path.resize(position, 0);
        } else {
            self.path.clear();
        }
    }
}

impl gix::traverse::tree::Visit for BoundedHeadVisitor {
    fn pop_back_tracked_path_and_set_current(&mut self) {
        self.path = self.path_deque.pop_back().unwrap_or_default();
    }

    fn pop_front_tracked_path_and_set_current(&mut self) {
        self.path = self.path_deque.pop_front().unwrap_or_default();
    }

    fn push_back_tracked_path_component(&mut self, component: &BStr) {
        self.push_element(component);
        self.path_deque.push_back(self.path.clone());
    }

    fn push_path_component(&mut self, component: &BStr) {
        self.push_element(component);
    }

    fn pop_path_component(&mut self) {
        self.pop_element();
    }

    fn visit_tree(
        &mut self,
        _entry: &gix::objs::tree::EntryRef<'_>,
    ) -> gix::traverse::tree::visit::Action {
        if self.entries.len() >= self.limit {
            self.truncated = true;
            std::ops::ControlFlow::Break(())
        } else {
            std::ops::ControlFlow::Continue(true)
        }
    }

    fn visit_nontree(
        &mut self,
        entry: &gix::objs::tree::EntryRef<'_>,
    ) -> gix::traverse::tree::visit::Action {
        if self.entries.len() >= self.limit {
            self.truncated = true;
            return std::ops::ControlFlow::Break(());
        }
        self.entries
            .push((self.path.clone(), entry.oid.to_owned(), entry.mode));
        std::ops::ControlFlow::Continue(true)
    }
}

#[derive(Default)]
struct PendingStatus {
    staged: Option<GitChangeKind>,
    worktree: Option<GitChangeKind>,
    details: BTreeSet<String>,
}

impl GitInspector {
    /// Opens the Git repository rooted exactly at `workspace_root`.
    ///
    /// Parent discovery is intentionally disabled: a tool authorized for the
    /// Pactrail workspace must not silently read sibling paths in a parent
    /// monorepo. The private implementation exposes and invokes no Git command,
    /// remote, credential, hook, or filter operation.
    ///
    /// # Errors
    ///
    /// Returns an error when the workspace is not a repository root, is not
    /// fully trusted by the operating-system ownership check, or has malformed
    /// repository configuration.
    pub fn open(workspace_root: impl AsRef<Path>) -> Result<Self, GitError> {
        let root = fs::canonicalize(workspace_root.as_ref()).map_err(|source| GitError::Io {
            path: workspace_root.as_ref().to_path_buf(),
            source,
        })?;
        if !root.is_dir() || !root.join(".git").exists() {
            return Err(GitError::NotRepositoryRoot(root));
        }
        let git_directory = root.join(".git");
        let git_metadata = fs::symlink_metadata(&git_directory).map_err(|source| GitError::Io {
            path: git_directory.clone(),
            source,
        })?;
        if !git_metadata.file_type().is_dir() {
            return Err(GitError::ExternalGitDirectory(git_directory));
        }
        validate_git_metadata_boundary(&git_directory)?;
        let options = gix::open::Options::isolated()
            .strict_config(true)
            .bail_if_untrusted(true)
            .config_overrides([format!("gitoxide.objects.allocLimit={MAX_OBJECT_BYTES}")]);
        let repository = gix::open_opts(&root, options)
            .map_err(|error| GitError::Repository(error.to_string()))?;
        let workdir = repository
            .workdir()
            .ok_or_else(|| GitError::BareRepository(root.clone()))?;
        let canonical_workdir = fs::canonicalize(workdir).map_err(|source| GitError::Io {
            path: workdir.to_path_buf(),
            source,
        })?;
        if canonical_workdir != root {
            return Err(GitError::MismatchedWorktree {
                expected: root,
                actual: canonical_workdir,
            });
        }
        Ok(Self { root, repository })
    }

    /// Returns bounded HEAD/index/raw-worktree status.
    ///
    /// The raw worktree comparison deliberately does not run clean filters,
    /// textconv, hooks, file-system monitors, or submodule commands. Files over
    /// the scan budgets are retained as explicit `unscanned` evidence.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed Git objects or index entries, unsafe
    /// repository paths, or filesystem failures.
    pub fn status(&self, max_entries: usize) -> Result<GitStatus, GitError> {
        if max_entries == 0 {
            return Err(GitError::InvalidLimit("max_entries must be positive"));
        }
        let mut telemetry = GitScanTelemetry::default();
        let (branch, detached, unborn, head) = self.head_identity()?;
        let head_entries = self.head_entries(unborn, &mut telemetry)?;
        let index_entries = self.index_entries(&mut telemetry)?;
        let mut pending = BTreeMap::<String, PendingStatus>::new();

        let mut index_stage_zero = BTreeMap::<String, IndexEntry>::new();
        let mut tracked_paths = BTreeSet::<String>::new();
        for (path, entries) in &index_entries {
            tracked_paths.insert(path.clone());
            if entries.iter().any(|entry| entry.stage != 0) {
                let entry = pending.entry(path.clone()).or_default();
                entry.staged = Some(GitChangeKind::Conflicted);
                entry.worktree = Some(GitChangeKind::Conflicted);
                entry
                    .details
                    .insert("index contains unresolved stages".to_owned());
            }
            if let Some(entry) = entries.iter().find(|entry| entry.stage == 0) {
                index_stage_zero.insert(path.clone(), entry.clone());
            }
        }

        let staged_paths = head_entries
            .keys()
            .chain(index_stage_zero.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        for path in staged_paths {
            if pending
                .get(&path)
                .is_some_and(|entry| entry.staged == Some(GitChangeKind::Conflicted))
            {
                continue;
            }
            let kind = match (head_entries.get(&path), index_stage_zero.get(&path)) {
                (None, Some(_)) => Some(GitChangeKind::Added),
                (Some(_), None) => Some(GitChangeKind::Deleted),
                (Some(before), Some(after)) if Some(before.mode) != after.mode => {
                    Some(GitChangeKind::TypeChanged)
                }
                (Some(before), Some(after)) if before.id != after.id => {
                    Some(GitChangeKind::Modified)
                }
                _ => None,
            };
            if let Some(kind) = kind {
                pending.entry(path).or_default().staged = Some(kind);
            }
        }

        let mut hash_budget_remaining = MAX_STATUS_HASH_BYTES;
        for (path, entry) in &index_stage_zero {
            let status =
                self.raw_worktree_status(path, entry, &mut hash_budget_remaining, &mut telemetry)?;
            if let Some((kind, detail)) = status {
                let pending = pending.entry(path.clone()).or_default();
                pending.worktree = Some(kind);
                if let Some(detail) = detail {
                    pending.details.insert(detail);
                }
            }
        }
        self.add_untracked(&tracked_paths, &mut pending, &mut telemetry)?;

        let total_entries = pending.len();
        let result_truncated = total_entries > max_entries || telemetry.traversal_truncated;
        let entries = pending
            .into_iter()
            .take(max_entries)
            .map(|(path, pending)| GitPathStatus {
                path,
                staged: pending.staged,
                worktree: pending.worktree,
                detail: (!pending.details.is_empty())
                    .then(|| pending.details.into_iter().collect::<Vec<_>>().join("; ")),
            })
            .collect();
        let mut warnings = vec![
            "Worktree comparisons hash raw bytes and never execute Git filters, textconv, hooks, filesystem monitors, or submodule commands.".to_owned(),
        ];
        if telemetry.unscanned_files > 0 || telemetry.traversal_truncated {
            warnings.push(
                "At least one path was inconclusive because a hard file, byte, or traversal budget was reached."
                    .to_owned(),
            );
        }
        Ok(GitStatus {
            branch,
            detached,
            unborn,
            head,
            entries,
            total_entries,
            result_truncated,
            comparison: "head_to_index_and_index_to_raw_worktree".to_owned(),
            telemetry,
            warnings,
        })
    }

    /// Returns bounded, newest-first history reachable from `HEAD`.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid limit or malformed commit data.
    pub fn history(&self, max_commits: usize) -> Result<GitHistory, GitError> {
        if max_commits == 0 || max_commits > MAX_HISTORY_COMMITS {
            return Err(GitError::InvalidLimit(
                "max_commits must be between 1 and 100",
            ));
        }
        let head = self.repository.head().map_err(git_repository_error)?;
        if head.is_unborn() {
            return Ok(GitHistory {
                commits: Vec::new(),
                requested: max_commits,
                result_truncated: false,
                shallow_repository: self.repository.is_shallow(),
            });
        }
        let head_id = self.repository.head_id().map_err(git_repository_error)?;
        let mut walk = self
            .repository
            .rev_walk([head_id])
            .sorting(gix::revision::walk::Sorting::ByCommitTime(
                gix::traverse::commit::simple::CommitTimeOrder::NewestFirst,
            ))
            .use_commit_graph(false)
            .all()
            .map_err(git_repository_error)?;
        let mut commits = Vec::with_capacity(max_commits);
        let mut has_more = false;
        for item in walk.by_ref().take(max_commits.saturating_add(1)) {
            let info = item.map_err(git_repository_error)?;
            if commits.len() == max_commits {
                has_more = true;
                break;
            }
            let commit = info.object().map_err(git_repository_error)?;
            let author = commit.author().map_err(git_repository_error)?;
            let time = commit.time().map_err(git_repository_error)?;
            let summary = commit
                .message_raw()
                .map_err(git_repository_error)?
                .lines()
                .next()
                .unwrap_or_default()
                .to_str_lossy();
            commits.push(GitCommitRecord {
                id: info.id.to_string(),
                parent_ids: info.parent_ids().map(|id| id.to_string()).collect(),
                author: truncate_utf8(&author.name.to_str_lossy(), MAX_AUTHOR_BYTES),
                committed_at_unix: time.seconds,
                summary: truncate_utf8(&summary, MAX_COMMIT_SUMMARY_BYTES),
            });
        }
        Ok(GitHistory {
            commits,
            requested: max_commits,
            result_truncated: has_more,
            shallow_repository: self.repository.is_shallow(),
        })
    }

    /// Returns a combined raw HEAD-to-source-worktree unified diff.
    ///
    /// `path` is workspace-relative. The result is a navigation artifact, not a
    /// replacement for Git's filter-aware command-line diff.
    ///
    /// # Errors
    ///
    /// Returns an error for unsafe paths, malformed Git objects, or filesystem
    /// failures.
    pub fn diff(&self, path: Option<&str>) -> Result<GitDiff, GitError> {
        let filter = path.map(SafeRelativePath::new).transpose()?;
        let status = self.status(MAX_HEAD_ENTRIES)?;
        let unborn = self
            .repository
            .head()
            .map_err(git_repository_error)?
            .is_unborn();
        let head_tree = (!unborn)
            .then(|| self.repository.head_tree().map_err(git_repository_error))
            .transpose()?;
        let mut rendered_diff = String::new();
        let mut binary_files = Vec::new();
        let mut omitted_files = Vec::new();
        let mut files = 0_usize;
        let mut examined_files = 0_usize;
        let mut total_changed_files = 0_usize;
        let mut result_truncated = status.result_truncated;

        for entry in status.entries.iter().filter(|entry| {
            filter
                .as_ref()
                .is_none_or(|value| value.portable() == entry.path)
        }) {
            if examined_files == MAX_DIFF_FILES {
                result_truncated = true;
                omitted_files.push(entry.path.clone());
                total_changed_files = total_changed_files.saturating_add(1);
                continue;
            }
            let before = match self.head_blob(head_tree.as_ref(), &entry.path) {
                Ok(value) => value,
                Err(GitError::DiffFileTooLarge(_)) => {
                    omitted_files.push(entry.path.clone());
                    result_truncated = true;
                    total_changed_files = total_changed_files.saturating_add(1);
                    examined_files = examined_files.saturating_add(1);
                    continue;
                }
                Err(error) => return Err(error),
            };
            let after = match read_optional_bounded(&self.root, &entry.path, MAX_DIFF_FILE_BYTES) {
                Ok(value) => value,
                Err(GitError::DiffFileTooLarge(_)) => {
                    omitted_files.push(entry.path.clone());
                    result_truncated = true;
                    total_changed_files = total_changed_files.saturating_add(1);
                    examined_files = examined_files.saturating_add(1);
                    continue;
                }
                Err(error) => return Err(error),
            };
            examined_files = examined_files.saturating_add(1);
            if before == after {
                continue;
            }
            total_changed_files = total_changed_files.saturating_add(1);
            if before.as_deref().is_some_and(is_binary) || after.as_deref().is_some_and(is_binary) {
                binary_files.push(entry.path.clone());
                continue;
            }
            let before_text = before
                .as_deref()
                .map(std::str::from_utf8)
                .transpose()
                .ok()
                .flatten();
            let after_text = after
                .as_deref()
                .map(std::str::from_utf8)
                .transpose()
                .ok()
                .flatten();
            if (before.is_some() && before_text.is_none())
                || (after.is_some() && after_text.is_none())
            {
                binary_files.push(entry.path.clone());
                continue;
            }
            append_unified_diff(&mut rendered_diff, &entry.path, before_text, after_text);
            files = files.saturating_add(1);
            if rendered_diff.len() > MAX_DIFF_OUTPUT_BYTES {
                result_truncated = true;
                truncate_with_marker(&mut rendered_diff, MAX_DIFF_OUTPUT_BYTES);
                break;
            }
        }
        if let Some(filter) = filter
            && total_changed_files == 0
        {
            return Err(GitError::PathNotChanged(filter.portable()));
        }
        Ok(GitDiff {
            patch: rendered_diff,
            files,
            total_changed_files,
            binary_files,
            omitted_files,
            result_truncated,
            comparison: "head_to_raw_source_worktree".to_owned(),
        })
    }

    fn head_identity(&self) -> Result<(Option<String>, bool, bool, Option<String>), GitError> {
        let head = self.repository.head().map_err(git_repository_error)?;
        let detached = head.is_detached();
        let unborn = head.is_unborn();
        let branch = head
            .referent_name()
            .map(|name| name.shorten().to_str_lossy().into_owned());
        let id = (!unborn)
            .then(|| self.repository.head_id().map(|id| id.to_string()))
            .transpose()
            .map_err(git_repository_error)?;
        Ok((branch, detached, unborn, id))
    }

    fn head_entries(
        &self,
        unborn: bool,
        telemetry: &mut GitScanTelemetry,
    ) -> Result<BTreeMap<String, HeadEntry>, GitError> {
        if unborn {
            return Ok(BTreeMap::new());
        }
        let tree = self.repository.head_tree().map_err(git_repository_error)?;
        let mut visitor = BoundedHeadVisitor::new(MAX_HEAD_ENTRIES);
        let traversal = tree.traverse().breadthfirst(&mut visitor);
        if let Err(error) = traversal
            && !visitor.truncated
        {
            return Err(git_repository_error(error));
        }
        telemetry.head_entries = visitor.entries.len();
        telemetry.traversal_truncated |= visitor.truncated;
        let mut output = BTreeMap::new();
        for (path, id, mode) in visitor.entries {
            let path = validated_git_path(path.as_bstr())?;
            output.insert(path, HeadEntry { id, mode });
        }
        Ok(output)
    }

    fn index_entries(
        &self,
        telemetry: &mut GitScanTelemetry,
    ) -> Result<BTreeMap<String, Vec<IndexEntry>>, GitError> {
        let index = self
            .repository
            .index_or_empty()
            .map_err(git_repository_error)?;
        if index.entries().len() > MAX_INDEX_ENTRIES {
            return Err(GitError::IndexTooLarge(index.entries().len()));
        }
        index.verify_integrity().map_err(git_repository_error)?;
        telemetry.index_entries = index.entries().len();
        let mut output = BTreeMap::<String, Vec<IndexEntry>>::new();
        for entry in index.entries() {
            let path = validated_git_path(entry.path(&index))?;
            output.entry(path).or_default().push(IndexEntry {
                id: entry.id,
                mode: entry.mode.to_tree_entry_mode(),
                stage: entry.stage_raw(),
            });
        }
        Ok(output)
    }

    fn raw_worktree_status(
        &self,
        path: &str,
        index: &IndexEntry,
        hash_budget_remaining: &mut u64,
        telemetry: &mut GitScanTelemetry,
    ) -> Result<Option<(GitChangeKind, Option<String>)>, GitError> {
        let safe = SafeRelativePath::new(path)?;
        let absolute = self.root.join(safe.as_path());
        let metadata = match fs::symlink_metadata(&absolute) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Some((GitChangeKind::Deleted, None)));
            }
            Err(source) => {
                return Err(GitError::Io {
                    path: absolute,
                    source,
                });
            }
        };
        let Some(index_mode) = index.mode else {
            telemetry.unscanned_files = telemetry.unscanned_files.saturating_add(1);
            return Ok(Some((
                GitChangeKind::Unscanned,
                Some("index entry has an unknown mode".to_owned()),
            )));
        };
        match index_mode.kind() {
            gix::object::tree::EntryKind::Commit => {
                telemetry.unscanned_files = telemetry.unscanned_files.saturating_add(1);
                return Ok(Some((
                    GitChangeKind::Unscanned,
                    Some("submodule worktree state is never executed or traversed".to_owned()),
                )));
            }
            gix::object::tree::EntryKind::Link => {
                telemetry.unscanned_files = telemetry.unscanned_files.saturating_add(1);
                return Ok(Some((
                    if metadata.file_type().is_symlink() {
                        GitChangeKind::Unscanned
                    } else {
                        GitChangeKind::TypeChanged
                    },
                    Some("symbolic-link targets are not followed or hashed".to_owned()),
                )));
            }
            gix::object::tree::EntryKind::Tree => {
                return Ok(Some((GitChangeKind::TypeChanged, None)));
            }
            gix::object::tree::EntryKind::Blob | gix::object::tree::EntryKind::BlobExecutable => {}
        }
        if !metadata.file_type().is_file() {
            return Ok(Some((GitChangeKind::TypeChanged, None)));
        }
        let header = self
            .repository
            .find_header(index.id)
            .map_err(git_repository_error)?;
        if metadata.len() != header.size() {
            return Ok(Some((GitChangeKind::Modified, None)));
        }
        if metadata.len() > MAX_STATUS_FILE_BYTES || metadata.len() > *hash_budget_remaining {
            telemetry.unscanned_files = telemetry.unscanned_files.saturating_add(1);
            return Ok(Some((
                GitChangeKind::Unscanned,
                Some("raw content hash exceeded the status scan budget".to_owned()),
            )));
        }
        let bytes = fs::read(&absolute).map_err(|source| GitError::Io {
            path: absolute,
            source,
        })?;
        *hash_budget_remaining = hash_budget_remaining.saturating_sub(metadata.len());
        telemetry.hashed_files = telemetry.hashed_files.saturating_add(1);
        telemetry.hashed_bytes = telemetry.hashed_bytes.saturating_add(metadata.len());
        let id =
            gix::objs::compute_hash(self.repository.object_hash(), gix::objs::Kind::Blob, &bytes)
                .map_err(git_repository_error)?;
        Ok((id != index.id).then_some((GitChangeKind::Modified, None)))
    }

    fn add_untracked(
        &self,
        tracked_paths: &BTreeSet<String>,
        pending: &mut BTreeMap<String, PendingStatus>,
        telemetry: &mut GitScanTelemetry,
    ) -> Result<(), GitError> {
        let walker = WalkBuilder::new(&self.root)
            .hidden(false)
            .git_ignore(true)
            .git_global(false)
            .git_exclude(true)
            .parents(false)
            .filter_entry(|entry| {
                let name = entry.file_name().to_string_lossy();
                name != ".git" && name != ".pactrail"
            })
            .build();
        for result in walker {
            let entry = result.map_err(|error| GitError::Walk(error.to_string()))?;
            if entry.path() == self.root {
                continue;
            }
            telemetry.worktree_entries = telemetry.worktree_entries.saturating_add(1);
            if telemetry.worktree_entries > MAX_WORKTREE_ENTRIES {
                telemetry.traversal_truncated = true;
                break;
            }
            let Some(file_type) = entry.file_type() else {
                return Err(GitError::UnsupportedPath(entry.path().to_path_buf()));
            };
            if file_type.is_dir() {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(&self.root)
                .map_err(|_| GitError::EscapedRoot(entry.path().to_path_buf()))?;
            let safe = SafeRelativePath::new(relative)?;
            if !tracked_paths.contains(&safe.portable()) {
                pending.entry(safe.portable()).or_default().worktree = Some(GitChangeKind::Added);
            }
        }
        Ok(())
    }

    fn head_blob(
        &self,
        tree: Option<&gix::Tree<'_>>,
        path: &str,
    ) -> Result<Option<Vec<u8>>, GitError> {
        let Some(tree) = tree else {
            return Ok(None);
        };
        let safe = SafeRelativePath::new(path)?;
        let Some(entry) = tree
            .lookup_entry_by_path(safe.as_path())
            .map_err(git_repository_error)?
        else {
            return Ok(None);
        };
        match entry.mode().kind() {
            gix::object::tree::EntryKind::Blob
            | gix::object::tree::EntryKind::BlobExecutable
            | gix::object::tree::EntryKind::Link => {
                let blob = self
                    .repository
                    .find_blob(entry.object_id())
                    .map_err(git_repository_error)?;
                if blob.data.len() > usize::try_from(MAX_DIFF_FILE_BYTES).unwrap_or(usize::MAX) {
                    return Err(GitError::DiffFileTooLarge(path.to_owned()));
                }
                Ok(Some(blob.data.clone()))
            }
            gix::object::tree::EntryKind::Commit | gix::object::tree::EntryKind::Tree => Ok(None),
        }
    }
}

fn read_optional_bounded(
    root: &Path,
    path: &str,
    max_bytes: u64,
) -> Result<Option<Vec<u8>>, GitError> {
    let safe = SafeRelativePath::new(path)?;
    let absolute = root.join(safe.as_path());
    let metadata = match fs::symlink_metadata(&absolute) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(GitError::Io {
                path: absolute,
                source,
            });
        }
    };
    if !metadata.file_type().is_file() {
        return Ok(None);
    }
    if metadata.len() > max_bytes {
        return Err(GitError::DiffFileTooLarge(path.to_owned()));
    }
    fs::read(&absolute)
        .map(Some)
        .map_err(|source| GitError::Io {
            path: absolute,
            source,
        })
}

fn validate_git_metadata_boundary(git_directory: &Path) -> Result<(), GitError> {
    for redirect in [
        git_directory.join("commondir"),
        git_directory.join("objects/info/alternates"),
        git_directory.join("objects/info/http-alternates"),
    ] {
        match fs::symlink_metadata(&redirect) {
            Ok(_) => return Err(GitError::RedirectedGitMetadata(redirect)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(GitError::Io {
                    path: redirect,
                    source,
                });
            }
        }
    }

    for path in [
        git_directory.to_path_buf(),
        git_directory.join("HEAD"),
        git_directory.join("config"),
        git_directory.join("index"),
        git_directory.join("packed-refs"),
        git_directory.join("shallow"),
        git_directory.join("refs"),
        git_directory.join("objects"),
        git_directory.join("objects/info"),
        git_directory.join("objects/pack"),
    ] {
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source) => return Err(GitError::Io { path, source }),
        };
        if metadata.file_type().is_symlink() {
            return Err(GitError::RedirectedGitMetadata(path));
        }
        let canonical = fs::canonicalize(&path).map_err(|source| GitError::Io {
            path: path.clone(),
            source,
        })?;
        if !canonical.starts_with(git_directory) {
            return Err(GitError::RedirectedGitMetadata(path));
        }
    }
    Ok(())
}

fn validated_git_path(path: &BStr) -> Result<String, GitError> {
    let path = path
        .to_str()
        .map_err(|_| GitError::NonUnicodeGitPath(path.to_str_lossy().into_owned()))?;
    let safe = SafeRelativePath::new(path)?;
    Ok(safe.portable())
}

fn append_unified_diff(output: &mut String, path: &str, before: Option<&str>, after: Option<&str>) {
    let before_exists = before.is_some();
    let after_exists = after.is_some();
    let before = before.unwrap_or_default();
    let after = after.unwrap_or_default();
    let before_label = if before_exists {
        format!("a/{path}")
    } else {
        "/dev/null".to_owned()
    };
    let after_label = if after_exists {
        format!("b/{path}")
    } else {
        "/dev/null".to_owned()
    };
    let rendered = TextDiff::from_lines(before, after)
        .unified_diff()
        .context_radius(3)
        .header(&before_label, &after_label)
        .to_string();
    if !rendered.is_empty() {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&rendered);
    }
}

fn truncate_with_marker(value: &mut String, max_bytes: usize) {
    const MARKER: &str = "\n... git diff truncated at the Pactrail output budget ...\n";
    let keep = max_bytes.saturating_sub(MARKER.len());
    let mut boundary = keep.min(value.len());
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary = boundary.saturating_sub(1);
    }
    value.truncate(boundary);
    value.push_str(MARKER);
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary = boundary.saturating_sub(1);
    }
    value[..boundary].to_owned()
}

fn is_binary(bytes: &[u8]) -> bool {
    bytes.contains(&0)
}

fn git_repository_error(error: impl std::fmt::Display) -> GitError {
    GitError::Repository(error.to_string())
}

/// Failure while producing bounded Git evidence.
#[derive(Debug, Error)]
pub enum GitError {
    #[error("workspace is not a Git repository root: {0}")]
    NotRepositoryRoot(PathBuf),
    #[error("Git metadata must be a real directory inside the workspace: {0}")]
    ExternalGitDirectory(PathBuf),
    #[error("redirected Git metadata is outside Pactrail's read boundary: {0}")]
    RedirectedGitMetadata(PathBuf),
    #[error("bare Git repositories are not valid Pactrail workspaces: {0}")]
    BareRepository(PathBuf),
    #[error("Git repository worktree mismatch: expected {expected}, found {actual}")]
    MismatchedWorktree { expected: PathBuf, actual: PathBuf },
    #[error("Git repository read failed: {0}")]
    Repository(String),
    #[error("Git index contains {0} entries, above the hard safety limit")]
    IndexTooLarge(usize),
    #[error("Git path is not valid Unicode: {0:?}")]
    NonUnicodeGitPath(String),
    #[error("Git path escaped the repository root: {0}")]
    EscapedRoot(PathBuf),
    #[error("Git path has an unsupported filesystem type: {0}")]
    UnsupportedPath(PathBuf),
    #[error("Git traversal failed: {0}")]
    Walk(String),
    #[error("diff file exceeds the {MAX_DIFF_FILE_BYTES}-byte per-file limit: {0}")]
    DiffFileTooLarge(String),
    #[error("requested Git diff path is unchanged: {0}")]
    PathNotChanged(String),
    #[error("invalid Git evidence limit: {0}")]
    InvalidLimit(&'static str),
    #[error("workspace path is unsafe: {0}")]
    UnsafePath(#[from] pactrail_workspace::PathError),
    #[error("I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    fn run_git(root: &Path, arguments: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
        let output = Command::new("git")
            .current_dir(root)
            .args(arguments)
            .output()?;
        if output.status.success() {
            return Ok(());
        }
        Err(format!(
            "git {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr)
        )
        .into())
    }

    fn git_output(root: &Path, arguments: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
        let output = Command::new("git")
            .current_dir(root)
            .args(arguments)
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "git {} failed: {}",
                arguments.join(" "),
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }

    fn committed_fixture() -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        run_git(root.path(), &["init", "--quiet"])?;
        fs::write(root.path().join("tracked.txt"), b"before\n")?;
        fs::write(root.path().join("empty.txt"), b"not empty\n")?;
        run_git(root.path(), &["add", "--", "tracked.txt", "empty.txt"])?;
        run_git(
            root.path(),
            &[
                "-c",
                "user.name=Pactrail Fixture",
                "-c",
                "user.email=pactrail-fixture@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "fixture baseline",
            ],
        )?;
        Ok(root)
    }

    #[test]
    fn reports_staged_worktree_and_untracked_evidence() -> Result<(), Box<dyn std::error::Error>> {
        let root = committed_fixture()?;
        fs::write(root.path().join("tracked.txt"), b"staged\n")?;
        run_git(root.path(), &["add", "--", "tracked.txt"])?;
        fs::write(root.path().join("tracked.txt"), b"worktree\n")?;
        fs::write(root.path().join("untracked.txt"), b"new\n")?;

        let status = GitInspector::open(root.path())?.status(20)?;
        let tracked = status
            .entries
            .iter()
            .find(|entry| entry.path == "tracked.txt")
            .ok_or("tracked status missing")?;
        assert_eq!(tracked.staged, Some(GitChangeKind::Modified));
        assert_eq!(tracked.worktree, Some(GitChangeKind::Modified));
        let untracked = status
            .entries
            .iter()
            .find(|entry| entry.path == "untracked.txt")
            .ok_or("untracked status missing")?;
        assert_eq!(untracked.worktree, Some(GitChangeKind::Added));
        assert!(!status.result_truncated);
        assert!(status.telemetry.hashed_files > 0);
        Ok(())
    }

    #[test]
    fn renders_history_and_bounded_raw_diff() -> Result<(), Box<dyn std::error::Error>> {
        let root = committed_fixture()?;
        fs::write(root.path().join("tracked.txt"), b"after\n")?;
        fs::write(root.path().join("empty.txt"), b"")?;

        let inspector = GitInspector::open(root.path())?;
        let history = inspector.history(10)?;
        assert_eq!(history.commits.len(), 1);
        assert_eq!(history.commits[0].summary, "fixture baseline");
        assert_eq!(history.commits[0].author, "Pactrail Fixture");

        let diff = inspector.diff(None)?;
        assert_eq!(diff.total_changed_files, 2);
        assert!(diff.patch.contains("-before"));
        assert!(diff.patch.contains("+after"));
        assert!(diff.patch.contains("+++ b/empty.txt"));
        assert!(!diff.patch.contains("+++ /dev/null"));
        Ok(())
    }

    #[test]
    fn requires_the_workspace_to_be_the_repository_root() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = committed_fixture()?;
        fs::create_dir(root.path().join("nested"))?;
        let result = GitInspector::open(root.path().join("nested"));
        assert!(matches!(result, Err(GitError::NotRepositoryRoot(_))));
        Ok(())
    }

    #[test]
    fn handles_unborn_repositories_and_result_limits() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        run_git(root.path(), &["init", "--quiet"])?;
        fs::write(root.path().join("one.txt"), b"one\n")?;
        fs::write(root.path().join("two.txt"), b"two\n")?;

        let inspector = GitInspector::open(root.path())?;
        let status = inspector.status(1)?;
        assert!(status.unborn);
        assert!(status.head.is_none());
        assert_eq!(status.total_entries, 2);
        assert_eq!(status.entries.len(), 1);
        assert!(status.result_truncated);
        assert!(inspector.history(10)?.commits.is_empty());
        Ok(())
    }

    #[test]
    fn staged_only_state_is_not_misreported_as_a_raw_diff() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = committed_fixture()?;
        fs::write(root.path().join("tracked.txt"), b"staged only\n")?;
        run_git(root.path(), &["add", "--", "tracked.txt"])?;
        fs::write(root.path().join("tracked.txt"), b"before\n")?;

        let inspector = GitInspector::open(root.path())?;
        let status = inspector.status(10)?;
        let tracked = status
            .entries
            .iter()
            .find(|entry| entry.path == "tracked.txt")
            .ok_or("tracked status missing")?;
        assert_eq!(tracked.staged, Some(GitChangeKind::Modified));
        assert_eq!(tracked.worktree, Some(GitChangeKind::Modified));
        let diff = inspector.diff(None)?;
        assert_eq!(diff.total_changed_files, 0);
        assert!(diff.patch.is_empty());
        assert!(matches!(
            inspector.diff(Some("tracked.txt")),
            Err(GitError::PathNotChanged(path)) if path == "tracked.txt"
        ));
        Ok(())
    }

    #[test]
    fn marks_binary_and_over_budget_files_without_embedding_them()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = committed_fixture()?;
        fs::write(root.path().join("binary.dat"), [0, 1, 2, 3])?;
        let diff = GitInspector::open(root.path())?.diff(None)?;
        assert_eq!(diff.binary_files, vec!["binary.dat"]);
        assert!(!diff.patch.contains("binary.dat"));

        let large_path = root.path().join("large.dat");
        let large = fs::File::create(&large_path)?;
        large.set_len(MAX_DIFF_FILE_BYTES.saturating_add(1))?;
        let diff = GitInspector::open(root.path())?.diff(None)?;
        assert!(diff.omitted_files.iter().any(|path| path == "large.dat"));
        assert!(diff.result_truncated);
        Ok(())
    }

    #[test]
    fn reports_unresolved_index_stages_as_conflicts() -> Result<(), Box<dyn std::error::Error>> {
        let root = committed_fixture()?;
        let original_branch = git_output(root.path(), &["branch", "--show-current"])?;
        run_git(root.path(), &["checkout", "--quiet", "-b", "conflicting"])?;
        fs::write(root.path().join("tracked.txt"), b"branch\n")?;
        run_git(root.path(), &["add", "--", "tracked.txt"])?;
        run_git(
            root.path(),
            &[
                "-c",
                "user.name=Pactrail Fixture",
                "-c",
                "user.email=pactrail-fixture@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "branch change",
            ],
        )?;
        run_git(root.path(), &["checkout", "--quiet", &original_branch])?;
        fs::write(root.path().join("tracked.txt"), b"main\n")?;
        run_git(root.path(), &["add", "--", "tracked.txt"])?;
        run_git(
            root.path(),
            &[
                "-c",
                "user.name=Pactrail Fixture",
                "-c",
                "user.email=pactrail-fixture@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "main change",
            ],
        )?;
        let merge = Command::new("git")
            .current_dir(root.path())
            .args([
                "-c",
                "user.name=Pactrail Fixture",
                "-c",
                "user.email=pactrail-fixture@example.invalid",
                "merge",
                "--no-edit",
                "conflicting",
            ])
            .output()?;
        assert!(!merge.status.success());

        let status = GitInspector::open(root.path())?.status(10)?;
        let conflict = status
            .entries
            .iter()
            .find(|entry| entry.path == "tracked.txt")
            .ok_or("conflict status missing")?;
        assert_eq!(conflict.staged, Some(GitChangeKind::Conflicted));
        assert_eq!(conflict.worktree, Some(GitChangeKind::Conflicted));
        Ok(())
    }

    #[test]
    fn rejects_unsafe_diff_paths_and_external_git_directories()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = committed_fixture()?;
        let inspector = GitInspector::open(root.path())?;
        assert!(matches!(
            inspector.diff(Some("../outside")),
            Err(GitError::UnsafePath(_))
        ));

        let linked = tempfile::tempdir()?;
        fs::write(linked.path().join(".git"), b"gitdir: ../outside\n")?;
        assert!(matches!(
            GitInspector::open(linked.path()),
            Err(GitError::ExternalGitDirectory(_))
        ));
        Ok(())
    }

    #[test]
    fn rejects_git_object_alternates_that_escape_the_workspace()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = committed_fixture()?;
        fs::write(
            root.path().join(".git/objects/info/alternates"),
            b"../../../../outside\n",
        )?;
        assert!(matches!(
            GitInspector::open(root.path()),
            Err(GitError::RedirectedGitMetadata(_))
        ));
        Ok(())
    }
}
