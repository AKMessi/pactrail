use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{StatusCode, header::HeaderMap};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value, json};
use tracing::warn;

use crate::sse::SseDecoder;
use crate::{
    ConversationItem, FinishReason, Message, ModelCapabilities, ModelDriver, ModelError,
    ModelRequest, ModelResponse, ModelStreamEvent, ModelStreamObserver, Role, ToolCall, Usage,
};

const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_TOOL_ARGUMENT_BYTES: usize = 1024 * 1024;
const MAX_TOOL_CALLS: usize = 128;
const MAX_THOUGHT_SIGNATURE_BYTES: usize = 64 * 1024;
const MAX_RETRIES: u32 = 3;
const MIN_RETRY_DELAY: Duration = Duration::from_millis(250);
const MAX_RETRY_DELAY: Duration = Duration::from_mins(1);
const RATE_LIMIT_RETRY_DELAY: Duration = Duration::from_secs(15);

/// Configuration for Google's native Gemini `GenerateContent` API.
#[derive(Clone)]
pub struct GeminiConfig {
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub api_key: SecretString,
    pub timeout: Duration,
    pub capabilities: ModelCapabilities,
    pub stream: bool,
}

impl GeminiConfig {
    /// Creates the default hosted Gemini configuration.
    #[must_use]
    pub fn hosted(model: impl Into<String>, api_key: SecretString) -> Self {
        Self {
            name: "gemini".to_owned(),
            base_url: "https://generativelanguage.googleapis.com".to_owned(),
            model: model.into(),
            api_key,
            timeout: Duration::from_mins(5),
            capabilities: ModelCapabilities::default(),
            stream: true,
        }
    }
}

/// Native Gemini driver preserving function-call IDs and thought signatures.
pub struct GeminiDriver {
    config: GeminiConfig,
    client: reqwest::Client,
}

impl GeminiDriver {
    /// Builds a credential-safe native Gemini driver.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe endpoint, invalid model identifier,
    /// empty identity or credential, zero timeout, or client construction.
    pub fn new(config: GeminiConfig) -> Result<Self, ModelError> {
        validate_endpoint(&config.base_url)?;
        validate_model_id(&config.model)?;
        if config.name.trim().is_empty() {
            return Err(ModelError::InvalidRequest(
                "provider name cannot be empty".to_owned(),
            ));
        }
        if config.api_key.expose_secret().is_empty() {
            return Err(ModelError::InvalidRequest(
                "Gemini API key cannot be empty".to_owned(),
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

    fn endpoint(&self, stream: bool) -> Result<reqwest::Url, ModelError> {
        let operation = if stream {
            "streamGenerateContent"
        } else {
            "generateContent"
        };
        let mut endpoint = reqwest::Url::parse(&self.config.base_url).map_err(|error| {
            ModelError::InvalidRequest(format!("invalid Gemini endpoint: {error}"))
        })?;
        let base_path = endpoint.path().trim_end_matches('/');
        endpoint.set_path(&format!(
            "{base_path}/v1beta/models/{}:{operation}",
            self.config.model
        ));
        if stream {
            endpoint.set_query(Some("alt=sse"));
        }
        Ok(endpoint)
    }

    async fn send_response(
        &self,
        body: &Value,
        stream: bool,
    ) -> Result<(reqwest::Response, Option<String>, Instant), ModelError> {
        let request_started = Instant::now();
        let endpoint = self.endpoint(stream)?;
        let mut attempt = 0_u32;
        loop {
            let response = self
                .client
                .post(endpoint.clone())
                .header("x-goog-api-key", self.config.api_key.expose_secret())
                .json(body)
                .send()
                .await
                .map_err(ModelError::Transport)?;
            let status = response.status();
            let request_id = response
                .headers()
                .get("x-request-id")
                .or_else(|| response.headers().get("x-goog-request-id"))
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
            let message = gemini_error_message(&bytes);
            if retryable && attempt < MAX_RETRIES {
                attempt += 1;
                let delay = retry_delay(status, attempt, retry_after);
                warn!(
                    attempt,
                    status = status.as_u16(),
                    delay_ms = delay.as_millis(),
                    "retrying Gemini request before response acceptance"
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
        let body = request_body(&self.config, request)?;
        let (response, request_id, _) = self.send_response(&body, false).await?;
        let bytes = read_bounded(response).await?;
        let value = serde_json::from_slice(&bytes).map_err(ModelError::Json)?;
        parse_response(&value, request_id)
    }

    async fn invoke_streaming(
        &self,
        request: &ModelRequest,
        observer: &dyn ModelStreamObserver,
    ) -> Result<ModelResponse, ModelError> {
        let body = request_body(&self.config, request)?;
        let (response, request_id, request_started) = self.send_response(&body, true).await?;
        require_event_stream(&response)?;
        accumulate_stream(response, request_id, request_started, observer).await
    }
}

#[async_trait]
impl ModelDriver for GeminiDriver {
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

fn request_body(config: &GeminiConfig, request: &ModelRequest) -> Result<Value, ModelError> {
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
    let (system, contents) = gemini_contents(&request.conversation)?;
    let declarations = request
        .tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.input_schema,
            })
        })
        .collect::<Vec<_>>();
    let mut generation = json!({"maxOutputTokens": request.max_output_tokens});
    if let Some(temperature) = request.temperature {
        if !(0.0..=2.0).contains(&temperature) {
            return Err(ModelError::InvalidRequest(
                "Gemini temperature must be between 0 and 2".to_owned(),
            ));
        }
        generation["temperature"] = json!(temperature);
    }
    let mut body = json!({
        "contents": contents,
        "generationConfig": generation,
    });
    if !system.is_empty() {
        body["systemInstruction"] = json!({"parts": [{"text": system}]});
    }
    if !declarations.is_empty() {
        body["tools"] = json!([{"functionDeclarations": declarations}]);
        body["toolConfig"] = json!({
            "functionCallingConfig": {"mode": "AUTO"}
        });
    }
    Ok(body)
}

fn gemini_contents(conversation: &[ConversationItem]) -> Result<(String, Vec<Value>), ModelError> {
    let mut system = Vec::new();
    let mut contents = Vec::<Value>::new();
    for item in conversation {
        match item {
            ConversationItem::Message(Message {
                role: Role::System,
                content,
            }) => system.push(content.as_str()),
            ConversationItem::Message(Message { role, content }) => {
                let role = match role {
                    Role::User => "user",
                    Role::Assistant => "model",
                    Role::System => {
                        return Err(ModelError::InvalidRequest(
                            "internal Gemini system-message routing failed".to_owned(),
                        ));
                    }
                };
                push_content(&mut contents, role, vec![json!({"text": content})])?;
            }
            ConversationItem::AssistantToolCalls { text, calls } => {
                let mut parts = Vec::with_capacity(calls.len().saturating_add(1));
                if !text.is_empty() {
                    parts.push(json!({"text": text}));
                }
                for call in calls {
                    let mut part = json!({
                        "functionCall": {
                            "id": call.id,
                            "name": call.name,
                            "args": call.arguments,
                        }
                    });
                    if let Some(signature) = call.extensions.get("thought_signature") {
                        part["thoughtSignature"] = signature.clone();
                    }
                    parts.push(part);
                }
                push_content(&mut contents, "model", parts)?;
            }
            ConversationItem::ToolResult(result) => {
                let response = if result.content.is_object() {
                    result.content.clone()
                } else {
                    json!({"result": result.content})
                };
                push_content(
                    &mut contents,
                    "user",
                    vec![json!({
                        "functionResponse": {
                            "id": result.call_id,
                            "name": result.name,
                            "response": response,
                        }
                    })],
                )?;
            }
        }
    }
    if contents.is_empty() {
        return Err(ModelError::InvalidRequest(
            "Gemini requires at least one non-system content item".to_owned(),
        ));
    }
    Ok((system.join("\n\n"), contents))
}

fn push_content(
    contents: &mut Vec<Value>,
    role: &str,
    parts: Vec<Value>,
) -> Result<(), ModelError> {
    if parts.is_empty() {
        return Err(ModelError::InvalidRequest(
            "Gemini content parts cannot be empty".to_owned(),
        ));
    }
    if let Some(last) = contents.last_mut()
        && last.get("role").and_then(Value::as_str) == Some(role)
    {
        let existing = last
            .get_mut("parts")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| {
                ModelError::InvalidRequest("internal Gemini content shape is invalid".to_owned())
            })?;
        existing.extend(parts);
    } else {
        contents.push(json!({"role": role, "parts": parts}));
    }
    Ok(())
}

fn parse_response(value: &Value, request_id: Option<String>) -> Result<ModelResponse, ModelError> {
    if let Some(reason) = value
        .pointer("/promptFeedback/blockReason")
        .and_then(Value::as_str)
    {
        return Ok(blocked_response(value, request_id, reason));
    }
    let candidate = one_candidate(value)?.ok_or_else(|| {
        ModelError::MalformedResponse("Gemini response contains no candidate".to_owned())
    })?;
    let mut text = String::new();
    let tool_calls = parse_parts(candidate, &mut text)?;
    let finish_reason = gemini_finish_reason(candidate.get("finishReason").and_then(Value::as_str));
    if finish_reason == FinishReason::ToolCalls && tool_calls.is_empty() {
        return Err(ModelError::MalformedResponse(
            "Gemini reported a function-call stop without a function call".to_owned(),
        ));
    }
    let usage = value
        .get("usageMetadata")
        .map_or_else(Usage::default, gemini_usage);
    Ok(ModelResponse {
        text,
        tool_calls,
        finish_reason,
        usage,
        provider_request_id: request_id,
        extensions: response_extensions(value, false, None),
    })
}

fn one_candidate(value: &Value) -> Result<Option<&Value>, ModelError> {
    let candidates = value
        .get("candidates")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if candidates.len() > 1 {
        return Err(ModelError::MalformedResponse(
            "Gemini returned more than one candidate".to_owned(),
        ));
    }
    Ok(candidates.first())
}

fn parse_parts(candidate: &Value, text: &mut String) -> Result<Vec<ToolCall>, ModelError> {
    let parts = candidate
        .pointer("/content/parts")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut calls = Vec::new();
    for part in parts {
        if part.get("thought").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        if let Some(delta) = part.get("text").and_then(Value::as_str) {
            append_text(text, delta)?;
            continue;
        }
        if let Some(call) = part.get("functionCall") {
            if calls.len() >= MAX_TOOL_CALLS {
                return Err(ModelError::MalformedResponse(
                    "Gemini exceeded the function-call limit".to_owned(),
                ));
            }
            calls.push(parse_function_call(call, part, calls.len())?);
            continue;
        }
        if part.get("thoughtSignature").is_some() {
            continue;
        }
        return Err(ModelError::MalformedResponse(
            "Gemini returned an unsupported content part".to_owned(),
        ));
    }
    Ok(calls)
}

fn parse_function_call(
    value: &Value,
    part: &Value,
    ordinal: usize,
) -> Result<ToolCall, ModelError> {
    let name = bounded_identifier(value.get("name"), "Gemini function name")?;
    let arguments = value.get("args").cloned().unwrap_or_else(|| json!({}));
    if !arguments.is_object() {
        return Err(ModelError::MalformedResponse(
            "Gemini function arguments must be a JSON object".to_owned(),
        ));
    }
    if serde_json::to_vec(&arguments)
        .map_err(ModelError::Json)?
        .len()
        > MAX_TOOL_ARGUMENT_BYTES
    {
        return Err(ModelError::ResponseTooLarge {
            limit: MAX_TOOL_ARGUMENT_BYTES,
        });
    }
    let id = match value.get("id") {
        Some(id) => bounded_identifier(Some(id), "Gemini function-call id")?,
        None => synthetic_call_id(&name, &arguments, ordinal)?,
    };
    let mut extensions = Map::new();
    if let Some(signature) = part.get("thoughtSignature").and_then(Value::as_str) {
        if signature.is_empty() || signature.len() > MAX_THOUGHT_SIGNATURE_BYTES {
            return Err(ModelError::MalformedResponse(
                "Gemini thought signature is empty or oversized".to_owned(),
            ));
        }
        extensions.insert(
            "thought_signature".to_owned(),
            Value::String(signature.to_owned()),
        );
    }
    Ok(ToolCall {
        id,
        name,
        arguments,
        extensions,
    })
}

fn synthetic_call_id(name: &str, arguments: &Value, ordinal: usize) -> Result<String, ModelError> {
    let bytes = serde_json::to_vec(&(name, arguments, ordinal)).map_err(ModelError::Json)?;
    let digest = blake3::hash(&bytes).to_hex().to_string();
    Ok(format!("gemini-{}", &digest[..24]))
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

fn gemini_usage(value: &Value) -> Usage {
    Usage {
        input_tokens: number(value, "promptTokenCount"),
        output_tokens: number(value, "candidatesTokenCount"),
        cached_input_tokens: number(value, "cachedContentTokenCount"),
    }
}

fn gemini_finish_reason(value: Option<&str>) -> FinishReason {
    match value {
        Some("STOP") => FinishReason::Complete,
        Some("MAX_TOKENS") => FinishReason::Length,
        Some("SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "IMAGE_SAFETY") => {
            FinishReason::ContentFilter
        }
        Some("MALFORMED_FUNCTION_CALL" | "UNEXPECTED_TOOL_CALL") => FinishReason::ToolCalls,
        _ => FinishReason::Unknown,
    }
}

fn blocked_response(value: &Value, request_id: Option<String>, reason: &str) -> ModelResponse {
    let mut extensions = response_extensions(value, false, None);
    extensions.insert("block_reason".to_owned(), Value::String(reason.to_owned()));
    ModelResponse {
        text: String::new(),
        tool_calls: Vec::new(),
        finish_reason: FinishReason::ContentFilter,
        usage: value
            .get("usageMetadata")
            .map_or_else(Usage::default, gemini_usage),
        provider_request_id: request_id,
        extensions,
    }
}

fn response_extensions(
    value: &Value,
    streaming: bool,
    first_byte_ms: Option<u64>,
) -> Map<String, Value> {
    let mut extensions = Map::new();
    for key in ["modelVersion", "responseId"] {
        if let Some(value) = value.get(key) {
            extensions.insert(key.to_owned(), value.clone());
        }
    }
    extensions.insert("streaming".to_owned(), Value::Bool(streaming));
    if let Some(ms) = first_byte_ms {
        extensions.insert("time_to_first_byte_ms".to_owned(), Value::from(ms));
    }
    extensions
}

#[derive(Default)]
struct GeminiStreamAccumulator {
    text: String,
    calls: Vec<ToolCall>,
    call_fingerprints: BTreeSet<String>,
    call_ids: BTreeMap<String, String>,
    finish_reason: Option<FinishReason>,
    usage: Usage,
    usage_seen: bool,
    response_id: Option<String>,
    model_version: Option<String>,
}

impl GeminiStreamAccumulator {
    fn apply(
        &mut self,
        value: &Value,
        observer: &dyn ModelStreamObserver,
    ) -> Result<(), ModelError> {
        if let Some(reason) = value
            .pointer("/promptFeedback/blockReason")
            .and_then(Value::as_str)
        {
            self.finish_reason = Some(FinishReason::ContentFilter);
            if self.text.is_empty() {
                return Ok(());
            }
            return Err(ModelError::MalformedResponse(format!(
                "Gemini blocked a stream after emitting text: {reason}"
            )));
        }
        merge_optional_string(
            &mut self.response_id,
            value.get("responseId").and_then(Value::as_str),
            "Gemini response id",
        )?;
        merge_optional_string(
            &mut self.model_version,
            value.get("modelVersion").and_then(Value::as_str),
            "Gemini model version",
        )?;
        if let Some(usage) = value.get("usageMetadata") {
            let next = gemini_usage(usage);
            if self.usage_seen
                && (next.input_tokens < self.usage.input_tokens
                    || next.output_tokens < self.usage.output_tokens
                    || next.cached_input_tokens < self.usage.cached_input_tokens)
            {
                return Err(ModelError::MalformedResponse(
                    "Gemini stream usage counters regressed".to_owned(),
                ));
            }
            self.usage = next;
            self.usage_seen = true;
            observer.on_event(&ModelStreamEvent::UsageUpdate { usage: next });
        }
        let Some(candidate) = one_candidate(value)? else {
            return Ok(());
        };
        let mut chunk_text = String::new();
        let calls = parse_parts(candidate, &mut chunk_text)?;
        append_text(&mut self.text, &chunk_text)?;
        if !chunk_text.is_empty() {
            observer.on_event(&ModelStreamEvent::TextDelta { text: chunk_text });
        }
        for call in calls {
            let fingerprint = tool_call_fingerprint(&call)?;
            if !self.call_fingerprints.insert(fingerprint) {
                continue;
            }
            if let Some(existing) = self.call_ids.insert(call.id.clone(), call.name.clone())
                && existing != call.name
            {
                return Err(ModelError::MalformedResponse(
                    "Gemini reused a function-call id with a different name".to_owned(),
                ));
            }
            let index = self.calls.len();
            if index >= MAX_TOOL_CALLS {
                return Err(ModelError::MalformedResponse(
                    "Gemini exceeded the function-call limit".to_owned(),
                ));
            }
            observer.on_event(&ModelStreamEvent::ToolCallStarted {
                index,
                id: call.id.clone(),
                name: call.name.clone(),
            });
            observer.on_event(&ModelStreamEvent::ToolArgumentsDelta {
                index,
                bytes: serde_json::to_vec(&call.arguments)
                    .map_err(ModelError::Json)?
                    .len(),
            });
            self.calls.push(call);
        }
        if let Some(reason) = candidate.get("finishReason").and_then(Value::as_str)
            && self
                .finish_reason
                .replace(gemini_finish_reason(Some(reason)))
                .is_some()
        {
            return Err(ModelError::MalformedResponse(
                "Gemini stream emitted more than one finish reason".to_owned(),
            ));
        }
        Ok(())
    }

    fn finish(
        self,
        request_id: Option<String>,
        first_byte_ms: u64,
    ) -> Result<ModelResponse, ModelError> {
        let finish_reason = self.finish_reason.ok_or_else(|| {
            ModelError::MalformedResponse(
                "Gemini stream disconnected before a finish reason".to_owned(),
            )
        })?;
        if finish_reason == FinishReason::ToolCalls && self.calls.is_empty() {
            return Err(ModelError::MalformedResponse(
                "Gemini stream reported a function-call failure without a call".to_owned(),
            ));
        }
        let mut extensions = Map::from_iter([
            ("streaming".to_owned(), Value::Bool(true)),
            (
                "time_to_first_byte_ms".to_owned(),
                Value::from(first_byte_ms),
            ),
        ]);
        if let Some(value) = self.response_id {
            extensions.insert("responseId".to_owned(), Value::String(value));
        }
        if let Some(value) = self.model_version {
            extensions.insert("modelVersion".to_owned(), Value::String(value));
        }
        Ok(ModelResponse {
            text: self.text,
            tool_calls: self.calls,
            finish_reason,
            usage: self.usage,
            provider_request_id: request_id,
            extensions,
        })
    }
}

fn merge_optional_string(
    target: &mut Option<String>,
    value: Option<&str>,
    field: &str,
) -> Result<(), ModelError> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(ModelError::MalformedResponse(format!(
            "{field} is empty, oversized, or contains control characters"
        )));
    }
    match target {
        Some(existing) if existing != value => Err(ModelError::MalformedResponse(format!(
            "{field} changed during the stream"
        ))),
        Some(_) => Ok(()),
        None => {
            *target = Some(value.to_owned());
            Ok(())
        }
    }
}

fn tool_call_fingerprint(call: &ToolCall) -> Result<String, ModelError> {
    let bytes = serde_json::to_vec(&(
        call.id.as_str(),
        call.name.as_str(),
        &call.arguments,
        &call.extensions,
    ))
    .map_err(ModelError::Json)?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
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
    let mut accumulator = GeminiStreamAccumulator::default();
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
            if event.event.as_deref().is_some_and(|kind| kind == "error") {
                return Err(ModelError::Provider {
                    status: 200,
                    message: gemini_error_message(event.data.as_bytes()),
                });
            }
            let value = serde_json::from_str(&event.data).map_err(ModelError::Json)?;
            accumulator.apply(&value, observer)?;
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

fn validate_model_id(model: &str) -> Result<(), ModelError> {
    if model.is_empty()
        || model.len() > 512
        || !model
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ModelError::InvalidRequest(
            "Gemini model id must use only ASCII letters, numbers, '-', '_', or '.'".to_owned(),
        ));
    }
    Ok(())
}

fn validate_endpoint(base_url: &str) -> Result<(), ModelError> {
    let endpoint = reqwest::Url::parse(base_url)
        .map_err(|error| ModelError::InvalidRequest(format!("invalid Gemini endpoint: {error}")))?;
    let host = endpoint.host_str().unwrap_or_default();
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if endpoint.scheme() != "https" && !(endpoint.scheme() == "http" && loopback) {
        return Err(ModelError::InvalidRequest(
            "remote Gemini endpoints must use HTTPS".to_owned(),
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
            "Gemini did not return text/event-stream; select buffered mode explicitly".to_owned(),
        ))
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

fn gemini_error_message(bytes: &[u8]) -> String {
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

    fn config() -> GeminiConfig {
        GeminiConfig {
            name: "gemini".to_owned(),
            base_url: "https://generativelanguage.googleapis.com".to_owned(),
            model: "gemini-test".to_owned(),
            api_key: SecretString::from("test-key"),
            timeout: Duration::from_secs(1),
            capabilities: ModelCapabilities {
                max_output_tokens: 1_024,
                ..ModelCapabilities::default()
            },
            stream: true,
        }
    }

    #[test]
    fn native_request_preserves_function_ids_results_and_thought_signatures() {
        let mut extensions = Map::new();
        extensions.insert(
            "thought_signature".to_owned(),
            Value::String("signed-state".to_owned()),
        );
        let request = ModelRequest {
            conversation: vec![
                ConversationItem::Message(Message::system("policy")),
                ConversationItem::Message(Message::user("inspect")),
                ConversationItem::AssistantToolCalls {
                    text: String::new(),
                    calls: vec![ToolCall {
                        id: "call-1".to_owned(),
                        name: "read_file".to_owned(),
                        arguments: json!({"path": "src/lib.rs"}),
                        extensions,
                    }],
                },
                ConversationItem::ToolResult(crate::ToolResult {
                    call_id: "call-1".to_owned(),
                    name: "read_file".to_owned(),
                    content: json!({"text": "source"}),
                    is_error: false,
                }),
            ],
            tools: Vec::new(),
            max_output_tokens: 512,
            temperature: Some(0.0),
        };
        let body = request_body(&config(), &request)
            .unwrap_or_else(|error| unreachable!("native request: {error}"));
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "policy");
        assert_eq!(
            body["contents"][1]["parts"][0]["thoughtSignature"],
            "signed-state"
        );
        assert_eq!(
            body["contents"][2]["parts"][0]["functionResponse"]["id"],
            "call-1"
        );
    }

    #[test]
    fn buffered_response_preserves_signed_function_call_and_usage() {
        let response = parse_response(
            &json!({
                "candidates": [{
                    "content": {"role": "model", "parts": [{
                        "functionCall": {"id": "call-1", "name": "read_file", "args": {"path": "src/lib.rs"}},
                        "thoughtSignature": "signed-state"
                    }]},
                    "finishReason": "STOP"
                }],
                "usageMetadata": {"promptTokenCount": 12, "candidatesTokenCount": 4, "cachedContentTokenCount": 3},
                "modelVersion": "gemini-test-001",
                "responseId": "response-1"
            }),
            Some("request-1".to_owned()),
        )
        .unwrap_or_else(|error| unreachable!("native response: {error}"));
        assert_eq!(response.tool_calls[0].id, "call-1");
        assert_eq!(
            response.tool_calls[0].extensions["thought_signature"],
            "signed-state"
        );
        assert_eq!(response.usage.cached_input_tokens, 3);
    }

    #[test]
    fn missing_function_id_gets_a_stable_synthetic_id() {
        let part = json!({"functionCall": {"name": "read_file", "args": {"path": "a"}}});
        let first = parse_function_call(&part["functionCall"], &part, 0)
            .unwrap_or_else(|error| unreachable!("call: {error}"));
        let second = parse_function_call(&part["functionCall"], &part, 0)
            .unwrap_or_else(|error| unreachable!("call: {error}"));
        assert_eq!(first.id, second.id);
        assert!(first.id.starts_with("gemini-"));
    }

    #[test]
    fn stream_deduplicates_identical_function_parts_and_rejects_usage_regression() {
        let observer = RecordingObserver::default();
        let mut accumulator = GeminiStreamAccumulator::default();
        let call = json!({
            "candidates": [{
                "content": {"parts": [{"functionCall": {"id": "call-1", "name": "read_file", "args": {"path": "a"}}}]}
            }],
            "usageMetadata": {"promptTokenCount": 10, "candidatesTokenCount": 2}
        });
        accumulator
            .apply(&call, &observer)
            .unwrap_or_else(|error| unreachable!("first chunk: {error}"));
        accumulator
            .apply(&call, &observer)
            .unwrap_or_else(|error| unreachable!("duplicate chunk: {error}"));
        assert_eq!(accumulator.calls.len(), 1);
        assert!(matches!(
            accumulator.apply(
                &json!({
                    "candidates": [],
                    "usageMetadata": {"promptTokenCount": 9, "candidatesTokenCount": 2}
                }),
                &observer
            ),
            Err(ModelError::MalformedResponse(message)) if message.contains("regressed")
        ));
    }

    #[test]
    fn model_identifier_cannot_escape_the_endpoint_path() {
        let mut invalid = config();
        invalid.model = "models/../other".to_owned();
        assert!(matches!(
            GeminiDriver::new(invalid),
            Err(ModelError::InvalidRequest(_))
        ));
    }
}
