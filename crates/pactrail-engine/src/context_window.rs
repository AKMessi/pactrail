use std::collections::BTreeMap;

use pactrail_models::{ConversationItem, ToolResult};
use pactrail_tools::ToolDescriptor;
use serde::Serialize;
use serde_json::{Value, json};
use thiserror::Error;

const ESTIMATED_BYTES_PER_TOKEN: u64 = 4;
const HIGH_WATER_PERCENT: u64 = 80;
const TARGET_PERCENT: u64 = 65;
const MIN_REQUEST_CEILING_BYTES: usize = 8 * 1024;
const MAX_REQUEST_CEILING_BYTES: usize = 8 * 1024 * 1024;
const PREVIEW_BYTES: usize = 384;
const MAX_ANCHORS: usize = 16;
const MAX_ANCHOR_BYTES: usize = 160;
const COMPACTION_VERSION: u8 = 1;

/// Deterministic provider-neutral controller for model-visible trajectory state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ContextWindow {
    high_water_bytes: usize,
    target_bytes: usize,
}

impl ContextWindow {
    pub(crate) fn from_model_limits(context_tokens: u64, max_output_tokens: u64) -> Self {
        let input_tokens = context_tokens.saturating_sub(max_output_tokens);
        let request_ceiling =
            usize::try_from(input_tokens.saturating_mul(ESTIMATED_BYTES_PER_TOKEN))
                .unwrap_or(MAX_REQUEST_CEILING_BYTES)
                .clamp(MIN_REQUEST_CEILING_BYTES, MAX_REQUEST_CEILING_BYTES);
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
    pub(crate) fn compact(
        self,
        conversation: &mut [ConversationItem],
        tools: &[ToolDescriptor],
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
        let mut after_bytes = before.bytes;
        for index in old_results {
            compact_result_at(conversation, index, &mut compacted_results)?;
            after_bytes = request_bytes(conversation, tools)?;
            if after_bytes <= self.target_bytes {
                break;
            }
        }

        // Recent evidence is deliberately the last material sacrificed. This
        // handles one unexpectedly large tool response without relying on a
        // provider-specific tokenizer or sending a predictably invalid request.
        if after_bytes > self.high_water_bytes {
            for index in latest_results {
                compact_result_at(conversation, index, &mut compacted_results)?;
                after_bytes = request_bytes(conversation, tools)?;
                if after_bytes <= self.target_bytes {
                    break;
                }
            }
        }

        if compacted_results == 0 {
            return Ok(None);
        }
        let after = request_fingerprint(conversation, tools)?;
        Ok(Some(CompactionReport {
            before_bytes: before.bytes,
            after_bytes: after.bytes,
            reclaimed_bytes: before.bytes.saturating_sub(after.bytes),
            compacted_results,
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
    pub(crate) high_water_bytes: usize,
    pub(crate) target_bytes: usize,
    pub(crate) before_digest: String,
    pub(crate) after_digest: String,
}

#[derive(Debug, Error)]
pub(crate) enum ContextWindowError {
    #[error("failed to serialize model context for deterministic compaction: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Serialize)]
struct RequestView<'a> {
    conversation: &'a [ConversationItem],
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
    let bytes = serde_json::to_vec(&RequestView {
        conversation,
        tools,
    })?;
    Ok(RequestFingerprint {
        bytes: bytes.len(),
        digest: blake3::hash(&bytes).to_hex().to_string(),
    })
}

fn request_bytes(
    conversation: &[ConversationItem],
    tools: &[ToolDescriptor],
) -> Result<usize, serde_json::Error> {
    serde_json::to_vec(&RequestView {
        conversation,
        tools,
    })
    .map(|bytes| bytes.len())
}

fn compact_result_at(
    conversation: &mut [ConversationItem],
    index: usize,
    compacted_results: &mut usize,
) -> Result<(), serde_json::Error> {
    let ConversationItem::ToolResult(result) = &mut conversation[index] else {
        return Ok(());
    };
    let original = serde_json::to_vec(&result.content)?;
    let compacted = compacted_content(result, &original);
    let compacted_bytes = serde_json::to_vec(&compacted)?;
    if compacted_bytes.len() >= original.len() {
        return Ok(());
    }
    result.content = compacted;
    *compacted_results = compacted_results.saturating_add(1);
    Ok(())
}

fn compacted_content(result: &ToolResult, original: &[u8]) -> Value {
    let serialized = String::from_utf8_lossy(original);
    let preview = truncate_utf8(&serialized, PREVIEW_BYTES);
    let mut anchors = BTreeMap::new();
    collect_anchors(&result.content, "", &mut anchors);
    json!({
        "pactrail_compacted": true,
        "version": COMPACTION_VERSION,
        "tool": result.name,
        "call_id": result.call_id,
        "is_error": result.is_error,
        "original_bytes": original.len(),
        "original_digest": blake3::hash(original).to_hex().to_string(),
        "anchors": anchors,
        "preview_json": preview,
        "guidance": "This is a deterministic compacted observation. Use the retained assistant tool call and run the tool again with narrower arguments before relying on omitted details."
    })
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
    use pactrail_models::{Message, ToolCall};

    use super::*;

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
    fn model_limits_reserve_output_and_headroom() {
        let window = ContextWindow::from_model_limits(4_096, 512);
        assert_eq!(window.high_water_bytes, 11_468);
        assert_eq!(window.target_bytes, 9_318);
    }
}
