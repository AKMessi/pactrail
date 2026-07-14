//! Capability-driven model providers for Pactrail.

mod driver;
mod openai_compatible;
mod types;

pub use driver::{ModelDriver, ModelError};
pub use openai_compatible::{OpenAiCompatibleConfig, OpenAiCompatibleDriver};
pub use types::{
    ConversationItem, FinishReason, Message, ModelCapabilities, ModelRequest, ModelResponse, Role,
    ToolCall, ToolResult, Usage,
};
