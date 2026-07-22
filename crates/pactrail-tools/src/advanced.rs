use std::collections::BTreeSet;

use async_trait::async_trait;
use pactrail_core::Capability;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::builtins::{descriptor, input, mutation_feedback, read_bounded, success};
use crate::registry::replace_checked_preserving_newlines;
use crate::{Tool, ToolContext, ToolDescriptor, ToolError, ToolOutput};

const MAX_BATCH_FILES: usize = 32;
const MAX_BATCH_FILE_BYTES: u64 = 256 * 1024;
const MAX_BATCH_TOTAL_BYTES: usize = 1024 * 1024;
const MAX_EDIT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_EDITS: usize = 64;
const MAX_MEMORY_RESULTS: usize = 12;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadManyFilesInput {
    /// Unique workspace-relative file paths. Absolute paths are forbidden.
    paths: Vec<String>,
}

/// Reads several bounded UTF-8 files in one model round trip.
pub struct ReadManyFilesTool;

#[async_trait]
impl Tool for ReadManyFilesTool {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor::<ReadManyFilesInput>(
            "read_many_files",
            "Read up to 32 small UTF-8 files in one bounded call. Prefer this after list_files or search identifies several relevant files.",
            Capability::FileRead,
        )
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        value: Value,
    ) -> Result<ToolOutput, ToolError> {
        let request: ReadManyFilesInput = input(value, "read_many_files")?;
        if request.paths.is_empty() || request.paths.len() > MAX_BATCH_FILES {
            return Err(ToolError::InvalidRange(format!(
                "paths must contain between 1 and {MAX_BATCH_FILES} files"
            )));
        }
        let unique = request.paths.iter().collect::<BTreeSet<_>>();
        if unique.len() != request.paths.len() {
            return Err(ToolError::InvalidRange(
                "paths must not contain duplicates".to_owned(),
            ));
        }
        for path in &request.paths {
            context.authorize(&Capability::FileRead, path.clone(), "read_many_files")?;
        }

        let mut total_bytes = 0_usize;
        let mut files = Vec::with_capacity(request.paths.len());
        let mut effects = Vec::with_capacity(request.paths.len());
        for relative in &request.paths {
            let path = context.workspace.resolve_read(relative)?;
            let bytes = read_bounded(&path, MAX_BATCH_FILE_BYTES)?;
            total_bytes = total_bytes
                .checked_add(bytes.len())
                .ok_or_else(|| ToolError::InvalidRange("batch size overflowed".to_owned()))?;
            if total_bytes > MAX_BATCH_TOTAL_BYTES {
                return Err(ToolError::InvalidRange(format!(
                    "combined file content exceeds {MAX_BATCH_TOTAL_BYTES} bytes"
                )));
            }
            let file_text =
                String::from_utf8(bytes).map_err(|_| ToolError::NonUtf8(path.clone()))?;
            let lines = file_text.lines().count();
            files.push(json!({
                "path": relative,
                "lines": lines,
                "bytes": file_text.len(),
                "content": file_text,
            }));
            effects.push(format!("fs.read:{relative}"));
        }
        Ok(success(
            json!({ "files": files, "total_bytes": total_bytes }),
            format!("read {} files ({total_bytes} bytes)", request.paths.len()),
            effects,
        ))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TextEdit {
    /// Exact text expected in the current in-memory file version.
    old: String,
    /// Replacement text.
    new: String,
    /// Required occurrence count. Defaults to one.
    #[serde(default = "default_replacement_count")]
    expected_replacements: usize,
}

const fn default_replacement_count() -> usize {
    1
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EditFileInput {
    /// Workspace-relative UTF-8 file path.
    path: String,
    /// Ordered exact replacements validated in memory before one final write.
    edits: Vec<TextEdit>,
}

/// Applies multiple exact replacements to one file with a single final write.
pub struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor::<EditFileInput>(
            "edit_file",
            "Atomically edit one UTF-8 file with 1-64 ordered, exact, count-checked replacements. No partial file is written when validation fails; success returns bounded current-source evidence around the changed lines.",
            Capability::FileWrite,
        )
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        value: Value,
    ) -> Result<ToolOutput, ToolError> {
        let request: EditFileInput = input(value, "edit_file")?;
        if request.edits.is_empty() || request.edits.len() > MAX_EDITS {
            return Err(ToolError::InvalidRange(format!(
                "edits must contain between 1 and {MAX_EDITS} replacements"
            )));
        }
        context.authorize(&Capability::FileRead, request.path.clone(), "edit_file")?;
        context.authorize(&Capability::FileWrite, request.path.clone(), "edit_file")?;
        let path = context.workspace.resolve_read(&request.path)?;
        let bytes = read_bounded(&path, MAX_EDIT_BYTES)?;
        let mut text = String::from_utf8(bytes).map_err(|_| ToolError::NonUtf8(path))?;
        let original_text = text.clone();
        let mut replacements = 0_usize;
        for (index, edit) in request.edits.iter().enumerate() {
            if edit.old.is_empty() || edit.expected_replacements == 0 {
                return Err(ToolError::InvalidRange(format!(
                    "edit {} must have non-empty old text and a positive expected count",
                    index + 1
                )));
            }
            if edit.old == edit.new {
                return Err(ToolError::InvalidRange(format!(
                    "edit {} must change the matched text; no-op edits produce no evidence",
                    index + 1
                )));
            }
            let (edited, actual) = replace_checked_preserving_newlines(
                &text,
                &edit.old,
                &edit.new,
                edit.expected_replacements,
            )
            .map_err(|actual| ToolError::InvalidEditCount {
                index: index + 1,
                expected: edit.expected_replacements,
                actual,
            })?;
            text = edited;
            if u64::try_from(text.len()).unwrap_or(u64::MAX) > MAX_EDIT_BYTES {
                return Err(ToolError::InvalidRange(format!(
                    "edited file would exceed {MAX_EDIT_BYTES} bytes"
                )));
            }
            replacements = replacements.saturating_add(actual);
        }
        context
            .workspace
            .write_file(&request.path, text.as_bytes())?;
        let post_edit = mutation_feedback(&request.path, Some(&original_text), &text);
        Ok(success(
            json!({
                "path": request.path,
                "edits": request.edits.len(),
                "replacements": replacements,
                "result_bytes": text.len(),
                "digest": blake3::hash(text.as_bytes()).to_hex().to_string(),
                "post_edit": post_edit,
            }),
            format!(
                "applied {} edits ({replacements} replacements) to {}",
                request.edits.len(),
                request.path
            ),
            vec![format!("fs.write:{}", request.path)],
        ))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WorkspaceChangesInput {}

/// Inspects the isolated candidate without invoking Git or a shell.
pub struct WorkspaceChangesTool;

#[async_trait]
impl Tool for WorkspaceChangesTool {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor::<WorkspaceChangesInput>(
            "workspace_changes",
            "Inspect all candidate file changes currently staged in Pactrail's isolated transaction, including digests and byte counts.",
            Capability::FileRead,
        )
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        value: Value,
    ) -> Result<ToolOutput, ToolError> {
        let _: WorkspaceChangesInput = input(value, "workspace_changes")?;
        context.authorize(&Capability::FileRead, ".", "workspace_changes")?;
        let changes = context.workspace.changes()?;
        let added = changes.iter().map(|change| change.bytes_added).sum::<u64>();
        let removed = changes
            .iter()
            .map(|change| change.bytes_removed)
            .sum::<u64>();
        Ok(success(
            json!({
                "changes": changes,
                "files": changes.len(),
                "bytes_added": added,
                "bytes_removed": removed,
            }),
            format!("inspected {} candidate file changes", changes.len()),
            vec!["transaction.inspect".to_owned()],
        ))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RecallMemoryInput {
    /// Topic, file, component, convention, or prior decision to recall.
    query: String,
    /// Maximum memories to return.
    #[serde(default = "default_memory_results")]
    max_results: usize,
}

const fn default_memory_results() -> usize {
    6
}

/// Retrieves bounded, provenance-labelled workspace memory.
pub struct RecallMemoryTool;

#[async_trait]
impl Tool for RecallMemoryTool {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor::<RecallMemoryInput>(
            "recall_memory",
            "Recall relevant workspace memory with explicit trust and freshness. Human memories are advisory; receipt-derived history is returned only while all recorded file digests match the current isolated candidate.",
            Capability::MemoryRead,
        )
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        value: Value,
    ) -> Result<ToolOutput, ToolError> {
        let request: RecallMemoryInput = input(value, "recall_memory")?;
        if request.query.trim().is_empty() {
            return Err(ToolError::InvalidRange(
                "memory query cannot be empty".to_owned(),
            ));
        }
        if request.max_results == 0 || request.max_results > MAX_MEMORY_RESULTS {
            return Err(ToolError::InvalidRange(format!(
                "max_results must be between 1 and {MAX_MEMORY_RESULTS}"
            )));
        }
        context.authorize(&Capability::MemoryRead, ".", "recall_memory")?;
        let memory = context.memory.ok_or(ToolError::MemoryUnavailable)?;
        // Validate a wider bounded candidate pool so stale high-ranking history
        // cannot crowd current lower-ranking memory out of the requested result set.
        let scan_limit = request.max_results.saturating_mul(4).min(48);
        let matches = memory.search(&request.query, scan_limit)?;
        let mut eligible = Vec::new();
        let mut stale = 0_usize;
        let mut unverified = 0_usize;
        for item in matches {
            let item = item.validate_against(|path| context.workspace.current_file_digest(path))?;
            if item.validation.eligible_for_model() {
                if eligible.len() < request.max_results {
                    eligible.push(item);
                }
            } else {
                match item.validation.freshness {
                    pactrail_memory::MemoryFreshness::Stale => {
                        stale = stale.saturating_add(1);
                    }
                    pactrail_memory::MemoryFreshness::Unverified => {
                        unverified = unverified.saturating_add(1);
                    }
                    pactrail_memory::MemoryFreshness::Advisory
                    | pactrail_memory::MemoryFreshness::Current => {}
                }
            }
        }
        let eligible_count = eligible.len();
        Ok(success(
            json!({
                "memories": eligible,
                "excluded": {
                    "stale": stale,
                    "unverified": unverified,
                },
                "policy": "stale and unverified receipt memories are withheld; current files remain authoritative",
            }),
            format!(
                "recalled {} current/advisory workspace memories for {:?}; withheld {stale} stale and {unverified} unverified",
                eligible_count, request.query
            ),
            vec!["memory.read:workspace".to_owned()],
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use pactrail_core::{
        ChangeReceipt, Evidence, EvidenceKind, FileChange, ReceiptInput, ReceiptOutcome, RunId,
        TaskContract,
    };
    use pactrail_memory::{MemoryDraft, MemoryKind, MemoryStore};
    use serde_json::json;

    use super::*;
    use crate::PolicyEngine;
    use pactrail_workspace::WorkspaceTransaction;

    fn fixture() -> (tempfile::TempDir, tempfile::TempDir, WorkspaceTransaction) {
        let source = tempfile::tempdir().unwrap_or_else(|error| unreachable!("source: {error}"));
        fs::write(source.path().join("a.txt"), "alpha\nbeta\n")
            .unwrap_or_else(|error| unreachable!("fixture: {error}"));
        fs::write(source.path().join("b.txt"), "bravo\n")
            .unwrap_or_else(|error| unreachable!("fixture: {error}"));
        let control = tempfile::tempdir().unwrap_or_else(|error| unreachable!("control: {error}"));
        let transaction = WorkspaceTransaction::create(
            source.path(),
            control.path().join("run"),
            &[".".to_owned()],
        )
        .unwrap_or_else(|error| unreachable!("transaction: {error}"));
        (source, control, transaction)
    }

    #[tokio::test]
    async fn multi_edit_validates_every_edit_before_writing() {
        let (_source, _control, transaction) = fixture();
        let policy = PolicyEngine::local_default();
        let context = ToolContext::new(&transaction, &policy, None);
        let failed = EditFileTool
            .execute(
                &context,
                json!({
                    "path": "a.txt",
                    "edits": [
                        {"old": "alpha", "new": "ALPHA"},
                        {"old": "missing", "new": "value"}
                    ]
                }),
            )
            .await;
        assert!(matches!(
            failed,
            Err(ToolError::InvalidEditCount { index: 2, .. })
        ));
        assert_eq!(
            fs::read_to_string(transaction.workspace_root().join("a.txt")).ok(),
            Some("alpha\nbeta\n".to_owned())
        );

        let output = EditFileTool
            .execute(
                &context,
                json!({
                    "path": "a.txt",
                    "edits": [
                        {"old": "alpha", "new": "ALPHA"},
                        {"old": "beta", "new": "BETA"}
                    ]
                }),
            )
            .await
            .unwrap_or_else(|error| unreachable!("edit: {error}"));
        assert_eq!(
            fs::read_to_string(transaction.workspace_root().join("a.txt")).ok(),
            Some("ALPHA\nBETA\n".to_owned())
        );
        assert_eq!(output.content["post_edit"]["changed_line_start"], 1);
        assert_eq!(output.content["post_edit"]["changed_line_end"], 2);
        assert_eq!(
            output.content["post_edit"]["changed_lines_fully_shown"],
            true
        );
        assert_eq!(output.content["digest"].as_str().map(str::len), Some(64));
    }

    #[tokio::test]
    async fn batch_read_and_change_inspection_are_bounded_native_tools() {
        let (_source, _control, transaction) = fixture();
        let policy = PolicyEngine::local_default();
        let context = ToolContext::new(&transaction, &policy, None);
        let read = ReadManyFilesTool
            .execute(&context, json!({"paths": ["a.txt", "b.txt"]}))
            .await
            .unwrap_or_else(|error| unreachable!("batch read: {error}"));
        assert_eq!(read.content["files"].as_array().map(Vec::len), Some(2));
        transaction
            .write_file("new.txt", b"candidate\n")
            .unwrap_or_else(|error| unreachable!("candidate: {error}"));
        let changes = WorkspaceChangesTool
            .execute(&context, json!({}))
            .await
            .unwrap_or_else(|error| unreachable!("changes: {error}"));
        assert_eq!(changes.content["files"], 1);
        assert_eq!(changes.content["changes"][0]["path"], "new.txt");
    }

    #[tokio::test]
    async fn memory_recall_requires_capability_and_preserves_provenance() {
        let (_source, _control, transaction) = fixture();
        let memory =
            MemoryStore::open_in_memory().unwrap_or_else(|error| unreachable!("memory: {error}"));
        memory
            .remember(MemoryDraft {
                kind: MemoryKind::Decision,
                title: "Parser ownership".to_owned(),
                content: "The parser module owns normalization.".to_owned(),
                tags: vec!["parser".to_owned()],
            })
            .unwrap_or_else(|error| unreachable!("remember: {error}"));
        let policy = PolicyEngine::local_default().with_allowed(Capability::MemoryRead);
        let context = ToolContext::new(&transaction, &policy, Some(&memory));
        let output = RecallMemoryTool
            .execute(&context, json!({"query": "parser"}))
            .await
            .unwrap_or_else(|error| unreachable!("recall: {error}"));
        assert_eq!(output.content["memories"][0]["memory"]["source"], "user");
        assert_eq!(
            output.content["memories"][0]["memory"]["title"],
            "Parser ownership"
        );
        assert_eq!(
            output.content["memories"][0]["validation"]["freshness"],
            "advisory"
        );
        assert_eq!(output.content["excluded"]["stale"], 0);
    }

    #[tokio::test]
    async fn memory_recall_withholds_receipt_history_after_candidate_drift() {
        let (_source, _control, transaction) = fixture();
        let current_digest = transaction
            .current_file_digest("a.txt")
            .unwrap_or_else(|error| unreachable!("digest: {error}"));
        let contract = TaskContract::new("Verified parser anchor", ".");
        let receipt = ChangeReceipt::build(ReceiptInput {
            run_id: RunId::new(),
            evidence: vec![Evidence::deterministic_pass(
                contract.obligations[0].id,
                EvidenceKind::Test,
                "fixture passed",
            )],
            contract,
            outcome: ReceiptOutcome::Applied,
            baseline_digest: "baseline".to_owned(),
            final_event_hash: "event".to_owned(),
            changes: vec![FileChange {
                path: "a.txt".to_owned(),
                before_digest: None,
                after_digest: current_digest,
                before_unix_mode: None,
                after_unix_mode: None,
                bytes_added: 11,
                bytes_removed: 0,
            }],
            approvals: Vec::new(),
            unresolved_risks: Vec::new(),
        })
        .unwrap_or_else(|error| unreachable!("receipt: {error}"));
        let memory =
            MemoryStore::open_in_memory().unwrap_or_else(|error| unreachable!("memory: {error}"));
        memory
            .remember_applied_run(&receipt)
            .unwrap_or_else(|error| unreachable!("remember: {error}"));
        let policy = PolicyEngine::local_default().with_allowed(Capability::MemoryRead);
        let context = ToolContext::new(&transaction, &policy, Some(&memory));

        let current = RecallMemoryTool
            .execute(&context, json!({"query": "parser anchor"}))
            .await
            .unwrap_or_else(|error| unreachable!("current recall: {error}"));
        assert_eq!(
            current.content["memories"].as_array().map(Vec::len),
            Some(1)
        );
        assert_eq!(
            current.content["memories"][0]["validation"]["freshness"],
            "current"
        );

        transaction
            .write_file("a.txt", b"candidate drift\n")
            .unwrap_or_else(|error| unreachable!("drift: {error}"));
        let stale = RecallMemoryTool
            .execute(&context, json!({"query": "parser anchor"}))
            .await
            .unwrap_or_else(|error| unreachable!("stale recall: {error}"));
        assert_eq!(stale.content["memories"].as_array().map(Vec::len), Some(0));
        assert_eq!(stale.content["excluded"]["stale"], 1);
    }
}
