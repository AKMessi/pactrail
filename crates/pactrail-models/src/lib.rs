//! Capability-driven model providers for Pactrail.

mod anthropic;
mod driver;
mod gemini;
mod openai_compatible;
mod probe;
mod sse;
#[cfg(test)]
mod test_support;
mod types;

pub use anthropic::{AnthropicConfig, AnthropicDriver};
pub use driver::{ModelDriver, ModelError, ModelStreamObserver};
pub use gemini::{GeminiConfig, GeminiDriver};
pub use openai_compatible::{OpenAiCompatibleConfig, OpenAiCompatibleDriver};
pub use probe::{
    CAPABILITY_PROBE_SCHEMA_VERSION, CapabilityProbeReport, ProbeObservation, probe_capabilities,
};
pub use types::{
    CapabilitySource, ConversationItem, FinishReason, ImageArtifact, ImageArtifactError,
    ImageMediaType, ImageSetSummary, MAX_INLINE_MODEL_REQUEST_BYTES, MAX_INPUT_IMAGE_BYTES,
    MAX_INPUT_IMAGE_DIMENSION, MAX_INPUT_IMAGES, MAX_TOTAL_INPUT_IMAGE_BYTES,
    MODEL_IR_SCHEMA_VERSION, Message, ModelCapabilities, ModelRequest, ModelResponse,
    ModelStreamEvent, Role, ToolCall, ToolResult, Usage, UserContent, validate_image_set,
};
