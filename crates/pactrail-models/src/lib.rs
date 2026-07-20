//! Capability-driven model providers for Pactrail.

mod driver;
mod openai_compatible;
mod sse;
mod types;

pub use driver::{ModelDriver, ModelError, ModelStreamObserver};
pub use openai_compatible::{OpenAiCompatibleConfig, OpenAiCompatibleDriver};
pub use types::{
    CapabilitySource, ConversationItem, FinishReason, Message, ModelCapabilities, ModelRequest,
    ModelResponse, ModelStreamEvent, Role, ToolCall, ToolResult, Usage,
};
