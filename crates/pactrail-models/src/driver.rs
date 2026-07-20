use async_trait::async_trait;
use thiserror::Error;

use crate::{ModelCapabilities, ModelRequest, ModelResponse, ModelStreamEvent};

/// Receives transient progress while a driver assembles one complete response.
pub trait ModelStreamObserver: Send + Sync {
    /// Observes one bounded provider-neutral stream event.
    fn on_event(&self, event: &ModelStreamEvent);
}

/// Capability-driven model endpoint used by the execution engine.
#[async_trait]
pub trait ModelDriver: Send + Sync {
    /// Stable provider configuration name.
    fn name(&self) -> &str;

    /// Configured model identifier.
    fn model(&self) -> &str;

    /// Capabilities used by context and tool compilers.
    fn capabilities(&self) -> &ModelCapabilities;

    /// Performs one model turn.
    ///
    /// # Errors
    ///
    /// Returns [`ModelError`] for invalid requests, transport failures, provider
    /// rejections, malformed responses, or local budget violations.
    async fn invoke(&self, request: &ModelRequest) -> Result<ModelResponse, ModelError>;

    /// Performs one model turn with optional transient progress.
    ///
    /// Drivers without a streaming transport inherit a complete-response
    /// adapter. The returned response remains the only authoritative output.
    async fn invoke_with_observer(
        &self,
        request: &ModelRequest,
        observer: &dyn ModelStreamObserver,
    ) -> Result<ModelResponse, ModelError> {
        let response = self.invoke(request).await?;
        observer.on_event(&ModelStreamEvent::ResponseStarted {
            provider_request_id: response.provider_request_id.clone(),
            time_to_first_byte_ms: 0,
        });
        if !response.text.is_empty() {
            observer.on_event(&ModelStreamEvent::TextDelta {
                text: response.text.clone(),
            });
        }
        for (index, call) in response.tool_calls.iter().enumerate() {
            observer.on_event(&ModelStreamEvent::ToolCallStarted {
                index,
                id: call.id.clone(),
                name: call.name.clone(),
            });
        }
        observer.on_event(&ModelStreamEvent::UsageUpdate {
            usage: response.usage,
        });
        Ok(response)
    }
}

/// Provider invocation or protocol failure.
#[derive(Debug, Error)]
pub enum ModelError {
    #[error("model request is invalid: {0}")]
    InvalidRequest(String),
    #[error("model transport failed: {0}")]
    Transport(reqwest::Error),
    #[error("provider rejected the request with HTTP {status}: {message}")]
    Provider { status: u16, message: String },
    #[error("provider response exceeded the {limit}-byte limit")]
    ResponseTooLarge { limit: usize },
    #[error("provider response is malformed: {0}")]
    MalformedResponse(String),
    #[error("provider response JSON is invalid: {0}")]
    Json(serde_json::Error),
}
