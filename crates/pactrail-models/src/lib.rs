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
pub use probe::{CapabilityProbeReport, ProbeObservation, probe_capabilities};
pub use types::{
    CapabilitySource, ConversationItem, FinishReason, Message, ModelCapabilities, ModelRequest,
    ModelResponse, ModelStreamEvent, Role, ToolCall, ToolResult, Usage,
};
