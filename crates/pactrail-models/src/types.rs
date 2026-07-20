use pactrail_tools::ToolDescriptor;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Participant role in Pactrail's provider-neutral message representation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// Provider-neutral text message.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    /// Creates a system instruction.
    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    /// Creates a user message.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    /// Creates an assistant message.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// Tool invocation requested by a model.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// Tool result returned to a model on the next turn.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ToolResult {
    pub call_id: String,
    pub name: String,
    pub content: Value,
    pub is_error: bool,
}

/// One durable item in the provider-neutral model conversation.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ConversationItem {
    Message(Message),
    AssistantToolCalls { text: String, calls: Vec<ToolCall> },
    ToolResult(ToolResult),
}

/// Declared capabilities of one configured model endpoint.
// Provider capabilities are intentionally independent feature flags rather than
// mutually exclusive states.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(default)]
pub struct ModelCapabilities {
    pub native_tools: bool,
    pub parallel_tools: bool,
    pub structured_output: bool,
    pub vision: bool,
    pub prompt_caching: bool,
    pub streaming: bool,
    pub reasoning_controls: bool,
    pub context_tokens: u64,
    pub max_output_tokens: u64,
    pub source: CapabilitySource,
}

/// Provenance of the effective model capability profile.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySource {
    #[default]
    ConservativeDefault,
    UserDeclared,
    Probed,
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            native_tools: true,
            parallel_tools: false,
            structured_output: false,
            vision: false,
            prompt_caching: false,
            streaming: false,
            reasoning_controls: false,
            context_tokens: 32_768,
            max_output_tokens: 4_096,
            source: CapabilitySource::ConservativeDefault,
        }
    }
}

/// Complete normalized request passed to a model driver.
#[derive(Clone, Debug)]
pub struct ModelRequest {
    pub conversation: Vec<ConversationItem>,
    pub tools: Vec<ToolDescriptor>,
    pub max_output_tokens: u64,
    pub temperature: Option<f32>,
}

/// Reason a model stopped producing output.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Complete,
    ToolCalls,
    Length,
    ContentFilter,
    Unknown,
}

/// Normalized token accounting from a provider.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
}

impl Usage {
    /// Adds another turn's counters without integer overflow.
    #[must_use]
    pub fn saturating_add(self, other: Self) -> Self {
        Self {
            input_tokens: self.input_tokens.saturating_add(other.input_tokens),
            output_tokens: self.output_tokens.saturating_add(other.output_tokens),
            cached_input_tokens: self
                .cached_input_tokens
                .saturating_add(other.cached_input_tokens),
        }
    }

    /// Total reported input and output tokens.
    #[must_use]
    pub const fn total(self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

/// Complete normalized response from a model driver.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ModelResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: FinishReason,
    pub usage: Usage,
    pub provider_request_id: Option<String>,
    /// Non-sensitive provider metadata preserved for diagnostics.
    pub extensions: serde_json::Map<String, Value>,
}

/// Transient, non-authoritative progress from one model response stream.
///
/// These events are suitable for live user interfaces. Pactrail does not
/// persist them or allow partial tool arguments to reach the tool kernel.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ModelStreamEvent {
    /// The provider accepted the request and response bytes began arriving.
    ResponseStarted {
        provider_request_id: Option<String>,
        time_to_first_byte_ms: u64,
    },
    /// A validated UTF-8 assistant-text fragment.
    TextDelta { text: String },
    /// A typed tool call began. Arguments are not yet executable.
    ToolCallStarted {
        index: usize,
        id: String,
        name: String,
    },
    /// Additional JSON argument bytes arrived for an in-progress tool call.
    ToolArgumentsDelta { index: usize, bytes: usize },
    /// Provider-reported cumulative usage became available.
    UsageUpdate { usage: Usage },
}
