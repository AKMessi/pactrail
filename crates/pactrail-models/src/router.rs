use std::borrow::Cow;
use std::collections::BTreeSet;

use async_trait::async_trait;
use serde_json::Value;

use crate::{
    CapabilitySource, ConversationItem, Message, ModelCapabilities, ModelDriver, ModelError,
    ModelPhase, ModelRequest, ModelResponse, ModelRoute, ModelStreamObserver,
};

const ROUTE_ORIGIN_KEY: &str = "pactrail_route_origin";

/// Opt-in phase router with no fallback or hidden provider substitution.
///
/// Investigation turns use the secondary model. Every other turn, including
/// implementation, validation, synthesis, recovery, and probes, uses primary.
pub struct PhaseModelRouter {
    primary: Box<dyn ModelDriver>,
    investigation: Box<dyn ModelDriver>,
    capabilities: ModelCapabilities,
    identity: String,
}

impl PhaseModelRouter {
    #[must_use]
    pub fn new(primary: Box<dyn ModelDriver>, investigation: Box<dyn ModelDriver>) -> Self {
        let a = primary.capabilities();
        let b = investigation.capabilities();
        let capabilities = ModelCapabilities {
            native_tools: a.native_tools && b.native_tools,
            parallel_tools: a.parallel_tools && b.parallel_tools,
            structured_output: a.structured_output && b.structured_output,
            vision: a.vision && b.vision,
            prompt_caching: a.prompt_caching && b.prompt_caching,
            streaming: a.streaming && b.streaming,
            reasoning_controls: a.reasoning_controls && b.reasoning_controls,
            context_tokens: a.context_tokens.min(b.context_tokens),
            max_output_tokens: a.max_output_tokens.min(b.max_output_tokens),
            source: CapabilitySource::UserDeclared,
        };
        let identity = format!(
            "primary={}/{};investigation={}/{}",
            primary.name(),
            primary.model(),
            investigation.name(),
            investigation.model()
        );
        Self {
            primary,
            investigation,
            capabilities,
            identity,
        }
    }

    fn select(
        &self,
        phase: Option<ModelPhase>,
        route: Option<ModelRoute>,
    ) -> (&dyn ModelDriver, &'static str) {
        if route == Some(ModelRoute::Investigation)
            || (route.is_none() && phase == Some(ModelPhase::Investigation))
        {
            (self.investigation.as_ref(), "investigation")
        } else {
            (self.primary.as_ref(), "primary")
        }
    }

    fn annotate(
        mut response: ModelResponse,
        model: &dyn ModelDriver,
        route: &str,
    ) -> ModelResponse {
        for call in &mut response.tool_calls {
            call.extensions
                .insert(ROUTE_ORIGIN_KEY.to_owned(), Value::String(route.to_owned()));
        }
        response
            .extensions
            .insert("route".to_owned(), Value::String(route.to_owned()));
        response
            .extensions
            .insert("model".to_owned(), Value::String(model.model().to_owned()));
        response.extensions.insert(
            "provider".to_owned(),
            Value::String(model.name().to_owned()),
        );
        response
    }

    fn portable_request<'a>(request: &'a ModelRequest, route: &str) -> Cow<'a, ModelRequest> {
        let foreign_call_ids = request
            .conversation
            .iter()
            .filter_map(|item| match item {
                ConversationItem::AssistantToolCalls { calls, .. }
                    if calls.iter().any(|call| {
                        call.extensions
                            .get(ROUTE_ORIGIN_KEY)
                            .and_then(Value::as_str)
                            != Some(route)
                    }) =>
                {
                    Some(calls.iter().map(|call| call.id.clone()))
                }
                _ => None,
            })
            .flatten()
            .collect::<BTreeSet<_>>();
        if foreign_call_ids.is_empty() {
            return Cow::Borrowed(request);
        }
        let mut request = request.clone();
        request.conversation = request
            .conversation
            .into_iter()
            .map(|item| match item {
                ConversationItem::AssistantToolCalls { text, calls }
                    if calls.iter().any(|call| foreign_call_ids.contains(&call.id)) => {
                        let portable_calls = calls
                            .iter()
                            .map(|call| {
                                serde_json::json!({
                                    "id": call.id,
                                    "name": call.name,
                                    "arguments": call.arguments,
                                })
                            })
                            .collect::<Vec<_>>();
                        ConversationItem::Message(Message::assistant(format!(
                            "Pactrail portable transcript from another model route. Assistant text: {text}\nTool requests: {}",
                            Value::Array(portable_calls)
                        )))
                    }
                ConversationItem::ToolResult(result)
                    if foreign_call_ids.contains(&result.call_id) => {
                        ConversationItem::Message(Message::user(format!(
                            "Pactrail untrusted tool result from another model route: {}",
                            serde_json::json!({
                                "call_id": result.call_id,
                                "name": result.name,
                                "is_error": result.is_error,
                                "content": result.content,
                            })
                        )))
                    }
                other => other,
            })
            .collect();
        Cow::Owned(request)
    }
}

#[async_trait]
impl ModelDriver for PhaseModelRouter {
    fn name(&self) -> &'static str {
        "phase-router"
    }

    fn model(&self) -> &str {
        &self.identity
    }

    fn capabilities(&self) -> &ModelCapabilities {
        &self.capabilities
    }

    fn capabilities_for_phase(&self, phase: ModelPhase) -> &ModelCapabilities {
        self.select(Some(phase), None).0.capabilities()
    }

    fn capabilities_for_route(
        &self,
        phase: ModelPhase,
        route: Option<ModelRoute>,
    ) -> &ModelCapabilities {
        self.select(Some(phase), route).0.capabilities()
    }

    async fn invoke(&self, request: &ModelRequest) -> Result<ModelResponse, ModelError> {
        let (model, route) = self.select(request.phase, request.route);
        let portable = Self::portable_request(request, route);
        let response = model.invoke(&portable).await?;
        Ok(Self::annotate(response, model, route))
    }

    async fn invoke_with_observer(
        &self,
        request: &ModelRequest,
        observer: &dyn ModelStreamObserver,
    ) -> Result<ModelResponse, ModelError> {
        let (model, route) = self.select(request.phase, request.route);
        let portable = Self::portable_request(request, route);
        let response = model.invoke_with_observer(&portable, observer).await?;
        Ok(Self::annotate(response, model, route))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::{FinishReason, Message, ToolCall, Usage};

    struct StaticModel {
        label: &'static str,
        capabilities: ModelCapabilities,
        calls: Mutex<u32>,
    }

    #[async_trait]
    impl ModelDriver for StaticModel {
        fn name(&self) -> &'static str {
            "static"
        }

        fn model(&self) -> &str {
            self.label
        }

        fn capabilities(&self) -> &ModelCapabilities {
            &self.capabilities
        }

        async fn invoke(&self, _request: &ModelRequest) -> Result<ModelResponse, ModelError> {
            *self
                .calls
                .lock()
                .map_err(|_| ModelError::InvalidRequest("lock".to_owned()))? += 1;
            Ok(ModelResponse {
                text: self.label.to_owned(),
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Complete,
                usage: Usage::default(),
                provider_request_id: None,
                extensions: serde_json::Map::new(),
            })
        }
    }

    #[tokio::test]
    async fn routing_is_phase_explicit_and_advertises_safe_capability_intersection() {
        let primary = StaticModel {
            label: "strong",
            capabilities: ModelCapabilities::default(),
            calls: Mutex::new(0),
        };
        let investigation = StaticModel {
            label: "economical",
            capabilities: ModelCapabilities {
                native_tools: false,
                context_tokens: 8_192,
                max_output_tokens: 1_024,
                ..ModelCapabilities::default()
            },
            calls: Mutex::new(0),
        };
        let router = PhaseModelRouter::new(Box::new(primary), Box::new(investigation));
        assert!(!router.capabilities().native_tools);
        assert_eq!(router.capabilities().context_tokens, 8_192);
        assert!(
            !router
                .capabilities_for_phase(ModelPhase::Investigation)
                .native_tools
        );
        assert!(
            router
                .capabilities_for_phase(ModelPhase::Implementation)
                .native_tools
        );
        let mut request = ModelRequest {
            conversation: vec![ConversationItem::Message(Message::user("task"))],
            tools: Vec::new(),
            max_output_tokens: 128,
            temperature: None,
            phase: Some(ModelPhase::Investigation),
            route: None,
        };
        let first = router
            .invoke(&request)
            .await
            .unwrap_or_else(|error| unreachable!("investigation: {error}"));
        assert_eq!(first.text, "economical");
        assert_eq!(first.extensions["route"], "investigation");
        request.phase = Some(ModelPhase::Implementation);
        let second = router
            .invoke(&request)
            .await
            .unwrap_or_else(|error| unreachable!("primary: {error}"));
        assert_eq!(second.text, "strong");
        assert_eq!(second.extensions["route"], "primary");
        request.route = Some(ModelRoute::Investigation);
        assert!(
            !router
                .capabilities_for_route(ModelPhase::Implementation, request.route)
                .native_tools
        );
        let override_response = router
            .invoke(&request)
            .await
            .unwrap_or_else(|error| unreachable!("explicit route: {error}"));
        assert_eq!(override_response.text, "economical");
        assert_eq!(override_response.extensions["route"], "investigation");
    }

    #[test]
    fn cross_route_requests_render_portable_tool_transcript_without_opaque_state() {
        let call = ToolCall {
            id: "call-1".to_owned(),
            name: "read_file".to_owned(),
            arguments: serde_json::json!({"path": "src/lib.rs"}),
            extensions: serde_json::Map::from_iter([
                (
                    ROUTE_ORIGIN_KEY.to_owned(),
                    Value::String("investigation".to_owned()),
                ),
                (
                    "openai_responses_output_items".to_owned(),
                    serde_json::json!([{"type": "reasoning", "encrypted_content": "sealed"}]),
                ),
                (
                    "thought_signature".to_owned(),
                    Value::String("sealed".to_owned()),
                ),
            ]),
        };
        let request = ModelRequest {
            conversation: vec![
                ConversationItem::AssistantToolCalls {
                    text: String::new(),
                    calls: vec![call],
                },
                ConversationItem::ToolResult(crate::ToolResult {
                    call_id: "call-1".to_owned(),
                    name: "read_file".to_owned(),
                    content: serde_json::json!({"text": "source"}),
                    is_error: false,
                }),
            ],
            tools: Vec::new(),
            max_output_tokens: 128,
            temperature: None,
            phase: Some(ModelPhase::Implementation),
            route: None,
        };
        let same = PhaseModelRouter::portable_request(&request, "investigation");
        let cross = PhaseModelRouter::portable_request(&request, "primary");
        assert!(matches!(&same, Cow::Borrowed(_)));
        assert!(matches!(&cross, Cow::Owned(_)));
        let ConversationItem::AssistantToolCalls { calls: same, .. } = &same.conversation[0] else {
            unreachable!("assistant call")
        };
        assert!(
            same[0]
                .extensions
                .contains_key("openai_responses_output_items")
        );
        let ConversationItem::Message(call_message) = &cross.conversation[0] else {
            unreachable!("portable assistant message")
        };
        assert!(call_message.content.contains("call-1"));
        assert!(call_message.content.contains("src/lib.rs"));
        assert!(!call_message.content.contains("sealed"));
        let ConversationItem::Message(result_message) = &cross.conversation[1] else {
            unreachable!("portable tool result")
        };
        assert!(result_message.content.contains("source"));
    }
}
