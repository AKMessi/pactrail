use async_trait::async_trait;
use serde_json::Value;

use crate::{
    CapabilitySource, ModelCapabilities, ModelDriver, ModelError, ModelPhase, ModelRequest,
    ModelResponse, ModelStreamObserver,
};

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

    fn select(&self, phase: Option<ModelPhase>) -> (&dyn ModelDriver, &'static str) {
        if phase == Some(ModelPhase::Investigation) {
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

    async fn invoke(&self, request: &ModelRequest) -> Result<ModelResponse, ModelError> {
        let (model, route) = self.select(request.phase);
        let response = model.invoke(request).await?;
        Ok(Self::annotate(response, model, route))
    }

    async fn invoke_with_observer(
        &self,
        request: &ModelRequest,
        observer: &dyn ModelStreamObserver,
    ) -> Result<ModelResponse, ModelError> {
        let (model, route) = self.select(request.phase);
        let response = model.invoke_with_observer(request, observer).await?;
        Ok(Self::annotate(response, model, route))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::{FinishReason, Message, Usage};

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
        let mut request = ModelRequest {
            conversation: vec![crate::ConversationItem::Message(Message::user("task"))],
            tools: Vec::new(),
            max_output_tokens: 128,
            temperature: None,
            phase: Some(ModelPhase::Investigation),
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
    }
}
