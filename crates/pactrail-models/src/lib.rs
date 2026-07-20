//! Capability-driven model providers for Pactrail.

mod anthropic;
mod driver;
mod gemini;
mod openai_compatible;
mod sse;
mod types;

pub use anthropic::{AnthropicConfig, AnthropicDriver};
pub use driver::{ModelDriver, ModelError, ModelStreamObserver};
pub use gemini::{GeminiConfig, GeminiDriver};
pub use openai_compatible::{OpenAiCompatibleConfig, OpenAiCompatibleDriver};
pub use types::{
    CapabilitySource, ConversationItem, FinishReason, Message, ModelCapabilities, ModelRequest,
    ModelResponse, ModelStreamEvent, Role, ToolCall, ToolResult, Usage,
};
