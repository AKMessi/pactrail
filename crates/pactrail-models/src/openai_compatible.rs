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

const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_STREAM_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_TOOL_ARGUMENT_BYTES: usize = 1024 * 1024;
const MAX_TOOL_CALLS: usize = 128;
const MAX_RETRIES: u32 = 3;
const MIN_RETRY_DELAY: Duration = Duration::from_millis(250);
const MAX_RETRY_DELAY: Duration = Duration::from_mins(1);
const RATE_LIMIT_RETRY_DELAY: Duration = Duration::from_secs(15);

/// Configuration for an `OpenAI` Chat Completions compatible endpoint.
#[derive(Clone)]
pub struct OpenAiCompatibleConfig {
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<SecretString>,
    pub timeout: Duration,
    pub capabilities: ModelCapabilities,
    /// Use Chat Completions SSE instead of a buffered JSON response.
    pub stream: bool,
    /// Request the provider's non-thinking mode using the OpenAI-compatible
    /// `thinking.type=disabled` extension. This is opt-in because the field is
    /// not part of the core `OpenAI` Chat Completions schema.
    pub disable_thinking: bool,
}

impl OpenAiCompatibleConfig {
    /// Creates a local Ollama configuration using its OpenAI-compatible endpoint.
    #[must_use]
    pub fn ollama(model: impl Into<String>) -> Self {
        Self {
            name: "ollama".to_owned(),
            base_url: "http://127.0.0.1:11434/v1".to_owned(),
            model: model.into(),
            api_key: None,
            timeout: Duration::from_mins(5),
            capabilities: ModelCapabilities::default(),
            stream: true,
            disable_thinking: false,
        }
    }
}

/// Production driver for Chat Completions compatible API and local endpoints.
pub struct OpenAiCompatibleDriver {
    config: OpenAiCompatibleConfig,
    client: reqwest::Client,
}

impl OpenAiCompatibleDriver {
    /// Builds a driver with the platform TLS backend and a bounded timeout.
    ///
    /// # Errors
    ///
    /// Returns a transport error if the HTTP client cannot be constructed, or
    /// an invalid-request error if the endpoint is not HTTP(S).
    pub fn new(config: OpenAiCompatibleConfig) -> Result<Self, ModelError> {
        let endpoint = reqwest::Url::parse(&config.base_url).map_err(|error| {
            ModelError::InvalidRequest(format!("invalid provider endpoint: {error}"))
        })?;
        let host = endpoint.host_str().unwrap_or_default();
        let loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        if endpoint.scheme() != "https" && !(endpoint.scheme() == "http" && loopback) {
            return Err(ModelError::InvalidRequest(
                "remote model endpoints must use HTTPS".to_owned(),
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
        if config.name.trim().is_empty() || config.model.trim().is_empty() {
            return Err(ModelError::InvalidRequest(
                "provider name and model cannot be empty".to_owned(),
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
        format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        )
    }

    async fn send(&self, body: &Value) -> Result<(Value, Option<String>), ModelError> {
        let mut attempt = 0_u32;
        loop {
            let mut request = self.client.post(self.endpoint()).json(body);
            if let Some(api_key) = &self.config.api_key {
                request = request.bearer_auth(api_key.expose_secret());
            }
            let response = request.send().await.map_err(ModelError::Transport)?;
            let status = response.status();
            let request_id = response
                .headers()
                .get("x-request-id")
                .and_then(|header| header.to_str().ok())
                .map(str::to_owned);
            let server_retry_after = parse_retry_after(response.headers(), SystemTime::now());
            let retryable = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
            let bytes = read_bounded(response, MAX_RESPONSE_BYTES).await?;
            if status.is_success() {
                let value = serde_json::from_slice(&bytes).map_err(ModelError::Json)?;
                return Ok((value, request_id));
            }
            let message = provider_message(&bytes);
            if retryable && attempt < MAX_RETRIES {
                attempt += 1;
                let delay = retry_delay(status, attempt, server_retry_after);
                warn!(
                    attempt,
                    status = status.as_u16(),
                    delay_ms = delay.as_millis(),
                    "retrying model request"
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

    async fn send_stream(
        &self,
        body: &Value,
        observer: &dyn ModelStreamObserver,
    ) -> Result<ModelResponse, ModelError> {
        let request_started = Instant::now();
        let mut attempt = 0_u32;
        loop {
            let mut request = self.client.post(self.endpoint()).json(body);
            if let Some(api_key) = &self.config.api_key {
                request = request.bearer_auth(api_key.expose_secret());
            }
            let response = request.send().await.map_err(ModelError::Transport)?;
            let status = response.status();
            let request_id = response
                .headers()
                .get("x-request-id")
                .and_then(|header| header.to_str().ok())
                .map(str::to_owned);
            let server_retry_after = parse_retry_after(response.headers(), SystemTime::now());
            if status.is_success() {
                let event_stream = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| {
                        value.split(';').next().is_some_and(|media| {
                            media.trim().eq_ignore_ascii_case("text/event-stream")
                        })
                    });
                if !event_stream {
                    return Err(ModelError::MalformedResponse(
                        "provider did not honor streaming with a text/event-stream response; select buffered mode explicitly"
                            .to_owned(),
                    ));
                }
                return accumulate_openai_stream(response, request_id, request_started, observer)
                    .await;
            }
            let retryable = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
            let bytes = read_bounded(response, MAX_RESPONSE_BYTES).await?;
            let message = provider_message(&bytes);
            if retryable && attempt < MAX_RETRIES {
                attempt += 1;
                let delay = retry_delay(status, attempt, server_retry_after);
                warn!(
                    attempt,
                    status = status.as_u16(),
                    delay_ms = delay.as_millis(),
                    "retrying model stream request before response acceptance"
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

fn retry_delay(status: StatusCode, attempt: u32, server_retry_after: Option<Duration>) -> Duration {
    let fallback = if status == StatusCode::TOO_MANY_REQUESTS {
        RATE_LIMIT_RETRY_DELAY.saturating_mul(2_u32.saturating_pow(attempt.saturating_sub(1)))
    } else {
        MIN_RETRY_DELAY.saturating_mul(2_u32.saturating_pow(attempt.saturating_sub(1)))
    };
    server_retry_after
        .unwrap_or(fallback)
        .clamp(MIN_RETRY_DELAY, MAX_RETRY_DELAY)
}

#[async_trait]
impl ModelDriver for OpenAiCompatibleDriver {
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
        let body = request_body(&self.config, request)?;
        let (response, request_id) = self.send(&body).await?;
        parse_response(&response, request_id)
    }

    async fn invoke_with_observer(
        &self,
        request: &ModelRequest,
        observer: &dyn ModelStreamObserver,
    ) -> Result<ModelResponse, ModelError> {
        if !self.config.stream {
            let response = self.invoke(request).await?;
            emit_complete_response(observer, &response);
            return Ok(response);
        }
        let body = request_body(&self.config, request)?;
        self.send_stream(&body, observer).await
    }
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

#[derive(Default)]
struct PartialToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
    announced: bool,
}

#[derive(Default)]
struct OpenAiStreamAccumulator {
    response_id: Option<String>,
    text: String,
    tool_calls: BTreeMap<usize, PartialToolCall>,
    finish_reason: Option<FinishReason>,
    usage: Usage,
    usage_seen: bool,
    done: bool,
}

impl OpenAiStreamAccumulator {
    fn apply(
        &mut self,
        event: &SseEvent,
        observer: &dyn ModelStreamObserver,
    ) -> Result<(), ModelError> {
        if self.done {
            return Err(ModelError::MalformedResponse(
                "OpenAI stream emitted data after [DONE]".to_owned(),
            ));
        }
        if event.data == "[DONE]" {
            self.done = true;
            return Ok(());
        }
        if event.event.as_deref() == Some("error") {
            return Err(ModelError::Provider {
                status: 200,
                message: provider_message(event.data.as_bytes()),
            });
        }
        let value: Value = serde_json::from_str(&event.data).map_err(ModelError::Json)?;
        if value.get("error").is_some() {
            return Err(ModelError::Provider {
                status: 200,
                message: provider_message(event.data.as_bytes()),
            });
        }
        if let Some(id) = value.get("id").and_then(Value::as_str) {
            merge_stable_string(&mut self.response_id, id, "response id")?;
        }
        if let Some(usage) = value.get("usage").filter(|usage| !usage.is_null()) {
            let next = Usage {
                input_tokens: number(usage, "prompt_tokens"),
                output_tokens: number(usage, "completion_tokens"),
                cached_input_tokens: cached_input_tokens(usage),
            };
            if self.usage_seen
                && (next.input_tokens < self.usage.input_tokens
                    || next.output_tokens < self.usage.output_tokens
                    || next.cached_input_tokens < self.usage.cached_input_tokens)
            {
                return Err(ModelError::MalformedResponse(
                    "OpenAI stream usage counters regressed".to_owned(),
                ));
            }
            self.usage = next;
            self.usage_seen = true;
            observer.on_event(&ModelStreamEvent::UsageUpdate { usage: next });
        }
        let choices = value
            .get("choices")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ModelError::MalformedResponse("OpenAI stream chunk is missing choices".to_owned())
            })?;
        if choices.len() > 1 {
            return Err(ModelError::MalformedResponse(
                "OpenAI stream returned more than one choice".to_owned(),
            ));
        }
        let Some(choice) = choices.first() else {
            return Ok(());
        };
        if choice.get("index").and_then(Value::as_u64).unwrap_or(0) != 0 {
            return Err(ModelError::MalformedResponse(
                "OpenAI stream returned a non-zero choice index".to_owned(),
            ));
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            let reason = openai_finish_reason(Some(reason));
            if self.finish_reason.replace(reason).is_some() {
                return Err(ModelError::MalformedResponse(
                    "OpenAI stream emitted more than one finish reason".to_owned(),
                ));
            }
        }
        let Some(delta) = choice.get("delta") else {
            return Ok(());
        };
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            if self.text.len().saturating_add(text.len()) > MAX_STREAM_TEXT_BYTES {
                return Err(ModelError::ResponseTooLarge {
                    limit: MAX_STREAM_TEXT_BYTES,
                });
            }
            self.text.push_str(text);
            if !text.is_empty() {
                observer.on_event(&ModelStreamEvent::TextDelta {
                    text: text.to_owned(),
                });
            }
        }
        let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) else {
            return Ok(());
        };
        for call in calls {
            self.apply_tool_delta(call, observer)?;
        }
        Ok(())
    }

    fn apply_tool_delta(
        &mut self,
        value: &Value,
        observer: &dyn ModelStreamObserver,
    ) -> Result<(), ModelError> {
        let index_u64 = value.get("index").and_then(Value::as_u64).ok_or_else(|| {
            ModelError::MalformedResponse("streamed tool call is missing its index".to_owned())
        })?;
        let index = usize::try_from(index_u64).map_err(|_| {
            ModelError::MalformedResponse("streamed tool call index is too large".to_owned())
        })?;
        if index >= MAX_TOOL_CALLS {
            return Err(ModelError::MalformedResponse(format!(
                "streamed tool call index exceeds the {MAX_TOOL_CALLS}-call limit"
            )));
        }
        let partial = self.tool_calls.entry(index).or_default();
        if let Some(id) = value.get("id").and_then(Value::as_str) {
            merge_stable_string(&mut partial.id, id, "tool call id")?;
        }
        if let Some(function) = value.get("function") {
            if let Some(name) = function.get("name").and_then(Value::as_str) {
                merge_stable_string(&mut partial.name, name, "tool call name")?;
            }
            if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                if partial.arguments.len().saturating_add(arguments.len()) > MAX_TOOL_ARGUMENT_BYTES
                {
                    return Err(ModelError::ResponseTooLarge {
                        limit: MAX_TOOL_ARGUMENT_BYTES,
                    });
                }
                partial.arguments.push_str(arguments);
                observer.on_event(&ModelStreamEvent::ToolArgumentsDelta {
                    index,
                    bytes: partial.arguments.len(),
                });
            }
        }
        if !partial.announced
            && let (Some(id), Some(name)) = (&partial.id, &partial.name)
        {
            observer.on_event(&ModelStreamEvent::ToolCallStarted {
                index,
                id: id.clone(),
                name: name.clone(),
            });
            partial.announced = true;
        }
        Ok(())
    }

    fn finish(
        self,
        request_id: Option<String>,
        first_byte_ms: u64,
    ) -> Result<ModelResponse, ModelError> {
        if !self.done {
            return Err(ModelError::MalformedResponse(
                "OpenAI stream disconnected before [DONE]".to_owned(),
            ));
        }
        let finish_reason = self.finish_reason.ok_or_else(|| {
            ModelError::MalformedResponse("OpenAI stream is missing a finish reason".to_owned())
        })?;
        let mut tool_calls = Vec::with_capacity(self.tool_calls.len());
        for (expected, (index, call)) in self.tool_calls.into_iter().enumerate() {
            if index != expected {
                return Err(ModelError::MalformedResponse(
                    "OpenAI stream tool call indexes are not contiguous".to_owned(),
                ));
            }
            let id = call.id.ok_or_else(|| {
                ModelError::MalformedResponse("streamed tool call is missing its id".to_owned())
            })?;
            let name = call.name.ok_or_else(|| {
                ModelError::MalformedResponse("streamed tool call is missing its name".to_owned())
            })?;
            let arguments: Value =
                serde_json::from_str(&call.arguments).map_err(ModelError::Json)?;
            if !arguments.is_object() {
                return Err(ModelError::MalformedResponse(
                    "streamed tool call arguments must be a JSON object".to_owned(),
                ));
            }
            tool_calls.push(ToolCall {
                id,
                name,
                arguments,
            });
        }
        if finish_reason == FinishReason::ToolCalls && tool_calls.is_empty() {
            return Err(ModelError::MalformedResponse(
                "OpenAI stream stopped for tool calls but returned none".to_owned(),
            ));
        }
        let mut extensions = serde_json::Map::from_iter([
            ("streaming".to_owned(), Value::Bool(true)),
            (
                "time_to_first_byte_ms".to_owned(),
                Value::from(first_byte_ms),
            ),
        ]);
        if let Some(id) = self.response_id {
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

fn merge_stable_string(
    target: &mut Option<String>,
    value: &str,
    field: &str,
) -> Result<(), ModelError> {
    if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(ModelError::MalformedResponse(format!(
            "streamed {field} is empty, oversized, or contains control characters"
        )));
    }
    match target {
        Some(existing) if existing != value => Err(ModelError::MalformedResponse(format!(
            "streamed {field} changed during the response"
        ))),
        Some(_) => Ok(()),
        None => {
            *target = Some(value.to_owned());
            Ok(())
        }
    }
}

async fn accumulate_openai_stream(
    response: reqwest::Response,
    request_id: Option<String>,
    request_started: Instant,
    observer: &dyn ModelStreamObserver,
) -> Result<ModelResponse, ModelError> {
    if response.content_length().is_some_and(|length| {
        usize::try_from(length).map_or(true, |length| length > MAX_RESPONSE_BYTES)
    }) {
        return Err(ModelError::ResponseTooLarge {
            limit: MAX_RESPONSE_BYTES,
        });
    }
    let mut decoder = SseDecoder::new();
    let mut accumulator = OpenAiStreamAccumulator::default();
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

fn openai_finish_reason(reason: Option<&str>) -> FinishReason {
    match reason {
        Some("stop") => FinishReason::Complete,
        Some("tool_calls" | "function_call") => FinishReason::ToolCalls,
        Some("length") => FinishReason::Length,
        Some("content_filter") => FinishReason::ContentFilter,
        _ => FinishReason::Unknown,
    }
}

fn request_body(
    config: &OpenAiCompatibleConfig,
    request: &ModelRequest,
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
    let messages = canonical_messages(&request.conversation)?;
    let tools = request
        .tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                }
            })
        })
        .collect::<Vec<_>>();
    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "max_tokens": request.max_output_tokens,
        "stream": config.stream,
    });
    if config.stream {
        body["stream_options"] = json!({ "include_usage": true });
    }
    if config.disable_thinking {
        body["thinking"] = json!({ "type": "disabled" });
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
        body["tool_choice"] = Value::String("auto".to_owned());
    }
    if let Some(temperature) = request.temperature {
        if !(0.0..=2.0).contains(&temperature) {
            return Err(ModelError::InvalidRequest(
                "temperature must be between 0 and 2".to_owned(),
            ));
        }
        body["temperature"] = json!(temperature);
    }

    Ok(body)
}

fn message_json(message: &Message) -> Value {
    let role = match message.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    json!({ "role": role, "content": message.content })
}

fn canonical_messages(conversation: &[ConversationItem]) -> Result<Vec<Value>, ModelError> {
    let mut system_instructions = Vec::new();
    let mut messages = Vec::with_capacity(conversation.len());
    for item in conversation {
        if let ConversationItem::Message(Message {
            role: Role::System,
            content,
        }) = item
        {
            system_instructions.push(content.as_str());
        } else {
            messages.push(conversation_json(item)?);
        }
    }

    if !system_instructions.is_empty() {
        messages.insert(
            0,
            json!({
                "role": "system",
                "content": system_instructions.join("\n\n"),
            }),
        );
    }
    Ok(messages)
}

fn conversation_json(item: &ConversationItem) -> Result<Value, ModelError> {
    match item {
        ConversationItem::Message(message) => Ok(message_json(message)),
        ConversationItem::AssistantToolCalls { text, calls } => {
            let calls = calls
                .iter()
                .map(|call| {
                    Ok(json!({
                        "id": call.id,
                        "type": "function",
                        "function": {
                            "name": call.name,
                            "arguments": serde_json::to_string(&call.arguments)
                                .map_err(ModelError::Json)?,
                        }
                    }))
                })
                .collect::<Result<Vec<Value>, ModelError>>()?;
            Ok(json!({
                "role": "assistant",
                "content": text,
                "tool_calls": calls,
            }))
        }
        ConversationItem::ToolResult(result) => Ok(json!({
            "role": "tool",
            "tool_call_id": result.call_id,
            "name": result.name,
            "content": serde_json::to_string(&result.content).map_err(ModelError::Json)?,
        })),
    }
}

fn parse_response(value: &Value, request_id: Option<String>) -> Result<ModelResponse, ModelError> {
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| ModelError::MalformedResponse("missing choices[0]".to_owned()))?;
    let message = choice
        .get("message")
        .ok_or_else(|| ModelError::MalformedResponse("missing response message".to_owned()))?;
    let text = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| calls.iter().map(parse_tool_call).collect())
        .transpose()?
        .unwrap_or_default();
    let finish_reason = openai_finish_reason(choice.get("finish_reason").and_then(Value::as_str));
    let usage = value
        .get("usage")
        .map_or_else(Usage::default, |usage| Usage {
            input_tokens: number(usage, "prompt_tokens"),
            output_tokens: number(usage, "completion_tokens"),
            cached_input_tokens: cached_input_tokens(usage),
        });
    let mut extensions = serde_json::Map::new();
    for key in ["id", "created", "system_fingerprint"] {
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

fn cached_input_tokens(usage: &Value) -> u64 {
    usage
        .get("prompt_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or_else(|| number(usage, "prompt_cache_hit_tokens"))
}

fn parse_tool_call(value: &Value) -> Result<ToolCall, ModelError> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| ModelError::MalformedResponse("tool call is missing id".to_owned()))?;
    let function = value
        .get("function")
        .ok_or_else(|| ModelError::MalformedResponse("tool call is missing function".to_owned()))?;
    let name = function
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| ModelError::MalformedResponse("tool call is missing name".to_owned()))?;
    let arguments = function
        .get("arguments")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ModelError::MalformedResponse("tool call is missing arguments".to_owned())
        })?;
    let arguments = serde_json::from_str(arguments).map_err(ModelError::Json)?;
    Ok(ToolCall {
        id: id.to_owned(),
        name: name.to_owned(),
        arguments,
    })
}

fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or_default()
}

async fn read_bounded(response: reqwest::Response, limit: usize) -> Result<Vec<u8>, ModelError> {
    if response
        .content_length()
        .is_some_and(|length| usize::try_from(length).map_or(true, |length| length > limit))
    {
        return Err(ModelError::ResponseTooLarge { limit });
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(ModelError::Transport)?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(ModelError::ResponseTooLarge { limit });
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn provider_message(bytes: &[u8]) -> String {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| {
            let text = String::from_utf8_lossy(bytes);
            text.chars().take(1_000).collect()
        })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

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

    fn sse(data: &Value) -> SseEvent {
        SseEvent {
            event: None,
            data: serde_json::to_string(data)
                .unwrap_or_else(|error| unreachable!("fixture JSON: {error}")),
        }
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> std::io::Result<Vec<u8>> {
        use std::io::{Error, ErrorKind, Read};

        const LIMIT: usize = 64 * 1024;
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4_096];
        loop {
            let read = stream.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            if request.len().saturating_add(read) > LIMIT {
                return Err(Error::new(ErrorKind::InvalidData, "request too large"));
            }
            request.extend_from_slice(&buffer[..read]);
            let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
            else {
                continue;
            };
            let header_bytes = header_end.saturating_add(4);
            let headers = std::str::from_utf8(&request[..header_end])
                .map_err(|_| Error::new(ErrorKind::InvalidData, "invalid request headers"))?;
            let content_length = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            });
            if content_length.is_none_or(|length| request.len() >= header_bytes + length) {
                return Ok(request);
            }
        }
        Ok(request)
    }

    fn config(base_url: &str) -> OpenAiCompatibleConfig {
        OpenAiCompatibleConfig {
            name: "test".to_owned(),
            base_url: base_url.to_owned(),
            model: "model".to_owned(),
            api_key: None,
            timeout: Duration::from_secs(1),
            capabilities: ModelCapabilities::default(),
            stream: false,
            disable_thinking: false,
        }
    }

    #[test]
    fn parses_text_tools_usage_and_extensions() {
        let value = json!({
            "id": "response-1",
            "choices": [{
                "message": {
                    "content": "I will inspect it.",
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": {"name": "read_file", "arguments": "{\"path\":\"src/lib.rs\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 4,
                "prompt_cache_hit_tokens": 6,
                "prompt_cache_miss_tokens": 4
            }
        });
        let response = parse_response(&value, Some("request-1".to_owned()))
            .unwrap_or_else(|error| unreachable!("valid response: {error}"));
        assert_eq!(response.tool_calls[0].name, "read_file");
        assert_eq!(response.usage.total(), 14);
        assert_eq!(response.usage.cached_input_tokens, 6);
        assert_eq!(response.finish_reason, FinishReason::ToolCalls);
        assert_eq!(response.extensions["id"], "response-1");
    }

    #[test]
    fn standard_cached_token_detail_takes_precedence_over_provider_extension() {
        let usage = json!({
            "prompt_tokens_details": { "cached_tokens": 7 },
            "prompt_cache_hit_tokens": 6
        });
        assert_eq!(cached_input_tokens(&usage), 7);
    }

    #[test]
    fn coalesces_all_system_instructions_into_one_leading_message() {
        let conversation = vec![
            ConversationItem::Message(Message::system("base policy")),
            ConversationItem::Message(Message::system("repository context")),
            ConversationItem::Message(Message::user("perform the task")),
            ConversationItem::Message(Message::assistant("working")),
            ConversationItem::Message(Message::system("recovery instruction")),
        ];

        let messages = canonical_messages(&conversation)
            .unwrap_or_else(|error| unreachable!("valid conversation: {error}"));

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(
            messages[0]["content"],
            "base policy\n\nrepository context\n\nrecovery instruction"
        );
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["role"] == "system")
                .count(),
            1
        );
    }

    #[test]
    fn preserves_conversations_without_system_instructions() {
        let conversation = vec![
            ConversationItem::Message(Message::user("question")),
            ConversationItem::Message(Message::assistant("answer")),
        ];

        let messages = canonical_messages(&conversation)
            .unwrap_or_else(|error| unreachable!("valid conversation: {error}"));

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[1]["role"], "assistant");
    }

    #[test]
    fn non_thinking_extension_is_explicit_in_request_body() {
        let request = ModelRequest {
            conversation: vec![ConversationItem::Message(Message::user("task"))],
            tools: Vec::new(),
            max_output_tokens: 128,
            temperature: Some(0.0),
        };
        let default_body = request_body(&config("https://api.example.com/v1"), &request)
            .unwrap_or_else(|error| unreachable!("valid request: {error}"));
        assert!(default_body.get("thinking").is_none());

        let mut non_thinking = config("https://api.example.com/v1");
        non_thinking.disable_thinking = true;
        let body = request_body(&non_thinking, &request)
            .unwrap_or_else(|error| unreachable!("valid request: {error}"));
        assert_eq!(body["thinking"]["type"], "disabled");
    }

    #[test]
    fn streaming_request_explicitly_asks_for_usage() {
        let request = ModelRequest {
            conversation: vec![ConversationItem::Message(Message::user("task"))],
            tools: Vec::new(),
            max_output_tokens: 128,
            temperature: None,
        };
        let mut streaming = config("https://api.example.com/v1");
        streaming.stream = true;
        let body = request_body(&streaming, &request)
            .unwrap_or_else(|error| unreachable!("valid request: {error}"));
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn accumulates_parallel_text_tool_arguments_and_cumulative_usage() {
        let observer = RecordingObserver::default();
        let mut accumulator = OpenAiStreamAccumulator::default();
        for event in [
            sse(&json!({
                "id": "response-1",
                "choices": [{"index": 0, "delta": {"content": "Inspecting "}}]
            })),
            sse(&json!({
                "id": "response-1",
                "choices": [{"index": 0, "delta": {"tool_calls": [{
                    "index": 0,
                    "id": "call-1",
                    "function": {"name": "read_file", "arguments": "{\"path\":"}
                }]}}]
            })),
            sse(&json!({
                "id": "response-1",
                "choices": [{"index": 0, "delta": {"tool_calls": [{
                    "index": 0,
                    "function": {"arguments": "\"src/lib.rs\"}"}
                }]}}]
            })),
            sse(&json!({
                "id": "response-1",
                "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]
            })),
            sse(&json!({
                "id": "response-1",
                "choices": [],
                "usage": {
                    "prompt_tokens": 20,
                    "completion_tokens": 8,
                    "prompt_tokens_details": {"cached_tokens": 5}
                }
            })),
            SseEvent {
                event: None,
                data: "[DONE]".to_owned(),
            },
        ] {
            accumulator
                .apply(&event, &observer)
                .unwrap_or_else(|error| unreachable!("valid chunk: {error}"));
        }
        let response = accumulator
            .finish(Some("request-1".to_owned()), 12)
            .unwrap_or_else(|error| unreachable!("complete stream: {error}"));
        assert_eq!(response.text, "Inspecting ");
        assert_eq!(response.finish_reason, FinishReason::ToolCalls);
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "read_file");
        assert_eq!(response.tool_calls[0].arguments["path"], "src/lib.rs");
        assert_eq!(response.usage.total(), 28);
        assert_eq!(response.usage.cached_input_tokens, 5);
        assert_eq!(response.extensions["time_to_first_byte_ms"], 12);
        let events = observer
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(events.iter().any(|event| matches!(
            event,
            ModelStreamEvent::ToolCallStarted { name, .. } if name == "read_file"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ModelStreamEvent::ToolArgumentsDelta { bytes, .. } if *bytes > 0
        )));
    }

    #[test]
    fn stream_rejects_usage_regression_and_disconnect_without_done() {
        let observer = RecordingObserver::default();
        let mut accumulator = OpenAiStreamAccumulator::default();
        accumulator
            .apply(
                &sse(&json!({
                    "choices": [],
                    "usage": {"prompt_tokens": 10, "completion_tokens": 4}
                })),
                &observer,
            )
            .unwrap_or_else(|error| unreachable!("first usage: {error}"));
        assert!(matches!(
            accumulator.apply(
                &sse(&json!({
                    "choices": [],
                    "usage": {"prompt_tokens": 9, "completion_tokens": 4}
                })),
                &observer
            ),
            Err(ModelError::MalformedResponse(message)) if message.contains("regressed")
        ));
        assert!(matches!(
            OpenAiStreamAccumulator::default().finish(None, 0),
            Err(ModelError::MalformedResponse(message)) if message.contains("disconnected")
        ));
    }

    #[test]
    fn remote_plain_http_is_rejected() {
        assert!(matches!(
            OpenAiCompatibleDriver::new(config("http://example.com/v1")),
            Err(ModelError::InvalidRequest(_))
        ));
    }

    #[test]
    fn localhost_prefix_confusion_is_rejected() {
        assert!(matches!(
            OpenAiCompatibleDriver::new(config("http://localhost.evil.example/v1")),
            Err(ModelError::InvalidRequest(_))
        ));
    }

    #[test]
    fn ambiguous_endpoint_components_are_rejected() {
        for endpoint in [
            "https://api.example.com/v1?tenant=other",
            "https://api.example.com/v1#fragment",
        ] {
            assert!(matches!(
                OpenAiCompatibleDriver::new(config(endpoint)),
                Err(ModelError::InvalidRequest(_))
            ));
        }
    }

    #[test]
    fn zero_timeout_is_rejected() {
        let mut config = config("https://api.example.com/v1");
        config.timeout = Duration::ZERO;
        assert!(matches!(
            OpenAiCompatibleDriver::new(config),
            Err(ModelError::InvalidRequest(_))
        ));
    }

    #[test]
    fn retry_after_delta_is_honored_and_bounded() {
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            "7".parse()
                .unwrap_or_else(|error| unreachable!("static header value must parse: {error}")),
        );
        assert_eq!(
            parse_retry_after(&headers, SystemTime::UNIX_EPOCH),
            Some(Duration::from_secs(7))
        );
        assert_eq!(
            retry_delay(
                StatusCode::TOO_MANY_REQUESTS,
                1,
                Some(Duration::from_mins(10))
            ),
            MAX_RETRY_DELAY
        );
    }

    #[test]
    fn retry_fallback_distinguishes_rate_limits_from_server_errors() {
        assert_eq!(
            retry_delay(StatusCode::TOO_MANY_REQUESTS, 2, None),
            Duration::from_secs(30)
        );
        assert_eq!(
            retry_delay(StatusCode::SERVICE_UNAVAILABLE, 2, None),
            Duration::from_millis(500)
        );
    }

    #[tokio::test]
    async fn redirects_are_not_followed() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };

        let listener = TcpListener::bind("127.0.0.1:0")
            .unwrap_or_else(|error| unreachable!("test listener must bind: {error}"));
        let address = listener
            .local_addr()
            .unwrap_or_else(|error| unreachable!("test listener needs an address: {error}"));
        let server = std::thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            let mut request = [0_u8; 4_096];
            let _ = stream.read(&mut request)?;
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://{address}/redirected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(response.as_bytes())?;
            Ok(())
        });

        let driver = OpenAiCompatibleDriver::new(config(&format!("http://{address}/v1")))
            .unwrap_or_else(|error| unreachable!("loopback endpoint must be valid: {error}"));
        let result = driver.send(&json!({})).await;

        assert!(matches!(
            result,
            Err(ModelError::Provider { status: 302, .. })
        ));
        assert!(matches!(server.join(), Ok(Ok(()))));
    }

    #[tokio::test]
    async fn streamed_http_response_is_normalized_without_buffered_fallback() {
        use std::{io::Write, net::TcpListener};

        let listener = TcpListener::bind("127.0.0.1:0")
            .unwrap_or_else(|error| unreachable!("test listener must bind: {error}"));
        let address = listener
            .local_addr()
            .unwrap_or_else(|error| unreachable!("test listener needs an address: {error}"));
        let server = std::thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            let request = read_http_request(&mut stream)?;
            let request = String::from_utf8_lossy(&request);
            assert!(request.contains("\"stream\":true"));
            let body = concat!(
                "data: {\"id\":\"response-1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\"}}]}\n\n",
                "data: {\"id\":\"response-1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: {\"id\":\"response-1\",\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":1}}\n\n",
                "data: [DONE]\n\n"
            );
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nContent-Length: {}\r\nX-Request-Id: request-1\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(headers.as_bytes())?;
            for byte in body.as_bytes() {
                stream.write_all(std::slice::from_ref(byte))?;
            }
            Ok(())
        });

        let mut stream_config = config(&format!("http://{address}/v1"));
        stream_config.stream = true;
        let driver = OpenAiCompatibleDriver::new(stream_config)
            .unwrap_or_else(|error| unreachable!("loopback endpoint: {error}"));
        let observer = RecordingObserver::default();
        let response = driver
            .invoke_with_observer(
                &ModelRequest {
                    conversation: vec![ConversationItem::Message(Message::user("hello"))],
                    tools: Vec::new(),
                    max_output_tokens: 32,
                    temperature: None,
                },
                &observer,
            )
            .await
            .unwrap_or_else(|error| unreachable!("valid stream: {error}"));
        assert_eq!(response.text, "hello");
        assert_eq!(response.usage.total(), 4);
        assert_eq!(response.provider_request_id.as_deref(), Some("request-1"));
        assert!(matches!(server.join(), Ok(Ok(()))));
    }
}
