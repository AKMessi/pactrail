use std::collections::BTreeSet;
use std::sync::Mutex;

use pactrail_core::Capability;
use pactrail_tools::{ToolAnnotations, ToolDescriptor};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    ConversationItem, FinishReason, Message, ModelDriver, ModelError, ModelRequest,
    ModelStreamEvent, ModelStreamObserver, Usage,
};

const PROBE_SCHEMA: u16 = 1;
const PROBE_TOOL_NAME: &str = "pactrail_capability_probe";

/// Positive observations from one bounded, side-effect-free model invocation.
///
/// A `false` value means the capability was not observed in this invocation;
/// it does not prove the configured model lacks that capability.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilityProbeReport {
    pub schema: u16,
    pub adapter: String,
    pub model: String,
    pub native_tools: ProbeObservation,
    pub parallel_tools: ProbeObservation,
    pub streaming: ProbeObservation,
    pub prompt_cache: ProbeObservation,
    pub valid_probe_calls: usize,
    pub finish_reason: FinishReason,
    pub usage: Usage,
}

/// Result of one positive-only capability check.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeObservation {
    Observed,
    NotObserved,
}

impl ProbeObservation {
    #[must_use]
    pub const fn from_observed(observed: bool) -> Self {
        if observed {
            Self::Observed
        } else {
            Self::NotObserved
        }
    }

    #[must_use]
    pub const fn is_observed(self) -> bool {
        matches!(self, Self::Observed)
    }
}

/// Performs one bounded capability probe without executing any returned tool.
///
/// The probe supplies a synthetic read-only descriptor and requests two calls
/// with distinct nonces. Only structurally valid calls are counted. The model
/// response is discarded after normalization and cannot mutate a workspace.
///
/// # Errors
///
/// Returns the configured driver's normal transport or protocol error when the
/// endpoint cannot complete the probe request.
pub async fn probe_capabilities(
    driver: &dyn ModelDriver,
) -> Result<CapabilityProbeReport, ModelError> {
    let observer = ProbeObserver::default();
    let request = ModelRequest {
        conversation: vec![
            ConversationItem::Message(Message::system(
                "This is a side-effect-free protocol capability probe. Do not answer in prose. Call the provided pactrail_capability_probe tool twice in the same response: once with nonce alpha and once with nonce beta.",
            )),
            ConversationItem::Message(Message::user(
                "Emit the two requested probe tool calls now.",
            )),
        ],
        tools: vec![probe_descriptor()],
        max_output_tokens: driver.capabilities().max_output_tokens.clamp(1, 256),
        temperature: Some(0.0),
    };
    let response = driver.invoke_with_observer(&request, &observer).await?;
    if response.finish_reason == FinishReason::ContentFilter {
        return Err(ModelError::MalformedResponse(
            "provider safety policy blocked the capability probe".to_owned(),
        ));
    }

    let mut call_ids = BTreeSet::new();
    let mut nonces = BTreeSet::new();
    for call in &response.tool_calls {
        if call.name != PROBE_TOOL_NAME || !call_ids.insert(call.id.as_str()) {
            continue;
        }
        let Some(nonce) = call
            .arguments
            .get("nonce")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        if matches!(nonce, "alpha" | "beta") {
            nonces.insert(nonce);
        }
    }
    let valid_probe_calls = nonces.len();
    Ok(CapabilityProbeReport {
        schema: PROBE_SCHEMA,
        adapter: driver.name().to_owned(),
        model: driver.model().to_owned(),
        native_tools: ProbeObservation::from_observed(valid_probe_calls >= 1),
        parallel_tools: ProbeObservation::from_observed(valid_probe_calls >= 2),
        streaming: ProbeObservation::from_observed(
            observer.stream_started()
                && response
                    .extensions
                    .get("streaming")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
        ),
        prompt_cache: ProbeObservation::from_observed(response.usage.cached_input_tokens > 0),
        valid_probe_calls,
        finish_reason: response.finish_reason,
        usage: response.usage,
    })
}

fn probe_descriptor() -> ToolDescriptor {
    ToolDescriptor {
        name: PROBE_TOOL_NAME.to_owned(),
        description: "Side-effect-free Pactrail capability marker; it is never executed."
            .to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "nonce": {"type": "string", "enum": ["alpha", "beta"]}
            },
            "required": ["nonce"],
            "additionalProperties": false
        }),
        required_capability: Capability::FileRead,
        annotations: ToolAnnotations::READ_ONLY,
    }
}

#[derive(Default)]
struct ProbeObserver {
    stream_started: Mutex<bool>,
}

impl ProbeObserver {
    fn stream_started(&self) -> bool {
        *self
            .stream_started
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl ModelStreamObserver for ProbeObserver {
    fn on_event(&self, event: &ModelStreamEvent) {
        if matches!(event, ModelStreamEvent::ResponseStarted { .. }) {
            *self
                .stream_started
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use serde_json::{Map, Value};

    use super::*;
    use crate::{ModelCapabilities, ModelResponse, ToolCall};

    struct ProbeModel {
        response: Mutex<Option<ModelResponse>>,
        capabilities: ModelCapabilities,
    }

    #[async_trait]
    impl ModelDriver for ProbeModel {
        fn name(&self) -> &'static str {
            "fixture"
        }

        fn model(&self) -> &'static str {
            "probe-model"
        }

        fn capabilities(&self) -> &ModelCapabilities {
            &self.capabilities
        }

        async fn invoke(&self, request: &ModelRequest) -> Result<ModelResponse, ModelError> {
            assert_eq!(request.tools.len(), 1);
            assert_eq!(request.tools[0].name, PROBE_TOOL_NAME);
            assert!(request.max_output_tokens <= 256);
            self.response
                .lock()
                .map_err(|_| ModelError::InvalidRequest("fixture lock poisoned".to_owned()))?
                .take()
                .ok_or_else(|| ModelError::InvalidRequest("fixture exhausted".to_owned()))
        }
    }

    fn call(id: &str, nonce: &str) -> ToolCall {
        ToolCall {
            id: id.to_owned(),
            name: PROBE_TOOL_NAME.to_owned(),
            arguments: json!({"nonce": nonce}),
            extensions: Map::new(),
        }
    }

    #[tokio::test]
    async fn positive_observations_require_distinct_valid_calls() {
        let model = ProbeModel {
            response: Mutex::new(Some(ModelResponse {
                text: String::new(),
                tool_calls: vec![call("one", "alpha"), call("two", "beta")],
                finish_reason: FinishReason::ToolCalls,
                usage: Usage {
                    input_tokens: 20,
                    output_tokens: 8,
                    cached_input_tokens: 5,
                },
                provider_request_id: None,
                extensions: Map::from_iter([("streaming".to_owned(), Value::Bool(false))]),
            })),
            capabilities: ModelCapabilities::default(),
        };

        let report = probe_capabilities(&model)
            .await
            .unwrap_or_else(|error| unreachable!("probe: {error}"));

        assert!(report.native_tools.is_observed());
        assert!(report.parallel_tools.is_observed());
        assert!(report.prompt_cache.is_observed());
        assert!(!report.streaming.is_observed());
        assert_eq!(report.valid_probe_calls, 2);
    }

    #[tokio::test]
    async fn missing_or_repeated_markers_never_enable_capabilities() {
        let model = ProbeModel {
            response: Mutex::new(Some(ModelResponse {
                text: "I cannot comply".to_owned(),
                tool_calls: vec![call("one", "alpha"), call("two", "alpha")],
                finish_reason: FinishReason::Complete,
                usage: Usage::default(),
                provider_request_id: None,
                extensions: Map::new(),
            })),
            capabilities: ModelCapabilities::default(),
        };

        let report = probe_capabilities(&model)
            .await
            .unwrap_or_else(|error| unreachable!("probe: {error}"));

        assert!(report.native_tools.is_observed());
        assert!(!report.parallel_tools.is_observed());
        assert_eq!(report.valid_probe_calls, 1);
    }
}
