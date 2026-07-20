use async_trait::async_trait;
use pactrail_core::Capability;
use pactrail_git::GitInspector;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::builtins::{descriptor, input};
use crate::{Tool, ToolContext, ToolDescriptor, ToolError, ToolOutput};

const DEFAULT_STATUS_ENTRIES: usize = 200;
const MAX_STATUS_ENTRIES: usize = 512;
const DEFAULT_HISTORY_COMMITS: usize = 20;
const MAX_HISTORY_COMMITS: usize = 100;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GitStatusInput {
    /// Maximum repository and candidate entries returned. Aggregate counts
    /// remain available when either result is truncated.
    #[serde(default = "default_status_entries")]
    max_entries: usize,
}

const fn default_status_entries() -> usize {
    DEFAULT_STATUS_ENTRIES
}

/// Reads process-free source-repository and isolated-candidate status.
pub struct GitStatusTool;

#[async_trait]
impl Tool for GitStatusTool {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor::<GitStatusInput>(
            "git_status",
            "Inspect bounded read-only Git status for the source repository plus Pactrail's isolated candidate changes. Distinguishes HEAD-to-index from index-to-raw-worktree evidence and never runs Git commands, filters, hooks, submodules, credentials, or network operations.",
            Capability::FileRead,
        )
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        value: Value,
    ) -> Result<ToolOutput, ToolError> {
        let request: GitStatusInput = input(value, "git_status")?;
        if !(1..=MAX_STATUS_ENTRIES).contains(&request.max_entries) {
            return Err(ToolError::InvalidRange(format!(
                "max_entries must be between 1 and {MAX_STATUS_ENTRIES}"
            )));
        }
        context.authorize(&Capability::FileRead, ".", "git_status")?;

        let repository =
            GitInspector::open(context.workspace.source_root())?.status(request.max_entries)?;
        let candidate_changes = context.workspace.changes()?;
        let candidate_total = candidate_changes.len();
        let candidate_truncated = candidate_total > request.max_entries;
        let candidate_changes = candidate_changes
            .into_iter()
            .take(request.max_entries)
            .collect::<Vec<_>>();
        let truncated = repository.result_truncated || candidate_truncated;

        Ok(ToolOutput {
            content: json!({
                "source_repository": repository,
                "isolated_candidate": {
                    "baseline_digest": context.workspace.baseline_digest(),
                    "changes": candidate_changes,
                    "total_changes": candidate_total,
                    "result_truncated": candidate_truncated,
                    "warning": "candidate changes are isolated and do not modify the source repository until explicit review and apply"
                }
            }),
            summary: format!(
                "source has {} changed path(s); isolated candidate has {candidate_total} change(s)",
                repository.total_entries
            ),
            observed_effects: vec![
                "git.read:status".to_owned(),
                "transaction.inspect:changes".to_owned(),
            ],
            succeeded: true,
            truncated,
        })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GitDiffInput {
    /// Optional workspace-relative source path. Absolute paths and traversal
    /// components are rejected.
    path: Option<String>,
}

/// Reads a bounded raw HEAD-to-source-worktree diff.
pub struct GitDiffTool;

#[async_trait]
impl Tool for GitDiffTool {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor::<GitDiffInput>(
            "git_diff",
            "Render a bounded read-only unified diff from HEAD to the raw source worktree, optionally for one path. This is Git navigation evidence; use workspace_changes and mutation feedback for Pactrail's isolated candidate.",
            Capability::FileRead,
        )
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        value: Value,
    ) -> Result<ToolOutput, ToolError> {
        let request: GitDiffInput = input(value, "git_diff")?;
        context.authorize(&Capability::FileRead, ".", "git_diff")?;
        let result =
            GitInspector::open(context.workspace.source_root())?.diff(request.path.as_deref())?;
        let truncated = result.result_truncated;
        Ok(ToolOutput {
            summary: format!(
                "rendered {} of {} changed source file(s)",
                result.files, result.total_changed_files
            ),
            content: json!({
                "warning": "raw-byte source diff only; Git filters, textconv, hooks, submodules, external commands, and the isolated candidate are not evaluated",
                "result": result,
            }),
            observed_effects: vec!["git.read:diff".to_owned()],
            succeeded: true,
            truncated,
        })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GitHistoryInput {
    /// Maximum newest-first commits to return.
    #[serde(default = "default_history_commits")]
    max_commits: usize,
}

const fn default_history_commits() -> usize {
    DEFAULT_HISTORY_COMMITS
}

/// Reads bounded commit history rooted at `HEAD`.
pub struct GitHistoryTool;

#[async_trait]
impl Tool for GitHistoryTool {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor::<GitHistoryInput>(
            "git_history",
            "Read bounded newest-first commit history from HEAD without executing Git, contacting remotes, resolving credentials, or exposing author email addresses.",
            Capability::FileRead,
        )
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        value: Value,
    ) -> Result<ToolOutput, ToolError> {
        let request: GitHistoryInput = input(value, "git_history")?;
        if !(1..=MAX_HISTORY_COMMITS).contains(&request.max_commits) {
            return Err(ToolError::InvalidRange(format!(
                "max_commits must be between 1 and {MAX_HISTORY_COMMITS}"
            )));
        }
        context.authorize(&Capability::FileRead, ".", "git_history")?;
        let result =
            GitInspector::open(context.workspace.source_root())?.history(request.max_commits)?;
        let truncated = result.result_truncated;
        Ok(ToolOutput {
            summary: format!("read {} commit(s) from HEAD", result.commits.len()),
            content: json!({
                "privacy": "author names are retained; author email addresses are omitted",
                "result": result,
            }),
            observed_effects: vec!["git.read:history".to_owned()],
            succeeded: true,
            truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use pactrail_core::PermissionSet;
    use pactrail_workspace::WorkspaceTransaction;

    use super::*;
    use crate::PolicyEngine;

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

    fn fixture() -> Result<
        (tempfile::TempDir, tempfile::TempDir, WorkspaceTransaction),
        Box<dyn std::error::Error>,
    > {
        let source = tempfile::tempdir()?;
        run_git(source.path(), &["init", "--quiet"])?;
        fs::write(source.path().join("tracked.txt"), b"before\n")?;
        run_git(source.path(), &["add", "--", "tracked.txt"])?;
        run_git(
            source.path(),
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
        let control = tempfile::tempdir()?;
        let transaction = WorkspaceTransaction::create(
            source.path(),
            control.path().join("run"),
            &[".".to_owned()],
        )?;
        Ok((source, control, transaction))
    }

    #[tokio::test]
    async fn status_keeps_source_and_candidate_evidence_separate()
    -> Result<(), Box<dyn std::error::Error>> {
        let (_source, _control, transaction) = fixture()?;
        transaction.write_file("candidate.txt", b"isolated\n")?;
        let policy = PolicyEngine::local_default();
        let context = ToolContext::new(&transaction, &policy, None);

        let output = GitStatusTool.execute(&context, json!({})).await?;
        assert_eq!(output.content["source_repository"]["total_entries"], 0);
        assert_eq!(output.content["isolated_candidate"]["total_changes"], 1);
        assert_eq!(
            output.content["isolated_candidate"]["changes"][0]["path"],
            "candidate.txt"
        );
        assert!(output.succeeded);
        Ok(())
    }

    #[tokio::test]
    async fn git_tools_fail_before_reading_when_file_authority_is_denied()
    -> Result<(), Box<dyn std::error::Error>> {
        let (_source, _control, transaction) = fixture()?;
        let policy = PolicyEngine::new(PermissionSet::default());
        let context = ToolContext::new(&transaction, &policy, None);
        let error = GitHistoryTool
            .execute(&context, json!({}))
            .await
            .err()
            .ok_or("expected policy denial")?;
        assert!(matches!(error, ToolError::Denied(_)));
        Ok(())
    }

    #[test]
    fn default_registry_exposes_the_git_evidence_tools() -> Result<(), Box<dyn std::error::Error>> {
        let names = crate::builtin_registry()?
            .descriptors()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert!(names.iter().any(|name| name == "git_status"));
        assert!(names.iter().any(|name| name == "git_diff"));
        assert!(names.iter().any(|name| name == "git_history"));
        Ok(())
    }
}
