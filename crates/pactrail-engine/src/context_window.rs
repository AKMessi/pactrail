use std::collections::BTreeMap;

use pactrail_models::{ConversationItem, ToolResult};
use pactrail_store::{ArtifactError, ArtifactStore};
use pactrail_tools::ToolDescriptor;
use serde::Serialize;
use serde_json::{Value, json};
use thiserror::Error;

use crate::controller::tool_result_digest;

const ESTIMATED_BYTES_PER_TOKEN: u64 = 4;
const HIGH_WATER_PERCENT: u64 = 80;
const TARGET_PERCENT: u64 = 65;
const MIN_REQUEST_CEILING_BYTES: usize = 8 * 1024;
const MAX_REQUEST_CEILING_BYTES: usize = 8 * 1024 * 1024;
const PREVIEW_BYTES: usize = 384;
const MAX_ANCHORS: usize = 16;
const MAX_ANCHOR_BYTES: usize = 160;
const COMPACTION_VERSION: u8 = 1;
const MIN_DUPLICATE_BYTES: usize = 512;

/// Exact duplicate evidence replaced by a small, digest-bound reference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeduplicationReport {
    pub(crate) source_call_id: String,
    pub(crate) duplicate_call_id: String,
    pub(crate) content_digest: String,
    pub(crate) original_bytes: usize,
    pub(crate) reference_bytes: usize,
    pub(crate) artifact_written: bool,
}

/// Keeps the first complete tool observation in the stable request prefix and
/// replaces a new, identical observation with a reference to it. A later
/// context compaction may summarize the source; the reference then instructs
/// the model to rerun the tool before relying on omitted details.
#[cfg(test)]
pub(crate) fn append_deduplicated_result(
    conversation: &mut Vec<ConversationItem>,
    result: ToolResult,
) -> Result<Option<DeduplicationReport>, ContextWindowError> {
    append_deduplicated_result_with_artifacts(conversation, result, None)
}

pub(crate) fn append_deduplicated_result_with_artifacts(
    conversation: &mut Vec<ConversationItem>,
    result: ToolResult,
    artifacts: Option<&ArtifactStore>,
) -> Result<Option<DeduplicationReport>, ContextWindowError> {
    let original = serde_json::to_vec(&result.content)?;
    if original.len() < MIN_DUPLICATE_BYTES {
        conversation.push(ConversationItem::ToolResult(result));
        return Ok(None);
    }
    let source = conversation.iter().find_map(|item| match item {
        ConversationItem::ToolResult(previous)
            if previous.name == result.name
                && previous.is_error == result.is_error
                && !is_compacted(previous)
                && !is_duplicate(previous)
                && previous.content == result.content =>
        {
            Some(previous.call_id.as_str())
        }
        _ => None,
    });
    let Some(source_call_id) = source else {
        conversation.push(ConversationItem::ToolResult(result));
        return Ok(None);
    };
    let digest = blake3::hash(&original).to_hex().to_string();
    let mut reference = json!({
        "pactrail_duplicate": true,
        "version": 1,
        "source_call_id": source_call_id,
        "original_digest": digest.clone(),
        "semantic_digest": tool_result_digest(&result),
        "original_bytes": original.len(),
        "guidance": if artifacts.is_some() {
            "This tool output exactly matches source_call_id. The exact JSON is available through read_observation using original_digest; read a bounded slice when details are needed."
        } else {
            "This tool output exactly matches source_call_id. If that result was compacted, rerun the tool before relying on omitted details."
        }
    });
    if artifacts.is_some() {
        reference["artifact_digest"] = Value::String(digest.clone());
    }
    let reference_bytes = serde_json::to_vec(&reference)?.len();
    if reference_bytes >= original.len() {
        conversation.push(ConversationItem::ToolResult(result));
        return Ok(None);
    }
    if let Some(store) = artifacts {
        let stored = store.put(&original)?;
        debug_assert_eq!(stored.digest, digest);
    }
    let report = DeduplicationReport {
        source_call_id: source_call_id.to_owned(),
        duplicate_call_id: result.call_id.clone(),
        content_digest: digest,
        original_bytes: original.len(),
        reference_bytes,
        artifact_written: artifacts.is_some(),
    };
    conversation.push(ConversationItem::ToolResult(ToolResult {
        content: reference,
        ..result
    }));
    Ok(Some(report))
}

fn is_duplicate(result: &ToolResult) -> bool {
    result
        .content
        .get("pactrail_duplicate")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Deterministic provider-neutral controller for model-visible trajectory state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ContextWindow {
    high_water_bytes: usize,
    target_bytes: usize,
}

impl ContextWindow {
    #[cfg(test)]
    pub(crate) fn from_model_limits(context_tokens: u64, max_output_tokens: u64) -> Self {
        Self::from_model_limits_with_request_ceiling(
            context_tokens,
            max_output_tokens,
            MAX_REQUEST_CEILING_BYTES,
        )
    }

    pub(crate) fn from_model_limits_with_request_ceiling(
        context_tokens: u64,
        max_output_tokens: u64,
        request_ceiling_bytes: usize,
    ) -> Self {
        let input_tokens = context_tokens.saturating_sub(max_output_tokens);
        let request_ceiling =
            usize::try_from(input_tokens.saturating_mul(ESTIMATED_BYTES_PER_TOKEN))
                .unwrap_or(MAX_REQUEST_CEILING_BYTES)
                .clamp(MIN_REQUEST_CEILING_BYTES, MAX_REQUEST_CEILING_BYTES)
                .min(request_ceiling_bytes.max(MIN_REQUEST_CEILING_BYTES));
        Self {
            high_water_bytes: percentage(request_ceiling, HIGH_WATER_PERCENT),
            target_bytes: percentage(request_ceiling, TARGET_PERCENT),
        }
    }

    #[cfg(test)]
    const fn with_limits(high_water_bytes: usize, target_bytes: usize) -> Self {
        Self {
            high_water_bytes,
            target_bytes,
        }
    }

    /// Compacts old tool observations without changing conversation topology.
    ///
    /// The latest tool turn remains byte-for-byte intact whenever compacting
    /// older results is sufficient. If the latest result alone exceeds the
    /// high-water mark, it is compacted as a final safety valve. Assistant tool
    /// calls are retained, so providers continue to receive valid call/result
    /// pairs and the model can repeat a call with narrower arguments.
    #[cfg(test)]
    pub(crate) fn compact(
        self,
        conversation: &mut [ConversationItem],
        tools: &[ToolDescriptor],
    ) -> Result<Option<CompactionReport>, ContextWindowError> {
        self.compact_with_artifacts(conversation, tools, None)
    }

    pub(crate) fn compact_with_artifacts(
        self,
        conversation: &mut [ConversationItem],
        tools: &[ToolDescriptor],
        artifacts: Option<&ArtifactStore>,
    ) -> Result<Option<CompactionReport>, ContextWindowError> {
        let before = request_fingerprint(conversation, tools)?;
        if before.bytes <= self.high_water_bytes {
            return Ok(None);
        }

        let latest_turn_start = conversation
            .iter()
            .rposition(|item| matches!(item, ConversationItem::AssistantToolCalls { .. }));
        let mut old_results = Vec::new();
        let mut latest_results = Vec::new();
        for (index, item) in conversation.iter().enumerate() {
            let ConversationItem::ToolResult(result) = item else {
                continue;
            };
            if is_compacted(result) {
                continue;
            }
            if latest_turn_start.is_some_and(|start| index > start) {
                latest_results.push(index);
            } else {
                old_results.push(index);
            }
        }

        let mut compacted_results = 0_usize;
        let mut artifacts_written = 0_usize;
        let mut after_bytes = before.bytes;
        for index in old_results {
            let reclaimed = compact_result_at(
                conversation,
                index,
                artifacts,
                &mut compacted_results,
                &mut artifacts_written,
            )?;
            after_bytes = after_bytes
                .checked_sub(reclaimed)
                .ok_or(ContextWindowError::Accounting)?;
            if after_bytes <= self.target_bytes {
                break;
            }
        }

        // Recent evidence is deliberately the last material sacrificed. This
        // handles one unexpectedly large tool response without relying on a
        // provider-specific tokenizer or sending a predictably invalid request.
        if after_bytes > self.high_water_bytes {
            for index in latest_results {
                let reclaimed = compact_result_at(
                    conversation,
                    index,
                    artifacts,
                    &mut compacted_results,
                    &mut artifacts_written,
                )?;
                after_bytes = after_bytes
                    .checked_sub(reclaimed)
                    .ok_or(ContextWindowError::Accounting)?;
                if after_bytes <= self.target_bytes {
                    break;
                }
            }
        }

        if compacted_results == 0 {
            return Ok(None);
        }
        let after = request_fingerprint(conversation, tools)?;
        if after.bytes != after_bytes {
            return Err(ContextWindowError::Accounting);
        }
        Ok(Some(CompactionReport {
            before_bytes: before.bytes,
            after_bytes: after.bytes,
            reclaimed_bytes: before.bytes.saturating_sub(after.bytes),
            compacted_results,
            artifacts_written,
            high_water_bytes: self.high_water_bytes,
            target_bytes: self.target_bytes,
            before_digest: before.digest,
            after_digest: after.digest,
        }))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompactionReport {
    pub(crate) before_bytes: usize,
    pub(crate) after_bytes: usize,
    pub(crate) reclaimed_bytes: usize,
    pub(crate) compacted_results: usize,
    pub(crate) artifacts_written: usize,
    pub(crate) high_water_bytes: usize,
    pub(crate) target_bytes: usize,
    pub(crate) before_digest: String,
    pub(crate) after_digest: String,
}

#[derive(Debug, Error)]
pub(crate) enum ContextWindowError {
    #[error("model context byte accounting disagreed with serialized request")]
    Accounting,
    #[error("failed to serialize model context for deterministic compaction: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("failed to persist an exact tool observation: {0}")]
    Artifact(#[from] ArtifactError),
}

#[derive(Serialize)]
struct RequestView<'a> {
    conversation: Value,
    tools: &'a [ToolDescriptor],
}

struct RequestFingerprint {
    bytes: usize,
    digest: String,
}

fn request_fingerprint(
    conversation: &[ConversationItem],
    tools: &[ToolDescriptor],
) -> Result<RequestFingerprint, serde_json::Error> {
    let bytes = normalized_request_bytes(conversation, tools)?;
    Ok(RequestFingerprint {
        bytes: bytes.len(),
        digest: blake3::hash(&bytes).to_hex().to_string(),
    })
}

fn normalized_request_bytes(
    conversation: &[ConversationItem],
    tools: &[ToolDescriptor],
) -> Result<Vec<u8>, serde_json::Error> {
    let mut conversation = serde_json::to_value(conversation)?;
    if let Some(items) = conversation.as_array_mut() {
        for item in items {
            if item.get("type").and_then(Value::as_str) != Some("user_content") {
                continue;
            }
            let Some(images) = item
                .get_mut("data")
                .and_then(|data| data.get_mut("images"))
                .and_then(Value::as_array_mut)
            else {
                continue;
            };
            for image in images {
                let digest = image
                    .get("digest")
                    .and_then(Value::as_str)
                    .unwrap_or("invalid")
                    .to_owned();
                if let Some(object) = image.as_object_mut() {
                    object.insert(
                        "data_base64".to_owned(),
                        Value::String(format!("<sealed-image:{digest}>")),
                    );
                }
            }
        }
    }
    serde_json::to_vec(&RequestView {
        conversation,
        tools,
    })
}

fn compact_result_at(
    conversation: &mut [ConversationItem],
    index: usize,
    artifacts: Option<&ArtifactStore>,
    compacted_results: &mut usize,
    artifacts_written: &mut usize,
) -> Result<usize, ContextWindowError> {
    let ConversationItem::ToolResult(result) = &mut conversation[index] else {
        return Ok(0);
    };
    let original = serde_json::to_vec(&result.content)?;
    let compacted = compacted_content(result, &original, artifacts.is_some());
    let compacted_bytes = serde_json::to_vec(&compacted)?;
    if compacted_bytes.len() >= original.len() {
        return Ok(0);
    }
    if let Some(store) = artifacts {
        let stored = store.put(&original)?;
        debug_assert_eq!(stored.digest, blake3::hash(&original).to_hex().to_string());
        *artifacts_written = artifacts_written.saturating_add(1);
    }
    result.content = compacted;
    *compacted_results = compacted_results.saturating_add(1);
    Ok(original.len().saturating_sub(compacted_bytes.len()))
}

fn compacted_content(result: &ToolResult, original: &[u8], artifact_available: bool) -> Value {
    let serialized = String::from_utf8_lossy(original);
    let preview = truncate_utf8(&serialized, PREVIEW_BYTES);
    let mut anchors = BTreeMap::new();
    collect_anchors(&result.content, "", &mut anchors);
    let digest = blake3::hash(original).to_hex().to_string();
    let mut compacted = json!({
        "pactrail_compacted": true,
        "version": COMPACTION_VERSION,
        "tool": result.name,
        "call_id": result.call_id,
        "is_error": result.is_error,
        "original_bytes": original.len(),
        "original_digest": digest.clone(),
        "semantic_digest": tool_result_digest(result),
        "anchors": anchors,
        "preview_json": preview,
        "guidance": "This is a deterministic compacted observation. Use the retained assistant tool call and run the tool again with narrower arguments before relying on omitted details."
    });
    if artifact_available {
        compacted["artifact_digest"] = Value::String(digest);
        compacted["guidance"] = Value::String(
            "Use read_observation with artifact_digest and a byte offset for exact omitted JSON."
                .to_owned(),
        );
    }
    compacted
}

fn collect_anchors(value: &Value, prefix: &str, anchors: &mut BTreeMap<String, String>) {
    if anchors.len() >= MAX_ANCHORS {
        return;
    }
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for key in keys {
                if anchors.len() >= MAX_ANCHORS {
                    break;
                }
                let child = &object[key];
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                if is_anchor_key(key)
                    && let Some(rendered) = render_anchor(child)
                {
                    anchors.insert(path.clone(), rendered);
                }
                collect_anchors(child, &path, anchors);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().take(MAX_ANCHORS).enumerate() {
                if anchors.len() >= MAX_ANCHORS {
                    break;
                }
                collect_anchors(child, &format!("{prefix}[{index}]"), anchors);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn is_anchor_key(key: &str) -> bool {
    matches!(
        key,
        "changed_files"
            | "changes"
            | "end_line"
            | "error"
            | "exit_code"
            | "files"
            | "guidance"
            | "name"
            | "next_start_line"
            | "path"
            | "paths"
            | "query"
            | "start_line"
            | "suggested_reads"
            | "symbol"
            | "total_lines"
            | "total_matches"
            | "truncated"
    )
}

fn render_anchor(value: &Value) -> Option<String> {
    match value {
        Value::Null | Value::Object(_) => None,
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::String(value) => Some(truncate_utf8(value, MAX_ANCHOR_BYTES)),
        Value::Array(_) => serde_json::to_string(value)
            .ok()
            .map(|rendered| truncate_utf8(&rendered, MAX_ANCHOR_BYTES)),
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}...", &value[..boundary])
}

fn is_compacted(result: &ToolResult) -> bool {
    result
        .content
        .get("pactrail_compacted")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn percentage(value: usize, percent: u64) -> usize {
    value.saturating_mul(usize::try_from(percent).unwrap_or(100)) / 100
}

#[cfg(test)]
mod tests {
    use pactrail_models::{ImageArtifact, Message, ToolCall, UserContent};

    use super::*;
    use crate::controller::ControllerKernel;

    fn tool_turn(id: &str, content: Value) -> [ConversationItem; 2] {
        [
            ConversationItem::AssistantToolCalls {
                text: String::new(),
                calls: vec![ToolCall {
                    id: id.to_owned(),
                    name: "read_file".to_owned(),
                    arguments: json!({"path": format!("src/{id}.rs")}),
                    extensions: serde_json::Map::new(),
                }],
            },
            ConversationItem::ToolResult(ToolResult {
                call_id: id.to_owned(),
                name: "read_file".to_owned(),
                content,
                is_error: false,
            }),
        ]
    }

    #[test]
    fn repeated_large_observation_is_referenced_without_changing_earlier_evidence() {
        let content = json!({"path": "src/lib.rs", "text": "x".repeat(4_096)});
        let mut conversation = vec![ConversationItem::Message(Message::user("inspect"))];
        conversation.extend(tool_turn("first", content.clone()));
        let stable_prefix = conversation.clone();
        conversation.push(tool_turn("second", content.clone())[0].clone());
        let report = append_deduplicated_result(
            &mut conversation,
            ToolResult {
                call_id: "second".to_owned(),
                name: "read_file".to_owned(),
                content: content.clone(),
                is_error: false,
            },
        )
        .unwrap_or_else(|error| unreachable!("deduplication: {error}"))
        .unwrap_or_else(|| unreachable!("duplicate was not recognized"));
        assert_eq!(&conversation[..stable_prefix.len()], stable_prefix);
        assert_eq!(report.source_call_id, "first");
        assert!(report.reference_bytes < report.original_bytes);
        let Some(ConversationItem::ToolResult(second)) = conversation.last() else {
            unreachable!("second result")
        };
        assert_eq!(second.call_id, "second");
        assert_eq!(second.content["source_call_id"], "first");
        assert_eq!(second.content["original_digest"], report.content_digest);

        let unique = append_deduplicated_result(
            &mut conversation,
            ToolResult {
                call_id: "third".to_owned(),
                name: "read_file".to_owned(),
                content: json!({"path": "src/other.rs", "text": "y".repeat(4_096)}),
                is_error: false,
            },
        )
        .unwrap_or_else(|error| unreachable!("unique: {error}"));
        assert!(unique.is_none());
    }

    #[test]
    fn controller_resume_preserves_semantic_identity_after_deduplication_and_compaction() {
        let content = json!({"path": "src/lib.rs", "text": "x".repeat(4_096)});
        let original = ToolResult {
            call_id: "first".to_owned(),
            name: "read_file".to_owned(),
            content: content.clone(),
            is_error: false,
        };
        let mut conversation = vec![ConversationItem::Message(Message::user("inspect"))];
        conversation.extend(tool_turn("first", content.clone()));
        conversation.push(tool_turn("second", content.clone())[0].clone());
        append_deduplicated_result(
            &mut conversation,
            ToolResult {
                call_id: "second".to_owned(),
                ..original.clone()
            },
        )
        .unwrap_or_else(|error| unreachable!("deduplication: {error}"));
        ContextWindow::with_limits(2_000, 1_500)
            .compact(&mut conversation, &[])
            .unwrap_or_else(|error| unreachable!("compaction: {error}"));
        let mut restored = ControllerKernel::restore("inspect", 8, &conversation);
        assert_eq!(
            restored.observe_turn(&[(original, false)]).novel_evidence,
            0
        );
    }

    #[test]
    fn duplicate_reference_retains_exact_retrievable_json() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        let artifacts = ArtifactStore::open(root.path())
            .unwrap_or_else(|error| unreachable!("artifacts: {error}"));
        let content = json!({"text": "evidence".repeat(1_000)});
        let original =
            serde_json::to_vec(&content).unwrap_or_else(|error| unreachable!("original: {error}"));
        let mut conversation = vec![ConversationItem::Message(Message::user("inspect"))];
        conversation.extend(tool_turn("first", content.clone()));
        conversation.push(tool_turn("second", content.clone())[0].clone());
        let report = append_deduplicated_result_with_artifacts(
            &mut conversation,
            ToolResult {
                call_id: "second".to_owned(),
                name: "read_file".to_owned(),
                content,
                is_error: false,
            },
            Some(&artifacts),
        )
        .unwrap_or_else(|error| unreachable!("deduplication: {error}"))
        .unwrap_or_else(|| unreachable!("expected duplicate"));
        assert!(report.artifact_written);
        assert_eq!(artifacts.get(&report.content_digest).ok(), Some(original));
        let Some(ConversationItem::ToolResult(reference)) = conversation.last() else {
            unreachable!("reference")
        };
        assert_eq!(reference.content["artifact_digest"], report.content_digest);
        assert!(
            reference.content["guidance"]
                .as_str()
                .is_some_and(|guidance| guidance.contains("read_observation"))
        );
    }

    #[test]
    fn leaves_recent_turn_lossless_when_old_results_are_enough() {
        let mut conversation = vec![ConversationItem::Message(Message::user("fix it"))];
        conversation.extend(tool_turn(
            "old",
            json!({"path": "src/old.rs", "content": "x".repeat(20_000)}),
        ));
        let recent = json!({"path": "src/recent.rs", "content": "current evidence"});
        conversation.extend(tool_turn("recent", recent.clone()));

        let report = ContextWindow::with_limits(8_000, 6_000)
            .compact(&mut conversation, &[])
            .unwrap_or_else(|error| unreachable!("compaction: {error}"))
            .unwrap_or_else(|| unreachable!("large conversation is compacted"));

        assert_eq!(report.compacted_results, 1);
        let ConversationItem::ToolResult(old) = &conversation[2] else {
            unreachable!("old result")
        };
        assert!(is_compacted(old));
        let ConversationItem::ToolResult(latest) = &conversation[4] else {
            unreachable!("latest result")
        };
        assert_eq!(latest.content, recent);
    }

    #[test]
    fn compacts_an_oversized_latest_result_as_a_safety_valve() {
        let mut conversation = vec![ConversationItem::Message(Message::user("inspect"))];
        conversation.extend(tool_turn(
            "latest",
            json!({
                "path": "src/large.rs",
                "start_line": 1,
                "next_start_line": 301,
                "content": "z".repeat(20_000)
            }),
        ));

        let report = ContextWindow::with_limits(4_000, 3_000)
            .compact(&mut conversation, &[])
            .unwrap_or_else(|error| unreachable!("compaction: {error}"))
            .unwrap_or_else(|| unreachable!("large latest result is compacted"));

        assert_eq!(report.compacted_results, 1);
        let ConversationItem::ToolResult(result) = &conversation[2] else {
            unreachable!("result")
        };
        assert_eq!(result.call_id, "latest");
        assert_eq!(result.name, "read_file");
        assert_eq!(result.content["anchors"]["path"], "src/large.rs");
        assert_eq!(result.content["anchors"]["next_start_line"], "301");
        assert_eq!(
            result.content["original_digest"].as_str().map(str::len),
            Some(64)
        );
    }

    #[test]
    fn compacted_observation_has_an_integrity_checked_retrieval_artifact() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        let artifacts = ArtifactStore::open(root.path())
            .unwrap_or_else(|error| unreachable!("artifact store: {error}"));
        let content = json!({"path": "src/lib.rs", "content": "z".repeat(20_000)});
        let original =
            serde_json::to_vec(&content).unwrap_or_else(|error| unreachable!("original: {error}"));
        let mut conversation = vec![ConversationItem::Message(Message::user("inspect"))];
        conversation.extend(tool_turn("latest", content));
        let report = ContextWindow::with_limits(4_000, 3_000)
            .compact_with_artifacts(&mut conversation, &[], Some(&artifacts))
            .unwrap_or_else(|error| unreachable!("compaction: {error}"))
            .unwrap_or_else(|| unreachable!("expected compaction"));
        assert_eq!(report.artifacts_written, 1);
        let ConversationItem::ToolResult(result) = &conversation[2] else {
            unreachable!("result")
        };
        let digest = result.content["artifact_digest"]
            .as_str()
            .unwrap_or_else(|| unreachable!("artifact digest"));
        assert_eq!(result.content["original_digest"], digest);
        assert_eq!(artifacts.get(digest).ok(), Some(original));
    }

    #[test]
    fn compaction_is_deterministic_and_idempotent() {
        let original = vec![
            ConversationItem::Message(Message::system("stable")),
            tool_turn(
                "one",
                json!({"query": "needle", "files": ["a.rs", "b.rs"], "content": "q".repeat(20_000)}),
            )[0]
            .clone(),
            tool_turn(
                "one",
                json!({"query": "needle", "files": ["a.rs", "b.rs"], "content": "q".repeat(20_000)}),
            )[1]
            .clone(),
        ];
        let mut left = original.clone();
        let mut right = original;
        let window = ContextWindow::with_limits(4_000, 3_000);
        let left_report = window
            .compact(&mut left, &[])
            .unwrap_or_else(|error| unreachable!("left: {error}"));
        let right_report = window
            .compact(&mut right, &[])
            .unwrap_or_else(|error| unreachable!("right: {error}"));
        assert_eq!(left, right);
        assert_eq!(left_report, right_report);
        assert!(
            window
                .compact(&mut left, &[])
                .unwrap_or_else(|error| unreachable!("repeat: {error}"))
                .is_none()
        );
    }

    #[test]
    fn incremental_compaction_size_matches_full_request_serialization() {
        let mut conversation = vec![ConversationItem::Message(Message::system("stable"))];
        for index in 0..6 {
            conversation.extend(tool_turn(
                &format!("result-{index}"),
                json!({"text": "x".repeat(6_000), "index": index}),
            ));
        }
        let before = request_fingerprint(&conversation, &[])
            .unwrap_or_else(|error| unreachable!("before: {error}"));
        let report = ContextWindow::with_limits(20_000, 15_000)
            .compact(&mut conversation, &[])
            .unwrap_or_else(|error| unreachable!("compaction: {error}"))
            .unwrap_or_else(|| unreachable!("expected compaction"));
        let after = request_fingerprint(&conversation, &[])
            .unwrap_or_else(|error| unreachable!("after: {error}"));
        assert!(report.compacted_results >= 2);
        assert_eq!(report.before_bytes, before.bytes);
        assert_eq!(report.after_bytes, after.bytes);
        assert_eq!(report.reclaimed_bytes, before.bytes - after.bytes);
        assert_eq!(report.after_digest, after.digest);
    }

    #[test]
    fn model_limits_reserve_output_and_headroom() {
        let window = ContextWindow::from_model_limits(4_096, 512);
        assert_eq!(window.high_water_bytes, 11_468);
        assert_eq!(window.target_bytes, 9_318);
    }

    #[test]
    fn sealed_image_payloads_do_not_masquerade_as_text_context() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&13_u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&640_u32.to_be_bytes());
        png.extend_from_slice(&480_u32.to_be_bytes());
        png.extend_from_slice(&[8, 2, 0, 0, 0]);
        png.extend_from_slice(&[0; 4]);
        png.extend_from_slice(&1_000_000_u32.to_be_bytes());
        png.extend_from_slice(b"IDAT");
        png.extend(std::iter::repeat_n(0, 1_000_000));
        png.extend_from_slice(&[0; 4]);
        png.extend_from_slice(&0_u32.to_be_bytes());
        png.extend_from_slice(b"IEND");
        png.extend_from_slice(&[0; 4]);
        let image = ImageArtifact::from_bytes("screen.png", &png)
            .unwrap_or_else(|error| unreachable!("image: {error}"));
        let conversation = vec![ConversationItem::UserContent(
            UserContent::new("inspect", vec![image])
                .unwrap_or_else(|error| unreachable!("content: {error}")),
        )];

        let fingerprint = request_fingerprint(&conversation, &[])
            .unwrap_or_else(|error| unreachable!("fingerprint: {error}"));

        assert!(fingerprint.bytes < 1_024);
    }
}
