use std::path::PathBuf;

use async_trait::async_trait;
use pactrail_core::Capability;
use pactrail_store::ArtifactStore;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::builtins::{descriptor, input, success};
use crate::{Tool, ToolContext, ToolDescriptor, ToolError, ToolOutput};

const MAX_CHUNK_BYTES: u64 = 4_096;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadObservationInput {
    /// BLAKE3 digest from a Pactrail compacted tool observation.
    digest: String,
    /// UTF-8 byte offset from the beginning of the original JSON.
    #[serde(default)]
    start_byte: u64,
    /// Maximum bytes to return, from 1 through 4096.
    #[serde(default = "default_chunk_bytes")]
    max_bytes: u64,
}

const fn default_chunk_bytes() -> u64 {
    MAX_CHUNK_BYTES
}

/// Reads one bounded slice of a run-local, integrity-checked tool observation.
pub struct ReadObservationTool {
    root: PathBuf,
}

impl ReadObservationTool {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

#[async_trait]
impl Tool for ReadObservationTool {
    fn descriptor(&self) -> ToolDescriptor {
        descriptor::<ReadObservationInput>(
            "read_observation",
            "Read up to 4096 exact UTF-8 bytes of an earlier Pactrail tool observation by its digest and byte offset. Use this for details omitted by a compacted observation.",
            Capability::FileRead,
        )
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        value: Value,
    ) -> Result<ToolOutput, ToolError> {
        let request: ReadObservationInput = input(value, "read_observation")?;
        if !(1..=MAX_CHUNK_BYTES).contains(&request.max_bytes) {
            return Err(ToolError::InvalidRange(format!(
                "max_bytes must be between 1 and {MAX_CHUNK_BYTES}"
            )));
        }
        if request.digest.len() != 64
            || !request
                .digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ToolError::InvalidRange(
                "digest must be a lowercase BLAKE3 hex value".to_owned(),
            ));
        }
        let run_id = context.run_id.ok_or_else(|| {
            ToolError::InvalidRange("read_observation requires a run identifier".to_owned())
        })?;
        context.authorize(
            &Capability::FileRead,
            format!("observation:{}", request.digest),
            "read_observation",
        )?;
        let unavailable = || {
            ToolError::InvalidRange(
                "observation artifact is unavailable or failed integrity validation".to_owned(),
            )
        };
        ArtifactStore::open(&self.root).map_err(|_| unavailable())?;
        let store =
            ArtifactStore::open(self.root.join(run_id.to_string())).map_err(|_| unavailable())?;
        let bytes = store.get(&request.digest).map_err(|_| unavailable())?;
        let observation_text = String::from_utf8(bytes).map_err(|_| {
            ToolError::InvalidRange("observation artifact is not UTF-8 JSON".to_owned())
        })?;
        let start = usize::try_from(request.start_byte).map_err(|_| {
            ToolError::InvalidRange("start_byte exceeds the observation length".to_owned())
        })?;
        if start > observation_text.len() || !observation_text.is_char_boundary(start) {
            return Err(ToolError::InvalidRange(
                "start_byte must be a valid UTF-8 boundary within the observation".to_owned(),
            ));
        }
        let requested_end = start
            .saturating_add(usize::try_from(request.max_bytes).unwrap_or(usize::MAX))
            .min(observation_text.len());
        let mut end = requested_end;
        while end > start && !observation_text.is_char_boundary(end) {
            end -= 1;
        }
        if end == start && start < observation_text.len() {
            return Err(ToolError::InvalidRange(
                "max_bytes cannot fit the next UTF-8 character".to_owned(),
            ));
        }
        let next = (end < observation_text.len()).then_some(end);
        Ok(success(
            json!({
                "digest": request.digest,
                "start_byte": start,
                "end_byte": end,
                "total_bytes": observation_text.len(),
                "next_start_byte": next,
                "content_json": &observation_text[start..end],
            }),
            format!("read observation bytes {start}..{end}"),
            vec![format!("artifact.read:{}", request.digest)],
        ))
    }
}

#[cfg(test)]
mod tests {
    use pactrail_core::RunId;
    use pactrail_workspace::WorkspaceTransaction;

    use super::*;
    use crate::PolicyEngine;

    #[tokio::test]
    async fn reads_only_a_bounded_slice_of_the_current_runs_artifact() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        let source = tempfile::tempdir().unwrap_or_else(|error| unreachable!("source: {error}"));
        let transaction = WorkspaceTransaction::create(
            source.path(),
            root.path().join("transaction"),
            &[".".to_owned()],
        )
        .unwrap_or_else(|error| unreachable!("transaction: {error}"));
        let run_id = RunId::new();
        let store = ArtifactStore::open(root.path().join("observations").join(run_id.to_string()))
            .unwrap_or_else(|error| unreachable!("store: {error}"));
        let artifact = store
            .put("{\"text\":\"αβγ\"}".as_bytes())
            .unwrap_or_else(|error| unreachable!("artifact: {error}"));
        let policy = PolicyEngine::local_default();
        let context = ToolContext {
            workspace: &transaction,
            policy: &policy,
            memory: None,
            run_id: Some(run_id),
            approval_resolver: None,
            policy_audit: None,
        };
        let tool = ReadObservationTool::new(root.path().join("observations"));
        let output = tool
            .execute(
                &context,
                json!({"digest": artifact.digest, "start_byte": 0, "max_bytes": 10}),
            )
            .await
            .unwrap_or_else(|error| unreachable!("read: {error}"));
        assert_eq!(output.content["start_byte"], 0);
        assert_eq!(output.content["next_start_byte"], 9);
        assert!(output.content["content_json"].as_str().is_some());
        assert!(
            tool.execute(&context, json!({"digest": "../other"}))
                .await
                .is_err()
        );
        let other_run = ToolContext {
            run_id: Some(RunId::new()),
            ..context
        };
        let cross_run_error = tool
            .execute(&other_run, json!({"digest": artifact.digest}))
            .await
            .err()
            .unwrap_or_else(|| unreachable!("cross-run read succeeded"));
        assert!(
            !cross_run_error
                .to_string()
                .contains(&root.path().display().to_string())
        );
    }
}
