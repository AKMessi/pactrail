use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{StatusCode, header::HeaderMap};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use tracing::warn;

use crate::{
    ConversationItem, FinishReason, Message, ModelCapabilities, ModelDriver, ModelError,
    ModelRequest, ModelResponse, Role, ToolCall, Usage,
};

const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
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
        "stream": false,
    });
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
    let finish_reason = match choice.get("finish_reason").and_then(Value::as_str) {
        Some("stop") => FinishReason::Complete,
        Some("tool_calls" | "function_call") => FinishReason::ToolCalls,
        Some("length") => FinishReason::Length,
        Some("content_filter") => FinishReason::ContentFilter,
        _ => FinishReason::Unknown,
    };
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
    use super::*;

    fn config(base_url: &str) -> OpenAiCompatibleConfig {
        OpenAiCompatibleConfig {
            name: "test".to_owned(),
            base_url: base_url.to_owned(),
            model: "model".to_owned(),
            api_key: None,
            timeout: Duration::from_secs(1),
            capabilities: ModelCapabilities::default(),
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
}
