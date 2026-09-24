use std::time::Duration;

use async_trait::async_trait;
use reqwest::StatusCode;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value, json};

use crate::types::{validate_request_body_size, validate_request_images};
use crate::{
    ConversationItem, FinishReason, ModelCapabilities, ModelDriver, ModelError, ModelRequest,
    ModelResponse, Role, ToolCall, Usage,
};

const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_ARGUMENT_BYTES: usize = 1024 * 1024;
const MAX_TOOL_CALLS: usize = 128;
const REPLAY_KEY: &str = "openai_responses_output_items";

/// Configuration for the native `OpenAI` Responses endpoint.
#[derive(Clone)]
pub struct OpenAiResponsesConfig {
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub api_key: SecretString,
    pub timeout: Duration,
    pub capabilities: ModelCapabilities,
}

/// Stateless Responses adapter with durable, exact output-item replay.
pub struct OpenAiResponsesDriver {
    config: OpenAiResponsesConfig,
    client: reqwest::Client,
}

impl OpenAiResponsesDriver {
    /// Validates the endpoint and constructs a bounded HTTP client.
    ///
    /// # Errors
    /// Returns an invalid-request or transport error for an unsafe configuration.
    pub fn new(config: OpenAiResponsesConfig) -> Result<Self, ModelError> {
        let endpoint = reqwest::Url::parse(&config.base_url).map_err(|error| {
            ModelError::InvalidRequest(format!("invalid Responses endpoint: {error}"))
        })?;
        let host = endpoint.host_str().unwrap_or_default();
        let loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        if !(endpoint.scheme() == "https" || (endpoint.scheme() == "http" && loopback))
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(ModelError::InvalidRequest(
                "Responses endpoint must be credential-free HTTPS or loopback HTTP".to_owned(),
            ));
        }
        if config.model.trim().is_empty() || config.name.trim().is_empty() {
            return Err(ModelError::InvalidRequest(
                "provider and model names cannot be empty".to_owned(),
            ));
        }
        if config.api_key.expose_secret().is_empty() || config.timeout.is_zero() {
            return Err(ModelError::InvalidRequest(
                "Responses requires a nonempty API key and positive timeout".to_owned(),
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
        format!("{}/responses", self.config.base_url.trim_end_matches('/'))
    }
}

#[async_trait]
impl ModelDriver for OpenAiResponsesDriver {
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
        let mut response = self
            .client
            .post(self.endpoint())
            .bearer_auth(self.config.api_key.expose_secret())
            .json(&body)
            .send()
            .await
            .map_err(ModelError::Transport)?;
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let status = response.status();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(ModelError::Transport)? {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(ModelError::ResponseTooLarge {
                    limit: MAX_RESPONSE_BYTES,
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        if status != StatusCode::OK {
            let message = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|value| {
                    value
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| String::from_utf8_lossy(&bytes).chars().take(512).collect());
            return Err(ModelError::Provider {
                status: status.as_u16(),
                message,
            });
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(ModelError::Json)?;
        parse_response(&value, request_id)
    }
}

fn request_body(
    config: &OpenAiResponsesConfig,
    request: &ModelRequest,
) -> Result<Value, ModelError> {
    if request.max_output_tokens == 0
        || request.max_output_tokens > config.capabilities.max_output_tokens
    {
        return Err(ModelError::InvalidRequest(
            "max_output_tokens is outside the configured model capacity".to_owned(),
        ));
    }
    validate_request_images(&request.conversation, config.capabilities.vision)
        .map_err(ModelError::InvalidRequest)?;
    let input = input_items(&request.conversation)?;
    let tools = request
        .tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.input_schema,
            })
        })
        .collect::<Vec<_>>();
    let mut body = json!({
        "model": config.model,
        "input": input,
        "max_output_tokens": request.max_output_tokens,
        "store": false,
        "stream": false,
        "truncation": "disabled",
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
        body["tool_choice"] = Value::String("auto".to_owned());
        body["parallel_tool_calls"] = Value::Bool(config.capabilities.parallel_tools);
    }
    validate_request_body_size(&body).map_err(ModelError::InvalidRequest)?;
    Ok(body)
}

fn input_items(conversation: &[ConversationItem]) -> Result<Vec<Value>, ModelError> {
    let mut input = Vec::new();
    let mut systems = Vec::new();
    for item in conversation {
        match item {
            ConversationItem::Message(message) => {
                let role = match message.role {
                    Role::System => {
                        systems.push(message.content.as_str());
                        continue;
                    }
                    Role::User => "user",
                    Role::Assistant => "assistant",
                };
                input.push(json!({"role": role, "content": message.content}));
            }
            ConversationItem::UserContent(content) => {
                let mut parts = vec![json!({"type": "input_text", "text": content.text})];
                for image in &content.images {
                    parts.push(json!({
                        "type": "input_text",
                        "text": format!("Attached image: {}", image.name()),
                    }));
                    parts.push(json!({
                        "type": "input_image",
                        "image_url": format!(
                            "data:{};base64,{}",
                            image.media_type().as_str(),
                            image.data_base64()
                        ),
                    }));
                }
                input.push(json!({"role": "user", "content": parts}));
            }
            ConversationItem::AssistantToolCalls { text, calls } => {
                if let Some(items) = calls
                    .first()
                    .and_then(|call| call.extensions.get(REPLAY_KEY))
                {
                    let items = items.as_array().ok_or_else(|| {
                        ModelError::MalformedResponse(
                            "Responses replay items are invalid".to_owned(),
                        )
                    })?;
                    input.extend(items.iter().cloned());
                } else {
                    if !text.is_empty() {
                        input.push(json!({"role": "assistant", "content": text}));
                    }
                    for call in calls {
                        input.push(json!({
                            "type": "function_call",
                            "call_id": call.id,
                            "name": call.name,
                            "arguments": serde_json::to_string(&call.arguments).map_err(ModelError::Json)?,
                        }));
                    }
                }
            }
            ConversationItem::ToolResult(result) => input.push(json!({
                "type": "function_call_output",
                "call_id": result.call_id,
                "output": serde_json::to_string(&result.content).map_err(ModelError::Json)?,
            })),
        }
    }
    if !systems.is_empty() {
        input.insert(
            0,
            json!({"role": "system", "content": systems.join("\n\n")}),
        );
    }
    Ok(input)
}

fn parse_response(value: &Value, request_id: Option<String>) -> Result<ModelResponse, ModelError> {
    if value.get("object").and_then(Value::as_str) != Some("response") {
        return Err(ModelError::MalformedResponse(
            "Responses object type is missing".to_owned(),
        ));
    }
    let output = value
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| ModelError::MalformedResponse("Responses output is missing".to_owned()))?;
    let status = value.get("status").and_then(Value::as_str);
    let mut finish_reason = match status {
        Some("completed") => FinishReason::Complete,
        Some("incomplete") => match value
            .pointer("/incomplete_details/reason")
            .and_then(Value::as_str)
        {
            Some("max_output_tokens") => FinishReason::Length,
            Some("content_filter") => FinishReason::ContentFilter,
            _ => FinishReason::Unknown,
        },
        _ => {
            return Err(ModelError::MalformedResponse(format!(
                "Responses status {status:?} is not terminal"
            )));
        }
    };
    let (text, mut calls, refused) = parse_output_items(output)?;
    if refused {
        finish_reason = FinishReason::ContentFilter;
    }
    if !calls.is_empty() {
        if refused {
            return Err(ModelError::MalformedResponse(
                "refused Responses output also contained tool calls".to_owned(),
            ));
        }
        if status != Some("completed") {
            return Err(ModelError::MalformedResponse(
                "incomplete Responses tool calls cannot be executed".to_owned(),
            ));
        }
        calls[0]
            .extensions
            .insert(REPLAY_KEY.to_owned(), Value::Array(output.clone()));
        finish_reason = FinishReason::ToolCalls;
    }
    let usage_value = value.get("usage");
    let usage = usage_value.map_or_else(Usage::default, |usage| Usage {
        input_tokens: number(usage, "input_tokens"),
        output_tokens: number(usage, "output_tokens"),
        cached_input_tokens: usage
            .pointer("/input_tokens_details/cached_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_creation_input_tokens: usage
            .pointer("/input_tokens_details/cache_write_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    });
    let mut extensions = Map::new();
    if let Some(id) = value.get("id") {
        extensions.insert("response_id".to_owned(), id.clone());
    }
    Ok(ModelResponse {
        text,
        tool_calls: calls,
        finish_reason,
        usage,
        provider_request_id: request_id,
        extensions,
    })
}

fn parse_output_items(output: &[Value]) -> Result<(String, Vec<ToolCall>, bool), ModelError> {
    let mut text = String::new();
    let mut calls = Vec::new();
    let mut refused = false;
    for item in output {
        match item.get("type").and_then(Value::as_str) {
            Some("reasoning") => {}
            Some("message") => {
                let parts = item
                    .get("content")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        ModelError::MalformedResponse(
                            "Responses message content is missing".to_owned(),
                        )
                    })?;
                for part in parts {
                    match part.get("type").and_then(Value::as_str) {
                        Some("output_text") => {
                            let fragment =
                                part.get("text").and_then(Value::as_str).ok_or_else(|| {
                                    ModelError::MalformedResponse(
                                        "Responses text is missing".to_owned(),
                                    )
                                })?;
                            if text.len().saturating_add(fragment.len()) > MAX_TEXT_BYTES {
                                return Err(ModelError::ResponseTooLarge {
                                    limit: MAX_TEXT_BYTES,
                                });
                            }
                            text.push_str(fragment);
                        }
                        Some("refusal") => refused = true,
                        _ => {
                            return Err(ModelError::MalformedResponse(
                                "unsupported Responses content part".to_owned(),
                            ));
                        }
                    }
                }
            }
            Some("function_call") => {
                if calls.len() >= MAX_TOOL_CALLS {
                    return Err(ModelError::MalformedResponse(
                        "too many Responses tool calls".to_owned(),
                    ));
                }
                let id = required_string(item, "call_id")?;
                let name = required_string(item, "name")?;
                let arguments = required_string(item, "arguments")?;
                if arguments.len() > MAX_ARGUMENT_BYTES {
                    return Err(ModelError::ResponseTooLarge {
                        limit: MAX_ARGUMENT_BYTES,
                    });
                }
                let arguments: Value = serde_json::from_str(arguments).map_err(ModelError::Json)?;
                if !arguments.is_object() {
                    return Err(ModelError::MalformedResponse(
                        "Responses tool arguments must be an object".to_owned(),
                    ));
                }
                calls.push(ToolCall {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    arguments,
                    extensions: Map::new(),
                });
            }
            _ => {
                return Err(ModelError::MalformedResponse(
                    "unsupported Responses output item".to_owned(),
                ));
            }
        }
    }
    Ok((text, calls, refused))
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, ModelError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|item| !item.is_empty())
        .ok_or_else(|| ModelError::MalformedResponse(format!("Responses {key} is missing")))
}

fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Message, ToolResult};

    fn config() -> OpenAiResponsesConfig {
        OpenAiResponsesConfig {
            name: "openai-responses".to_owned(),
            base_url: "https://api.openai.com/v1".to_owned(),
            model: "test-model".to_owned(),
            api_key: SecretString::from("test".to_owned()),
            timeout: Duration::from_secs(30),
            capabilities: ModelCapabilities::default(),
        }
    }

    #[test]
    fn stateless_request_replays_exact_reasoning_and_function_outputs() {
        let response = parse_response(
            &json!({
                "object": "response", "id": "resp-1", "status": "completed",
                "output": [
                    {"type": "reasoning", "id": "reason-1", "encrypted_content": "sealed"},
                    {"type": "function_call", "call_id": "call-1", "name": "read_file", "arguments": "{\"path\":\"src/lib.rs\"}"}
                ],
                "usage": {"input_tokens": 20, "output_tokens": 5,
                    "input_tokens_details": {"cached_tokens": 4, "cache_write_tokens": 2}}
            }),
            None,
        )
        .unwrap_or_else(|error| unreachable!("response: {error}"));
        assert_eq!(response.finish_reason, FinishReason::ToolCalls);
        assert_eq!(response.usage.cache_creation_input_tokens, 2);
        let conversation = vec![
            ConversationItem::Message(Message::user("Inspect source")),
            ConversationItem::AssistantToolCalls {
                text: response.text,
                calls: response.tool_calls,
            },
            ConversationItem::ToolResult(ToolResult {
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                content: json!({"content": "pub fn x() {}"}),
                is_error: false,
            }),
        ];
        let request = ModelRequest {
            conversation,
            tools: Vec::new(),
            max_output_tokens: 128,
            temperature: Some(0.0),
        };
        let body = request_body(&config(), &request)
            .unwrap_or_else(|error| unreachable!("request: {error}"));
        assert_eq!(body["store"], false);
        assert_eq!(body["input"][1]["encrypted_content"], "sealed");
        assert_eq!(body["input"][2]["call_id"], "call-1");
        assert_eq!(body["input"][3]["type"], "function_call_output");
    }

    #[test]
    fn invalid_response_items_and_plain_http_fail_closed() {
        let mut unsafe_config = config();
        unsafe_config.base_url = "http://api.openai.com/v1".to_owned();
        assert!(OpenAiResponsesDriver::new(unsafe_config).is_err());
        assert!(parse_response(
            &json!({"object": "response", "status": "completed", "output": [{"type": "web_search_call"}]}),
            None
        ).is_err());
    }

    #[tokio::test]
    async fn buffered_http_response_uses_native_responses_path() {
        use std::{io::Write, net::TcpListener};

        let listener = TcpListener::bind("127.0.0.1:0")
            .unwrap_or_else(|error| unreachable!("listener: {error}"));
        let address = listener
            .local_addr()
            .unwrap_or_else(|error| unreachable!("address: {error}"));
        let server = std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            let request = crate::test_support::read_http_request(&mut stream)?;
            let body = json!({
                "object": "response", "id": "resp-test", "status": "completed",
                "output": [{"type": "message", "content": [{"type": "output_text", "text": "done"}]}],
                "usage": {"input_tokens": 10, "output_tokens": 2}
            })
            .to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )?;
            Ok(request)
        });
        let mut config = config();
        config.base_url = format!("http://{address}/v1");
        let driver = OpenAiResponsesDriver::new(config)
            .unwrap_or_else(|error| unreachable!("driver: {error}"));
        let response = driver
            .invoke(&ModelRequest {
                conversation: vec![ConversationItem::Message(Message::user("hi"))],
                tools: Vec::new(),
                max_output_tokens: 128,
                temperature: None,
            })
            .await
            .unwrap_or_else(|error| unreachable!("response: {error}"));
        assert_eq!(response.text, "done");
        let request = server
            .join()
            .unwrap_or_else(|_| unreachable!("server join"))
            .unwrap_or_else(|error| unreachable!("server: {error}"));
        let request = String::from_utf8(request)
            .unwrap_or_else(|error| unreachable!("UTF-8 request: {error}"));
        assert!(request.starts_with("POST /v1/responses HTTP/1.1"));
        assert!(request.contains("\"store\":false"));
    }
}
