use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{StatusCode, header::HeaderMap};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use tracing::warn;

use crate::sse::{SseDecoder, SseEvent};
use crate::{
    ConversationItem, FinishReason, Message, ModelCapabilities, ModelDriver, ModelError,
    ModelRequest, ModelResponse, ModelStreamEvent, ModelStreamObserver, Role, ToolCall, Usage,
};

const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_TOOL_ARGUMENT_BYTES: usize = 1024 * 1024;
const MAX_CONTENT_BLOCKS: usize = 256;
const MAX_RETRIES: u32 = 3;
const MIN_RETRY_DELAY: Duration = Duration::from_millis(250);
const MAX_RETRY_DELAY: Duration = Duration::from_mins(1);
const RATE_LIMIT_RETRY_DELAY: Duration = Duration::from_secs(15);

/// Configuration for Anthropic's native Messages API.
#[derive(Clone)]
pub struct AnthropicConfig {
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub api_key: SecretString,
    pub timeout: Duration,
    pub capabilities: ModelCapabilities,
    pub stream: bool,
}

impl AnthropicConfig {
    /// Creates the default hosted Anthropic configuration.
    #[must_use]
    pub fn hosted(model: impl Into<String>, api_key: SecretString) -> Self {
        Self {
            name: "anthropic".to_owned(),
            base_url: "https://api.anthropic.com".to_owned(),
            model: model.into(),
            api_key,
            timeout: Duration::from_mins(5),
            capabilities: ModelCapabilities::default(),
            stream: true,
        }
    }
}

/// Native Anthropic Messages driver with typed content-block streaming.
pub struct AnthropicDriver {
    config: AnthropicConfig,
    client: reqwest::Client,
}

impl AnthropicDriver {
    /// Builds a credential-safe native Anthropic driver.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe endpoint, empty identity or credential,
    /// zero timeout, or HTTP-client construction failure.
    pub fn new(config: AnthropicConfig) -> Result<Self, ModelError> {
        validate_endpoint(&config.base_url)?;
        if config.name.trim().is_empty() || config.model.trim().is_empty() {
            return Err(ModelError::InvalidRequest(
                "provider name and model cannot be empty".to_owned(),
            ));
        }
        if config.api_key.expose_secret().is_empty() {
            return Err(ModelError::InvalidRequest(
                "Anthropic API key cannot be empty".to_owned(),
            ));
        }
        if config.timeout.is_zero() {
            return Err(ModelError::InvalidRequest(
                "provider timeout must be greater than zero".to_owned(),
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("pactrail/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(ModelError::Transport)?;
        Ok(Self { config, client })
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/messages", self.config.base_url.trim_end_matches('/'))
    }

    async fn send_response(
        &self,
        body: &Value,
    ) -> Result<(reqwest::Response, Option<String>, Instant), ModelError> {
        let request_started = Instant::now();
        let mut attempt = 0_u32;
        loop {
            let response = self
                .client
                .post(self.endpoint())
                .header("x-api-key", self.config.api_key.expose_secret())
                .header("anthropic-version", ANTHROPIC_VERSION)
                .json(body)
                .send()
                .await
                .map_err(ModelError::Transport)?;
            let status = response.status();
            let request_id = response
                .headers()
                .get("request-id")
                .or_else(|| response.headers().get("x-request-id"))
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            if status.is_success() {
                return Ok((response, request_id, request_started));
            }
            let retry_after = parse_retry_after(response.headers(), SystemTime::now());
            let retryable = status == StatusCode::TOO_MANY_REQUESTS
                || status == StatusCode::REQUEST_TIMEOUT
                || status.is_server_error();
            let bytes = read_bounded(response).await?;
            let message = anthropic_error_message(&bytes);
            if retryable && attempt < MAX_RETRIES {
                attempt += 1;
                let delay = retry_delay(status, attempt, retry_after);
                warn!(
                    attempt,
                    status = status.as_u16(),
                    delay_ms = delay.as_millis(),
                    "retrying Anthropic request before response acceptance"
                );
                tokio::time::sleep(delay).await;
                continue;
            }
            return Err(ModelError::Provider {
                status: status.as_u16(),
                message,
            });
        }
    }

    async fn invoke_buffered(&self, request: &ModelRequest) -> Result<ModelResponse, ModelError> {
        let body = request_body(&self.config, request, false)?;
        let (response, request_id, _) = self.send_response(&body).await?;
        let bytes = read_bounded(response).await?;
        let value = serde_json::from_slice(&bytes).map_err(ModelError::Json)?;
        parse_response(&value, request_id)
    }

    async fn invoke_streaming(
        &self,
        request: &ModelRequest,
        observer: &dyn ModelStreamObserver,
    ) -> Result<ModelResponse, ModelError> {
        let body = request_body(&self.config, request, true)?;
        let (response, request_id, request_started) = self.send_response(&body).await?;
        require_event_stream(&response)?;
        accumulate_stream(response, request_id, request_started, observer).await
    }
}

#[async_trait]
impl ModelDriver for AnthropicDriver {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn model(&self) -> &str {
        &self.config.model
    }

    fn capabilities(&self) -> &ModelCapabilities {
        &self.config.capabilities
    }

    async fn invoke(&self, request: &ModelRequest) -> Result<ModelResponse, ModelError> {
        self.invoke_buffered(request).await
    }

    async fn invoke_with_observer(
        &self,
        request: &ModelRequest,
        observer: &dyn ModelStreamObserver,
    ) -> Result<ModelResponse, ModelError> {
        if self.config.stream {
            self.invoke_streaming(request, observer).await
        } else {
            let response = self.invoke_buffered(request).await?;
            emit_complete_response(observer, &response);
            Ok(response)
        }
    }
}

fn request_body(
    config: &AnthropicConfig,
    request: &ModelRequest,
    stream: bool,
) -> Result<Value, ModelError> {
    if request.conversation.is_empty() {
        return Err(ModelError::InvalidRequest(
            "at least one message is required".to_owned(),
        ));
    }
    if request.max_output_tokens == 0
        || request.max_output_tokens > config.capabilities.max_output_tokens
    {
        return Err(ModelError::InvalidRequest(format!(
            "max_output_tokens must be between 1 and {}",
            config.capabilities.max_output_tokens
        )));
    }
    let (system, messages) = anthropic_messages(&request.conversation)?;
    let tools = request
        .tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema,
            })
        })
        .collect::<Vec<_>>();
    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "max_tokens": request.max_output_tokens,
        "stream": stream,
    });
    if !system.is_empty() {
        body["system"] = Value::String(system);
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
        body["tool_choice"] = json!({
            "type": "auto",
            "disable_parallel_tool_use": !config.capabilities.parallel_tools,
        });
    }
    if let Some(temperature) = request.temperature {
        if !(0.0..=1.0).contains(&temperature) {
            return Err(ModelError::InvalidRequest(
                "Anthropic temperature must be between 0 and 1".to_owned(),
            ));
        }
        body["temperature"] = json!(temperature);
    }
    Ok(body)
}

fn anthropic_messages(
    conversation: &[ConversationItem],
) -> Result<(String, Vec<Value>), ModelError> {
    let mut system = Vec::new();
    let mut messages = Vec::<Value>::new();
    for item in conversation {
        match item {
            ConversationItem::Message(Message {
                role: Role::System,
                content,
            }) => system.push(content.as_str()),
            ConversationItem::Message(Message { role, content }) => {
                let role = match role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::System => {
                        return Err(ModelError::InvalidRequest(
                            "internal Anthropic system-message routing failed".to_owned(),
                        ));
                    }
                };
                push_message_blocks(
                    &mut messages,
                    role,
                    vec![json!({"type": "text", "text": content})],
                )?;
            }
            ConversationItem::AssistantToolCalls { text, calls } => {
                let mut blocks = Vec::with_capacity(calls.len().saturating_add(1));
                if !text.is_empty() {
                    blocks.push(json!({"type": "text", "text": text}));
                }
                blocks.extend(calls.iter().map(|call| {
                    json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": call.arguments,
                    })
                }));
                push_message_blocks(&mut messages, "assistant", blocks)?;
            }
            ConversationItem::ToolResult(result) => {
                push_message_blocks(
                    &mut messages,
                    "user",
                    vec![json!({
                        "type": "tool_result",
                        "tool_use_id": result.call_id,
                        "content": serde_json::to_string(&result.content).map_err(ModelError::Json)?,
                        "is_error": result.is_error,
                    })],
                )?;
            }
        }
    }
    if messages.is_empty() {
        return Err(ModelError::InvalidRequest(
            "Anthropic requires at least one non-system message".to_owned(),
        ));
    }
    Ok((system.join("\n\n"), messages))
}

fn push_message_blocks(
    messages: &mut Vec<Value>,
    role: &str,
    blocks: Vec<Value>,
) -> Result<(), ModelError> {
    if blocks.is_empty() {
        return Err(ModelError::InvalidRequest(
            "Anthropic message content cannot be empty".to_owned(),
        ));
    }
    if let Some(last) = messages.last_mut()
        && last.get("role").and_then(Value::as_str) == Some(role)
    {
        let content = last
            .get_mut("content")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| {
                ModelError::InvalidRequest("internal Anthropic message shape is invalid".to_owned())
            })?;
        content.extend(blocks);
    } else {
        messages.push(json!({"role": role, "content": blocks}));
    }
    Ok(())
}

fn parse_response(value: &Value, request_id: Option<String>) -> Result<ModelResponse, ModelError> {
    let content = value
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ModelError::MalformedResponse("Anthropic response is missing content".to_owned())
        })?;
    if content.len() > MAX_CONTENT_BLOCKS {
        return Err(ModelError::MalformedResponse(
            "Anthropic response exceeded the content-block limit".to_owned(),
        ));
    }
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for block in content {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => append_text(
                &mut text,
                block
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )?,
            Some("tool_use") => tool_calls.push(parse_tool_use(block)?),
            Some("thinking" | "redacted_thinking") => {}
            Some(other) => {
                return Err(ModelError::MalformedResponse(format!(
                    "unsupported Anthropic content block {other:?}"
                )));
            }
            None => {
                return Err(ModelError::MalformedResponse(
                    "Anthropic content block is missing its type".to_owned(),
                ));
            }
        }
    }
    let finish_reason = anthropic_finish_reason(value.get("stop_reason").and_then(Value::as_str));
    if finish_reason == FinishReason::ToolCalls && tool_calls.is_empty() {
        return Err(ModelError::MalformedResponse(
            "Anthropic stopped for tool use but returned no tool block".to_owned(),
        ));
    }
    let usage = value
        .get("usage")
        .map_or_else(Usage::default, anthropic_usage);
    let mut extensions = serde_json::Map::new();
    for key in ["id", "model", "type"] {
        if let Some(value) = value.get(key) {
            extensions.insert(key.to_owned(), value.clone());
        }
    }
    Ok(ModelResponse {
        text,
        tool_calls,
        finish_reason,
        usage,
        provider_request_id: request_id,
        extensions,
    })
}

fn parse_tool_use(block: &Value) -> Result<ToolCall, ModelError> {
    let id = bounded_identifier(block.get("id"), "Anthropic tool-use id")?;
    let name = bounded_identifier(block.get("name"), "Anthropic tool-use name")?;
    let arguments = block.get("input").cloned().ok_or_else(|| {
        ModelError::MalformedResponse("Anthropic tool-use block is missing input".to_owned())
    })?;
    if !arguments.is_object() {
        return Err(ModelError::MalformedResponse(
            "Anthropic tool input must be a JSON object".to_owned(),
        ));
    }
    Ok(ToolCall {
        id,
        name,
        arguments,
    })
}

fn bounded_identifier(value: Option<&Value>, field: &str) -> Result<String, ModelError> {
    let value = value.and_then(Value::as_str).ok_or_else(|| {
        ModelError::MalformedResponse(format!("{field} is missing or is not a string"))
    })?;
    if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(ModelError::MalformedResponse(format!(
            "{field} is empty, oversized, or contains control characters"
        )));
    }
    Ok(value.to_owned())
}

fn append_text(target: &mut String, delta: &str) -> Result<(), ModelError> {
    if target.len().saturating_add(delta.len()) > MAX_TEXT_BYTES {
        return Err(ModelError::ResponseTooLarge {
            limit: MAX_TEXT_BYTES,
        });
    }
    target.push_str(delta);
    Ok(())
}

fn anthropic_usage(value: &Value) -> Usage {
    Usage {
        input_tokens: number(value, "input_tokens"),
        output_tokens: number(value, "output_tokens"),
        cached_input_tokens: number(value, "cache_read_input_tokens"),
    }
}

fn anthropic_finish_reason(value: Option<&str>) -> FinishReason {
    match value {
        Some("end_turn" | "stop_sequence") => FinishReason::Complete,
        Some("tool_use") => FinishReason::ToolCalls,
        Some("max_tokens" | "model_context_window_exceeded") => FinishReason::Length,
        Some("refusal") => FinishReason::ContentFilter,
        _ => FinishReason::Unknown,
    }
}

#[derive(Default)]
struct ToolBlock {
    id: String,
    name: String,
    initial_input: Value,
    partial_json: String,
    stopped: bool,
}

enum StreamBlock {
    Text { stopped: bool },
    Tool(ToolBlock),
    Ignored { stopped: bool },
}

#[derive(Default)]
struct AnthropicStreamAccumulator {
    message_id: Option<String>,
    text: String,
    blocks: BTreeMap<usize, StreamBlock>,
    finish_reason: Option<FinishReason>,
    usage: Usage,
    usage_seen: bool,
    message_started: bool,
    done: bool,
}

impl AnthropicStreamAccumulator {
    fn apply(
        &mut self,
        event: &SseEvent,
        observer: &dyn ModelStreamObserver,
    ) -> Result<(), ModelError> {
        let value: Value = serde_json::from_str(&event.data).map_err(ModelError::Json)?;
        let kind = value.get("type").and_then(Value::as_str).ok_or_else(|| {
            ModelError::MalformedResponse("Anthropic stream event is missing type".to_owned())
        })?;
        if let Some(named) = event.event.as_deref()
            && named != kind
        {
            return Err(ModelError::MalformedResponse(
                "Anthropic SSE name disagrees with its JSON event type".to_owned(),
            ));
        }
        match kind {
            "error" => Err(ModelError::Provider {
                status: 200,
                message: value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("Anthropic stream error")
                    .chars()
                    .take(1_000)
                    .collect(),
            }),
            "message_start" => self.start_message(&value, observer),
            "content_block_start" => self.start_block(&value, observer),
            "content_block_delta" => self.apply_delta(&value, observer),
            "content_block_stop" => self.stop_block(&value),
            "message_delta" => self.apply_message_delta(&value, observer),
            "message_stop" => {
                if !self.message_started || self.done {
                    return Err(ModelError::MalformedResponse(
                        "Anthropic message_stop was duplicated or out of order".to_owned(),
                    ));
                }
                self.done = true;
                Ok(())
            }
            // Anthropic explicitly reserves the right to add event types.
            _ => Ok(()),
        }
    }

    fn start_message(
        &mut self,
        value: &Value,
        observer: &dyn ModelStreamObserver,
    ) -> Result<(), ModelError> {
        if self.message_started || self.done {
            return Err(ModelError::MalformedResponse(
                "Anthropic message_start was duplicated or out of order".to_owned(),
            ));
        }
        self.message_started = true;
        let message = value.get("message").ok_or_else(|| {
            ModelError::MalformedResponse("message_start is missing message".to_owned())
        })?;
        if let Some(id) = message.get("id") {
            self.message_id = Some(bounded_identifier(Some(id), "Anthropic message id")?);
        }
        if let Some(usage) = message.get("usage") {
            self.merge_usage(anthropic_usage(usage), observer)?;
        }
        Ok(())
    }

    fn start_block(
        &mut self,
        value: &Value,
        observer: &dyn ModelStreamObserver,
    ) -> Result<(), ModelError> {
        if !self.message_started || self.done {
            return Err(ModelError::MalformedResponse(
                "Anthropic content block began outside an active message".to_owned(),
            ));
        }
        let index = stream_index(value)?;
        if index >= MAX_CONTENT_BLOCKS || self.blocks.contains_key(&index) {
            return Err(ModelError::MalformedResponse(
                "Anthropic content block index is duplicate or out of bounds".to_owned(),
            ));
        }
        let block = value.get("content_block").ok_or_else(|| {
            ModelError::MalformedResponse("content_block_start is missing its block".to_owned())
        })?;
        let block = match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                let initial = block
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                append_text(&mut self.text, initial)?;
                if !initial.is_empty() {
                    observer.on_event(&ModelStreamEvent::TextDelta {
                        text: initial.to_owned(),
                    });
                }
                StreamBlock::Text { stopped: false }
            }
            Some("tool_use") => {
                let id = bounded_identifier(block.get("id"), "Anthropic tool-use id")?;
                let name = bounded_identifier(block.get("name"), "Anthropic tool-use name")?;
                let initial_input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                if !initial_input.is_object() {
                    return Err(ModelError::MalformedResponse(
                        "Anthropic streamed tool input must begin as an object".to_owned(),
                    ));
                }
                observer.on_event(&ModelStreamEvent::ToolCallStarted {
                    index,
                    id: id.clone(),
                    name: name.clone(),
                });
                StreamBlock::Tool(ToolBlock {
                    id,
                    name,
                    initial_input,
                    partial_json: String::new(),
                    stopped: false,
                })
            }
            Some("thinking" | "redacted_thinking") => StreamBlock::Ignored { stopped: false },
            Some(other) => {
                return Err(ModelError::MalformedResponse(format!(
                    "unsupported Anthropic streamed content block {other:?}"
                )));
            }
            None => {
                return Err(ModelError::MalformedResponse(
                    "Anthropic streamed content block is missing type".to_owned(),
                ));
            }
        };
        self.blocks.insert(index, block);
        Ok(())
    }

    fn apply_delta(
        &mut self,
        value: &Value,
        observer: &dyn ModelStreamObserver,
    ) -> Result<(), ModelError> {
        let index = stream_index(value)?;
        let block = self.blocks.get_mut(&index).ok_or_else(|| {
            ModelError::MalformedResponse("Anthropic delta has no matching block start".to_owned())
        })?;
        let delta = value.get("delta").ok_or_else(|| {
            ModelError::MalformedResponse("content_block_delta is missing delta".to_owned())
        })?;
        match (block, delta.get("type").and_then(Value::as_str)) {
            (StreamBlock::Text { stopped: false }, Some("text_delta")) => {
                let text = delta.get("text").and_then(Value::as_str).ok_or_else(|| {
                    ModelError::MalformedResponse("text_delta is missing text".to_owned())
                })?;
                append_text(&mut self.text, text)?;
                if !text.is_empty() {
                    observer.on_event(&ModelStreamEvent::TextDelta {
                        text: text.to_owned(),
                    });
                }
                Ok(())
            }
            (StreamBlock::Tool(tool), Some("input_json_delta")) if !tool.stopped => {
                let partial = delta
                    .get("partial_json")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ModelError::MalformedResponse(
                            "input_json_delta is missing partial_json".to_owned(),
                        )
                    })?;
                if tool.partial_json.len().saturating_add(partial.len()) > MAX_TOOL_ARGUMENT_BYTES {
                    return Err(ModelError::ResponseTooLarge {
                        limit: MAX_TOOL_ARGUMENT_BYTES,
                    });
                }
                tool.partial_json.push_str(partial);
                observer.on_event(&ModelStreamEvent::ToolArgumentsDelta {
                    index,
                    bytes: tool.partial_json.len(),
                });
                Ok(())
            }
            (
                StreamBlock::Ignored { stopped: false },
                Some("thinking_delta" | "signature_delta"),
            ) => Ok(()),
            _ => Err(ModelError::MalformedResponse(
                "Anthropic content delta type disagrees with its active block".to_owned(),
            )),
        }
    }

    fn stop_block(&mut self, value: &Value) -> Result<(), ModelError> {
        let index = stream_index(value)?;
        let block = self.blocks.get_mut(&index).ok_or_else(|| {
            ModelError::MalformedResponse("Anthropic block stop has no matching start".to_owned())
        })?;
        let stopped = match block {
            StreamBlock::Text { stopped } | StreamBlock::Ignored { stopped } => stopped,
            StreamBlock::Tool(tool) => &mut tool.stopped,
        };
        if *stopped {
            return Err(ModelError::MalformedResponse(
                "Anthropic content block stopped more than once".to_owned(),
            ));
        }
        *stopped = true;
        Ok(())
    }

    fn apply_message_delta(
        &mut self,
        value: &Value,
        observer: &dyn ModelStreamObserver,
    ) -> Result<(), ModelError> {
        if !self.message_started || self.done {
            return Err(ModelError::MalformedResponse(
                "Anthropic message_delta arrived outside an active message".to_owned(),
            ));
        }
        if let Some(reason) = value
            .get("delta")
            .and_then(|delta| delta.get("stop_reason"))
            .and_then(Value::as_str)
            && self
                .finish_reason
                .replace(anthropic_finish_reason(Some(reason)))
                .is_some()
        {
            return Err(ModelError::MalformedResponse(
                "Anthropic stream emitted more than one stop reason".to_owned(),
            ));
        }
        if let Some(usage) = value.get("usage") {
            let next = Usage {
                input_tokens: self.usage.input_tokens,
                output_tokens: number(usage, "output_tokens"),
                cached_input_tokens: self.usage.cached_input_tokens,
            };
            self.merge_usage(next, observer)?;
        }
        Ok(())
    }

    fn merge_usage(
        &mut self,
        next: Usage,
        observer: &dyn ModelStreamObserver,
    ) -> Result<(), ModelError> {
        if self.usage_seen
            && (next.input_tokens < self.usage.input_tokens
                || next.output_tokens < self.usage.output_tokens
                || next.cached_input_tokens < self.usage.cached_input_tokens)
        {
            return Err(ModelError::MalformedResponse(
                "Anthropic stream usage counters regressed".to_owned(),
            ));
        }
        self.usage = next;
        self.usage_seen = true;
        observer.on_event(&ModelStreamEvent::UsageUpdate { usage: next });
        Ok(())
    }

    fn finish(
        self,
        request_id: Option<String>,
        first_byte_ms: u64,
    ) -> Result<ModelResponse, ModelError> {
        if !self.done {
            return Err(ModelError::MalformedResponse(
                "Anthropic stream disconnected before message_stop".to_owned(),
            ));
        }
        let finish_reason = self.finish_reason.ok_or_else(|| {
            ModelError::MalformedResponse("Anthropic stream is missing a stop reason".to_owned())
        })?;
        let mut tool_calls = Vec::new();
        for (expected, (index, block)) in self.blocks.into_iter().enumerate() {
            if expected != index {
                return Err(ModelError::MalformedResponse(
                    "Anthropic stream content block indexes are not contiguous".to_owned(),
                ));
            }
            match block {
                StreamBlock::Text { stopped: true } | StreamBlock::Ignored { stopped: true } => {}
                StreamBlock::Tool(tool) if tool.stopped => {
                    let arguments = if tool.partial_json.is_empty() {
                        tool.initial_input
                    } else {
                        serde_json::from_str(&tool.partial_json).map_err(ModelError::Json)?
                    };
                    if !arguments.is_object() {
                        return Err(ModelError::MalformedResponse(
                            "Anthropic streamed tool input must be a JSON object".to_owned(),
                        ));
                    }
                    tool_calls.push(ToolCall {
                        id: tool.id,
                        name: tool.name,
                        arguments,
                    });
                }
                _ => {
                    return Err(ModelError::MalformedResponse(
                        "Anthropic stream ended before a content block stopped".to_owned(),
                    ));
                }
            }
        }
        if finish_reason == FinishReason::ToolCalls && tool_calls.is_empty() {
            return Err(ModelError::MalformedResponse(
                "Anthropic stream stopped for tool use but returned no tool call".to_owned(),
            ));
        }
        let mut extensions = serde_json::Map::from_iter([
            ("streaming".to_owned(), Value::Bool(true)),
            (
                "time_to_first_byte_ms".to_owned(),
                Value::from(first_byte_ms),
            ),
        ]);
        if let Some(id) = self.message_id {
            extensions.insert("id".to_owned(), Value::String(id));
        }
        Ok(ModelResponse {
            text: self.text,
            tool_calls,
            finish_reason,
            usage: self.usage,
            provider_request_id: request_id,
            extensions,
        })
    }
}

fn stream_index(value: &Value) -> Result<usize, ModelError> {
    value
        .get("index")
        .and_then(Value::as_u64)
        .and_then(|index| usize::try_from(index).ok())
        .ok_or_else(|| {
            ModelError::MalformedResponse("Anthropic stream event has invalid index".to_owned())
        })
}

async fn accumulate_stream(
    response: reqwest::Response,
    request_id: Option<String>,
    request_started: Instant,
    observer: &dyn ModelStreamObserver,
) -> Result<ModelResponse, ModelError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(ModelError::ResponseTooLarge {
            limit: MAX_RESPONSE_BYTES,
        });
    }
    let mut decoder = SseDecoder::new();
    let mut accumulator = AnthropicStreamAccumulator::default();
    let mut wire_bytes = 0_usize;
    let mut first_byte_ms = None;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(ModelError::Transport)?;
        wire_bytes = wire_bytes.saturating_add(chunk.len());
        if wire_bytes > MAX_RESPONSE_BYTES {
            return Err(ModelError::ResponseTooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
        let elapsed = first_byte_ms.get_or_insert_with(|| {
            u64::try_from(request_started.elapsed().as_millis()).unwrap_or(u64::MAX)
        });
        if wire_bytes == chunk.len() {
            observer.on_event(&ModelStreamEvent::ResponseStarted {
                provider_request_id: request_id.clone(),
                time_to_first_byte_ms: *elapsed,
            });
        }
        for event in decoder
            .push(&chunk)
            .map_err(|error| ModelError::MalformedResponse(error.to_string()))?
        {
            accumulator.apply(&event, observer)?;
        }
    }
    decoder
        .finish()
        .map_err(|error| ModelError::MalformedResponse(error.to_string()))?;
    accumulator.finish(request_id, first_byte_ms.unwrap_or_default())
}

fn emit_complete_response(observer: &dyn ModelStreamObserver, response: &ModelResponse) {
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
}

fn require_event_stream(response: &reqwest::Response) -> Result<(), ModelError> {
    let valid = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|media| media.trim().eq_ignore_ascii_case("text/event-stream"))
        });
    if valid {
        Ok(())
    } else {
        Err(ModelError::MalformedResponse(
            "Anthropic did not return text/event-stream; select buffered mode explicitly"
                .to_owned(),
        ))
    }
}

fn validate_endpoint(base_url: &str) -> Result<(), ModelError> {
    let endpoint = reqwest::Url::parse(base_url).map_err(|error| {
        ModelError::InvalidRequest(format!("invalid Anthropic endpoint: {error}"))
    })?;
    let host = endpoint.host_str().unwrap_or_default();
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if endpoint.scheme() != "https" && !(endpoint.scheme() == "http" && loopback) {
        return Err(ModelError::InvalidRequest(
            "remote Anthropic endpoints must use HTTPS".to_owned(),
        ));
    }
    if !endpoint.username().is_empty() || endpoint.password().is_some() {
        return Err(ModelError::InvalidRequest(
            "provider credentials must not be embedded in the endpoint URL".to_owned(),
        ));
    }
    if endpoint.query().is_some() || endpoint.fragment().is_some() {
        return Err(ModelError::InvalidRequest(
            "provider endpoint must not contain a query or fragment".to_owned(),
        ));
    }
    Ok(())
}

fn parse_retry_after(headers: &HeaderMap, now: SystemTime) -> Option<Duration> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    httpdate::parse_http_date(value)
        .ok()?
        .duration_since(now)
        .ok()
}

fn retry_delay(status: StatusCode, attempt: u32, server: Option<Duration>) -> Duration {
    let fallback = if status == StatusCode::TOO_MANY_REQUESTS {
        RATE_LIMIT_RETRY_DELAY.saturating_mul(2_u32.saturating_pow(attempt.saturating_sub(1)))
    } else {
        MIN_RETRY_DELAY.saturating_mul(2_u32.saturating_pow(attempt.saturating_sub(1)))
    };
    server
        .unwrap_or(fallback)
        .clamp(MIN_RETRY_DELAY, MAX_RETRY_DELAY)
}

async fn read_bounded(response: reqwest::Response) -> Result<Vec<u8>, ModelError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(ModelError::ResponseTooLarge {
            limit: MAX_RESPONSE_BYTES,
        });
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(ModelError::Transport)?;
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(ModelError::ResponseTooLarge {
                limit: MAX_RESPONSE_BYTES,
            });
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn anthropic_error_message(bytes: &[u8]) -> String {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| String::from_utf8_lossy(bytes).chars().take(1_000).collect())
}

fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use pactrail_tools::builtin_registry;

    use super::*;

    #[derive(Default)]
    struct RecordingObserver(Mutex<Vec<ModelStreamEvent>>);

    impl ModelStreamObserver for RecordingObserver {
        fn on_event(&self, event: &ModelStreamEvent) {
            if let Ok(mut events) = self.0.lock() {
                events.push(event.clone());
            }
        }
    }

    fn config() -> AnthropicConfig {
        AnthropicConfig {
            name: "anthropic".to_owned(),
            base_url: "https://api.anthropic.com".to_owned(),
            model: "claude-test".to_owned(),
            api_key: SecretString::from("test-key"),
            timeout: Duration::from_secs(1),
            capabilities: ModelCapabilities {
                parallel_tools: true,
                max_output_tokens: 1_024,
                ..ModelCapabilities::default()
            },
            stream: true,
        }
    }

    fn event(kind: &str, value: &Value) -> SseEvent {
        SseEvent {
            event: Some(kind.to_owned()),
            data: serde_json::to_string(value)
                .unwrap_or_else(|error| unreachable!("fixture JSON: {error}")),
        }
    }

    #[test]
    fn maps_system_tools_and_parallel_results_to_native_blocks() {
        let tools = builtin_registry()
            .unwrap_or_else(|error| unreachable!("registry: {error}"))
            .descriptors();
        let request = ModelRequest {
            conversation: vec![
                ConversationItem::Message(Message::system("policy")),
                ConversationItem::Message(Message::system("context")),
                ConversationItem::Message(Message::user("inspect")),
                ConversationItem::AssistantToolCalls {
                    text: String::new(),
                    calls: vec![ToolCall {
                        id: "call-1".to_owned(),
                        name: "read_file".to_owned(),
                        arguments: json!({"path": "src/lib.rs"}),
                    }],
                },
                ConversationItem::ToolResult(crate::ToolResult {
                    call_id: "call-1".to_owned(),
                    name: "read_file".to_owned(),
                    content: json!({"text": "source"}),
                    is_error: false,
                }),
            ],
            tools,
            max_output_tokens: 512,
            temperature: Some(0.0),
        };
        let body = request_body(&config(), &request, true)
            .unwrap_or_else(|error| unreachable!("native request: {error}"));
        assert_eq!(body["system"], "policy\n\ncontext");
        assert_eq!(body["stream"], true);
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(body["tool_choice"]["disable_parallel_tool_use"], false);
    }

    #[test]
    fn parses_buffered_native_tool_response_and_cache_usage() {
        let response = parse_response(
            &json!({
                "id": "msg-1",
                "type": "message",
                "model": "claude-test",
                "content": [
                    {"type": "text", "text": "checking"},
                    {"type": "tool_use", "id": "call-1", "name": "read_file", "input": {"path": "src/lib.rs"}}
                ],
                "stop_reason": "tool_use",
                "usage": {"input_tokens": 20, "output_tokens": 8, "cache_read_input_tokens": 5}
            }),
            Some("request-1".to_owned()),
        )
        .unwrap_or_else(|error| unreachable!("native response: {error}"));
        assert_eq!(response.text, "checking");
        assert_eq!(response.tool_calls[0].name, "read_file");
        assert_eq!(response.finish_reason, FinishReason::ToolCalls);
        assert_eq!(response.usage.cached_input_tokens, 5);
    }

    #[test]
    fn assembles_partial_json_only_after_the_tool_block_stops() {
        let observer = RecordingObserver::default();
        let mut accumulator = AnthropicStreamAccumulator::default();
        let events = [
            event(
                "message_start",
                &json!({"type": "message_start", "message": {"id": "msg-1", "usage": {"input_tokens": 10, "output_tokens": 0}}}),
            ),
            event(
                "content_block_start",
                &json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": "call-1", "name": "read_file", "input": {}}}),
            ),
            event(
                "content_block_delta",
                &json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": "{\"path\":\"src/"}}),
            ),
            event(
                "content_block_delta",
                &json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": "lib.rs\"}"}}),
            ),
            event(
                "content_block_stop",
                &json!({"type": "content_block_stop", "index": 0}),
            ),
            event(
                "message_delta",
                &json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 6}}),
            ),
            event("message_stop", &json!({"type": "message_stop"})),
        ];
        for item in events {
            accumulator
                .apply(&item, &observer)
                .unwrap_or_else(|error| unreachable!("valid event: {error}"));
        }
        let response = accumulator
            .finish(Some("request-1".to_owned()), 9)
            .unwrap_or_else(|error| unreachable!("complete stream: {error}"));
        assert_eq!(response.tool_calls[0].arguments["path"], "src/lib.rs");
        assert_eq!(response.usage.total(), 16);
        let events = observer
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(events.iter().any(|event| matches!(
            event,
            ModelStreamEvent::ToolArgumentsDelta { bytes, .. } if *bytes > 0
        )));
    }

    #[test]
    fn rejects_named_event_disagreement_and_unfinished_blocks() {
        let observer = RecordingObserver::default();
        let mut accumulator = AnthropicStreamAccumulator::default();
        assert!(matches!(
            accumulator.apply(
                &event("ping", &json!({"type": "message_stop"})),
                &observer
            ),
            Err(ModelError::MalformedResponse(message)) if message.contains("disagrees")
        ));

        let mut incomplete = AnthropicStreamAccumulator {
            message_started: true,
            done: true,
            finish_reason: Some(FinishReason::Complete),
            ..AnthropicStreamAccumulator::default()
        };
        incomplete
            .blocks
            .insert(0, StreamBlock::Text { stopped: false });
        assert!(matches!(
            incomplete.finish(None, 0),
            Err(ModelError::MalformedResponse(message)) if message.contains("before a content block stopped")
        ));
    }
}
