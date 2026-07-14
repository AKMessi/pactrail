use async_trait::async_trait;
use thiserror::Error;

use crate::{ModelCapabilities, ModelRequest, ModelResponse};

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
