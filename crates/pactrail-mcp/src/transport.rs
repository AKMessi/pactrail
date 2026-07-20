use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt, stream::BoxStream};
use http::{HeaderName, HeaderValue};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientInfo, ClientJsonRpcMessage,
    ContentBlock, GetPromptRequestParams, Implementation, ProtocolVersion,
    ReadResourceRequestParams, ResourceContents, ServerJsonRpcMessage,
};
use rmcp::service::{RoleClient, RunningService, RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use rmcp::transport::async_rw::{JsonRpcMessageCodec, JsonRpcMessageCodecError};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClient, StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    StreamableHttpError, StreamableHttpPostResponse,
};
use rmcp::{ServiceExt, model::JsonObject};
use serde_json::{Value, json};
use sse_stream::{Error as SseError, Sse, SseStream};
use thiserror::Error;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use tokio_util::codec::{FramedRead, FramedWrite};
use tokio_util::sync::CancellationToken;

use crate::{
    McpContextCapture, McpContextKind, McpDiscoveredCatalog, McpDiscoveredTool, McpError,
    McpPromptSelection, McpServerConfig, McpServerIdentity, McpSnapshot, McpSnapshotTool,
    McpTransportConfig,
};

const MAX_WIRE_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_CATALOG_TOOLS: usize = 256;
const MAX_CATALOG_PAGES: usize = 64;
const MAX_CONTEXT_ITEM_BYTES: usize = 64 * 1024;
const MAX_ENV_VALUE_BYTES: usize = 256 * 1024;
const MAX_ENV_TOTAL_BYTES: usize = 2 * 1024 * 1024;
const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);
const JSON_MIME_TYPE: &str = "application/json";
const EVENT_STREAM_MIME_TYPE: &str = "text/event-stream";
const HEADER_SESSION_ID: &str = "Mcp-Session-Id";
const HEADER_LAST_EVENT_ID: &str = "Last-Event-Id";
const HEADER_PROTOCOL_VERSION: &str = "MCP-Protocol-Version";

type ClientSession = RunningService<RoleClient, ClientInfo>;

pub(crate) struct McpInvocation {
    pub content: Value,
    pub summary: String,
    pub observed_effects: Vec<String>,
    pub succeeded: bool,
}

/// Connects explicitly, captures a bounded catalog, and closes the server session.
///
/// This function exercises process or network authority and is intended for an
/// explicit `pactrail mcp snapshot` operation, not normal prompt construction.
///
/// # Errors
///
/// Returns an error for invalid configuration, missing environment values, startup
/// or request timeout, cancellation, protocol failure, oversized data, or cleanup
/// failure.
pub async fn discover(
    config: &McpServerConfig,
    workspace: &Path,
    cancellation: CancellationToken,
) -> Result<McpDiscoveredCatalog, McpError> {
    config.validate()?;
    let runtime_digest = transport_runtime_digest(config)?;
    let mut session = connect(config, workspace, cancellation.clone()).await?;
    let operation = async {
        let identity = server_identity(&session)?;
        let tools = discover_tools(&session, config.request_timeout_seconds, &cancellation).await?;
        let context = capture_context(
            &session,
            &config.resources,
            &config.prompts,
            config.request_timeout_seconds,
            &cancellation,
        )
        .await?;
        Ok(McpDiscoveredCatalog {
            identity,
            transport_runtime_digest: runtime_digest,
            tools,
            context,
        })
    }
    .await;
    let close = session.close_with_timeout(CLOSE_TIMEOUT).await;
    match (operation, close) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(McpError::Transport(format!(
            "MCP cleanup task failed: {error}"
        ))),
        (Ok(_), Ok(None)) => Err(McpError::Transport(
            "MCP cleanup exceeded its deadline".to_owned(),
        )),
        (Ok(catalog), Ok(Some(_))) => Ok(catalog),
    }
}

pub(crate) async fn invoke(
    config: &McpServerConfig,
    snapshot: &McpSnapshot,
    tool: &McpSnapshotTool,
    arguments: JsonObject,
    workspace: &Path,
    cancellation: CancellationToken,
) -> Result<McpInvocation, McpError> {
    let runtime_digest = transport_runtime_digest(config)?;
    snapshot.verify_transport_runtime(&runtime_digest)?;
    let mut session = connect(config, workspace, cancellation.clone()).await?;
    let operation = async {
        snapshot.verify_live_identity(&server_identity(&session)?)?;
        let live_tools =
            discover_tools(&session, config.request_timeout_seconds, &cancellation).await?;
        let live = live_tools
            .iter()
            .find(|candidate| candidate.name == tool.remote_name)
            .ok_or_else(|| {
                McpError::IdentityChanged(format!(
                    "pinned tool {:?} disappeared from the live catalog",
                    tool.remote_name
                ))
            })?;
        snapshot.verify_live_tool(live)?;
        let result = await_request(
            "tool call",
            config.request_timeout_seconds,
            &cancellation,
            session.call_tool(
                CallToolRequestParams::new(tool.remote_name.clone()).with_arguments(arguments),
            ),
        )
        .await?
        .map_err(|error| service_error(&error))?;
        normalize_call_result(config, snapshot, tool, &result)
    }
    .await;
    let close = session.close_with_timeout(CLOSE_TIMEOUT).await;
    match (operation, close) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(McpError::Transport(format!(
            "MCP cleanup task failed: {error}"
        ))),
        (Ok(_), Ok(None)) => Err(McpError::Transport(
            "MCP cleanup exceeded its deadline".to_owned(),
        )),
        (Ok(result), Ok(Some(_))) => Ok(result),
    }
}

/// Computes the runtime identity used to detect executable replacement.
///
/// # Errors
///
/// Returns an error when a stdio executable cannot be canonicalized, opened, or read,
/// or when the transport identity cannot be serialized.
pub fn transport_runtime_digest(config: &McpServerConfig) -> Result<String, McpError> {
    match &config.transport {
        McpTransportConfig::Stdio { command, .. } => {
            let canonical = std::fs::canonicalize(command).map_err(|source| McpError::Io {
                path: command.clone(),
                source,
            })?;
            let metadata = std::fs::metadata(&canonical).map_err(|source| McpError::Io {
                path: canonical.clone(),
                source,
            })?;
            if !metadata.is_file() {
                return Err(McpError::InvalidManifest(format!(
                    "stdio command {} is not a regular file",
                    canonical.display()
                )));
            }
            let mut file = File::open(&canonical).map_err(|source| McpError::Io {
                path: canonical.clone(),
                source,
            })?;
            let mut hasher = blake3::Hasher::new();
            hasher.update(b"pactrail:mcp:stdio-runtime:v1\0");
            hasher.update(canonical.to_string_lossy().as_bytes());
            let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
            loop {
                let read = file.read(&mut buffer).map_err(|source| McpError::Io {
                    path: canonical.clone(),
                    source,
                })?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
            Ok(hasher.finalize().to_hex().to_string())
        }
        McpTransportConfig::StreamableHttp { .. } => config.transport_digest(),
    }
}

pub(crate) async fn connect(
    config: &McpServerConfig,
    workspace: &Path,
    cancellation: CancellationToken,
) -> Result<ClientSession, McpError> {
    let client_info = ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("pactrail", env!("CARGO_PKG_VERSION")),
    )
    .with_protocol_version(ProtocolVersion::V_2025_11_25);
    let startup = Duration::from_secs(config.startup_timeout_seconds);
    match &config.transport {
        McpTransportConfig::Stdio { command, args } => {
            let transport = BoundedChildTransport::spawn(
                command,
                args,
                &config.environment,
                workspace,
                MAX_WIRE_MESSAGE_BYTES,
            )?;
            await_startup(
                client_info.serve_with_ct(transport, cancellation.clone()),
                startup,
                cancellation,
            )
            .await
        }
        McpTransportConfig::StreamableHttp {
            url,
            bearer_token_env,
            ..
        } => {
            let token = bearer_token_env
                .as_deref()
                .map(required_environment_value)
                .transpose()?;
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(startup)
                .timeout(Duration::from_secs(config.request_timeout_seconds))
                .build()
                .map_err(|error| {
                    McpError::Transport(format!("HTTP client setup failed: {error}"))
                })?;
            let bounded = BoundedHttpClient {
                client,
                max_response_bytes: MAX_WIRE_MESSAGE_BYTES,
            };
            let mut transport_config = StreamableHttpClientTransportConfig::with_uri(url.clone());
            transport_config.allow_stateless = true;
            transport_config.reinit_on_expired_session = false;
            transport_config.auth_header = token;
            let transport = StreamableHttpClientTransport::with_client(bounded, transport_config);
            await_startup(
                client_info.serve_with_ct(transport, cancellation.clone()),
                startup,
                cancellation,
            )
            .await
        }
    }
}

async fn await_startup<F>(
    future: F,
    deadline: Duration,
    cancellation: CancellationToken,
) -> Result<ClientSession, McpError>
where
    F: Future<Output = Result<ClientSession, rmcp::service::ClientInitializeError>>,
{
    tokio::select! {
        () = cancellation.cancelled() => Err(McpError::Transport("MCP startup was cancelled".to_owned())),
        result = tokio::time::timeout(deadline, future) => match result {
            Ok(Ok(session)) => Ok(session),
            Ok(Err(error)) => Err(McpError::Transport(format!("MCP initialization failed: {error}"))),
            Err(_) => Err(McpError::Timeout { operation: "startup", seconds: deadline.as_secs() }),
        }
    }
}

fn server_identity(session: &ClientSession) -> Result<McpServerIdentity, McpError> {
    let info = session.peer_info().ok_or_else(|| {
        McpError::Transport("MCP server omitted initialization identity".to_owned())
    })?;
    Ok(McpServerIdentity {
        protocol_version: info.protocol_version.to_string(),
        name: info.server_info.name.clone(),
        version: info.server_info.version.clone(),
    })
}

async fn discover_tools(
    session: &ClientSession,
    timeout_seconds: u64,
    cancellation: &CancellationToken,
) -> Result<Vec<McpDiscoveredTool>, McpError> {
    let future = async {
        let mut cursor = None;
        let mut seen_cursors = BTreeSet::new();
        let mut tools = Vec::new();
        for _ in 0..MAX_CATALOG_PAGES {
            let result = session
                .list_tools(Some(
                    rmcp::model::PaginatedRequestParams::default().with_cursor(cursor.clone()),
                ))
                .await
                .map_err(|error| service_error(&error))?;
            for tool in result.tools {
                if tools.len() >= MAX_CATALOG_TOOLS {
                    return Err(McpError::InvalidSnapshot(format!(
                        "server advertises more than {MAX_CATALOG_TOOLS} tools"
                    )));
                }
                tools.push(McpDiscoveredTool {
                    name: tool.name.into_owned(),
                    description: tool
                        .description
                        .map_or_else(String::new, std::borrow::Cow::into_owned),
                    input_schema: Value::Object((*tool.input_schema).clone()),
                    output_schema: tool
                        .output_schema
                        .map(|schema| Value::Object((*schema).clone())),
                });
            }
            let Some(next) = result.next_cursor else {
                return Ok(tools);
            };
            if !seen_cursors.insert(next.clone()) {
                return Err(McpError::InvalidSnapshot(
                    "server repeated a tool-catalog cursor".to_owned(),
                ));
            }
            cursor = Some(next);
        }
        Err(McpError::InvalidSnapshot(format!(
            "tool catalog exceeded {MAX_CATALOG_PAGES} pages"
        )))
    };
    await_request("tool discovery", timeout_seconds, cancellation, future).await?
}

async fn capture_context(
    session: &ClientSession,
    resources: &[String],
    prompts: &[McpPromptSelection],
    timeout_seconds: u64,
    cancellation: &CancellationToken,
) -> Result<Vec<McpContextCapture>, McpError> {
    let mut captures = Vec::with_capacity(resources.len() + prompts.len());
    for uri in resources {
        let result = await_request(
            "resource read",
            timeout_seconds,
            cancellation,
            session.read_resource(ReadResourceRequestParams::new(uri.clone())),
        )
        .await?
        .map_err(|error| service_error(&error))?;
        let content = render_resources(result.contents)?;
        captures.push(McpContextCapture {
            kind: McpContextKind::Resource,
            identifier: uri.clone(),
            content,
        });
    }
    for prompt in prompts {
        let arguments = prompt
            .arguments
            .iter()
            .map(|(name, value)| (name.clone(), Value::String(value.clone())))
            .collect::<JsonObject>();
        let request = if arguments.is_empty() {
            GetPromptRequestParams::new(prompt.name.clone())
        } else {
            GetPromptRequestParams::new(prompt.name.clone()).with_arguments(arguments)
        };
        let result = await_request(
            "prompt read",
            timeout_seconds,
            cancellation,
            session.get_prompt(request),
        )
        .await?
        .map_err(|error| service_error(&error))?;
        let mut rendered = String::new();
        for message in result.messages {
            let role = serde_json::to_value(message.role)?
                .as_str()
                .unwrap_or("unknown")
                .to_owned();
            let text = render_content_block(&message.content)?;
            append_bounded(&mut rendered, &format!("{role}: {text}"))?;
        }
        captures.push(McpContextCapture {
            kind: McpContextKind::Prompt,
            identifier: prompt.name.clone(),
            content: rendered,
        });
    }
    Ok(captures)
}

async fn await_request<T, F>(
    operation: &'static str,
    timeout_seconds: u64,
    cancellation: &CancellationToken,
    future: F,
) -> Result<T, McpError>
where
    F: Future<Output = T>,
{
    tokio::select! {
        () = cancellation.cancelled() => Err(McpError::Transport(format!("{operation} was cancelled"))),
        result = tokio::time::timeout(Duration::from_secs(timeout_seconds), future) => result.map_err(|_| McpError::Timeout {
            operation,
            seconds: timeout_seconds,
        }),
    }
}

fn render_resources(contents: Vec<ResourceContents>) -> Result<String, McpError> {
    let mut rendered = String::new();
    for content in contents {
        match content {
            ResourceContents::TextResourceContents { uri, text, .. } => {
                append_bounded(&mut rendered, &format!("resource {uri}:\n{text}"))?;
            }
            ResourceContents::BlobResourceContents { .. } => {
                return Err(McpError::InvalidSnapshot(
                    "binary MCP resources cannot be captured as context".to_owned(),
                ));
            }
            _ => {
                return Err(McpError::InvalidSnapshot(
                    "unsupported MCP resource content cannot be captured".to_owned(),
                ));
            }
        }
    }
    Ok(rendered)
}

fn render_content_block(content: &ContentBlock) -> Result<String, McpError> {
    match content {
        ContentBlock::Text(text) => Ok(text.text.clone()),
        ContentBlock::Resource(resource) => match &resource.resource {
            ResourceContents::TextResourceContents { text, .. } => Ok(text.clone()),
            ResourceContents::BlobResourceContents { .. } => Err(McpError::InvalidSnapshot(
                "binary embedded prompt resources cannot be captured as context".to_owned(),
            )),
            _ => Err(McpError::InvalidSnapshot(
                "unsupported embedded prompt content cannot be captured".to_owned(),
            )),
        },
        ContentBlock::Image(_) | ContentBlock::Audio(_) | ContentBlock::ResourceLink(_) => {
            Err(McpError::InvalidSnapshot(
                "non-text MCP prompt content cannot be captured in 0.7".to_owned(),
            ))
        }
        _ => Err(McpError::InvalidSnapshot(
            "unsupported MCP prompt content cannot be captured".to_owned(),
        )),
    }
}

fn normalize_call_result(
    config: &McpServerConfig,
    snapshot: &McpSnapshot,
    tool: &McpSnapshotTool,
    result: &CallToolResult,
) -> Result<McpInvocation, McpError> {
    if result.content.len() > 256 {
        return Err(McpError::ResponseTooLarge {
            limit: config.max_output_bytes,
        });
    }
    if let Some(output_schema) = &tool.output_schema {
        let structured = result.structured_content.as_ref().ok_or_else(|| {
            McpError::InvalidSnapshot(format!(
                "tool {:?} declares an output schema but returned no structured content",
                tool.remote_name
            ))
        })?;
        let validator =
            jsonschema::validator_for(output_schema).map_err(|error| McpError::InvalidSchema {
                tool: tool.remote_name.clone(),
                reason: format!("pinned output schema no longer compiles: {error}"),
            })?;
        if !validator.is_valid(structured) {
            return Err(McpError::InvalidSnapshot(format!(
                "structured result from {:?} does not match its pinned output schema",
                tool.remote_name
            )));
        }
    }
    let blocks = result
        .content
        .iter()
        .map(normalize_result_block)
        .collect::<Vec<_>>();
    let content = json!({
        "server": snapshot.server,
        "tool": tool.remote_name,
        "content": blocks,
        "structured_content": result.structured_content,
        "is_error": result.is_error.unwrap_or(false),
        "snapshot_digest": snapshot.digest,
        "schema_digest": tool.schema_digest,
    });
    let encoded = serde_json::to_vec(&content)?;
    if encoded.len() > config.max_output_bytes {
        return Err(McpError::ResponseTooLarge {
            limit: config.max_output_bytes,
        });
    }
    let succeeded = !result.is_error.unwrap_or(false);
    Ok(McpInvocation {
        content,
        summary: if succeeded {
            format!(
                "MCP tool {} on server {} completed",
                tool.remote_name, snapshot.server
            )
        } else {
            format!(
                "MCP tool {} on server {} reported an error",
                tool.remote_name, snapshot.server
            )
        },
        observed_effects: vec![
            format!("mcp.server:{}", snapshot.server),
            format!("mcp.transport:{}", config.transport.kind()),
            format!("mcp.snapshot:{}", snapshot.digest),
            format!("mcp.schema:{}", tool.schema_digest),
        ],
        succeeded,
    })
}

fn normalize_result_block(block: &ContentBlock) -> Value {
    match block {
        ContentBlock::Text(text) => json!({ "type": "text", "text": text.text }),
        ContentBlock::Image(image) => json!({
            "type": "image",
            "mime_type": image.mime_type,
            "encoded_bytes": image.data.len(),
            "omitted": true,
        }),
        ContentBlock::Audio(audio) => json!({
            "type": "audio",
            "mime_type": audio.mime_type,
            "encoded_bytes": audio.data.len(),
            "omitted": true,
        }),
        ContentBlock::Resource(resource) => match &resource.resource {
            ResourceContents::TextResourceContents {
                uri,
                mime_type,
                text,
                ..
            } => json!({
                "type": "embedded_resource",
                "uri": uri,
                "mime_type": mime_type,
                "text_bytes": text.len(),
                "omitted": true,
            }),
            ResourceContents::BlobResourceContents {
                uri,
                mime_type,
                blob,
                ..
            } => json!({
                "type": "embedded_resource",
                "uri": uri,
                "mime_type": mime_type,
                "encoded_bytes": blob.len(),
                "omitted": true,
            }),
            _ => json!({ "type": "unsupported_resource", "omitted": true }),
        },
        ContentBlock::ResourceLink(link) => json!({
            "type": "resource_link",
            "uri": link.uri,
            "name": link.name,
            "mime_type": link.mime_type,
            "size": link.size,
        }),
        _ => json!({ "type": "unsupported", "omitted": true }),
    }
}

fn append_bounded(target: &mut String, value: &str) -> Result<(), McpError> {
    let separator = usize::from(!target.is_empty());
    if target
        .len()
        .saturating_add(separator)
        .saturating_add(value.len())
        > MAX_CONTEXT_ITEM_BYTES
    {
        return Err(McpError::ResponseTooLarge {
            limit: MAX_CONTEXT_ITEM_BYTES,
        });
    }
    if !target.is_empty() {
        target.push('\n');
    }
    target.push_str(value);
    Ok(())
}

fn service_error(error: &rmcp::ServiceError) -> McpError {
    McpError::Transport(bounded_message(
        &format!("MCP protocol request failed: {error}"),
        2_048,
    ))
}

fn bounded_message(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    format!("{}...", &value[..end])
}

fn required_environment_value(name: &str) -> Result<String, McpError> {
    let value = std::env::var(name).map_err(|_| {
        McpError::InvalidManifest(format!(
            "configured environment variable {name:?} is unavailable or not valid UTF-8"
        ))
    })?;
    if value.len() > MAX_ENV_VALUE_BYTES {
        return Err(McpError::InvalidManifest(format!(
            "configured environment variable {name:?} exceeds {MAX_ENV_VALUE_BYTES} bytes"
        )));
    }
    Ok(value)
}

#[derive(Debug, Error)]
enum WireError {
    #[error("stdio I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("stdio framing failed: {0}")]
    Codec(#[from] JsonRpcMessageCodecError),
}

type Reader = FramedRead<ChildStdout, JsonRpcMessageCodec<RxJsonRpcMessage<RoleClient>>>;
type Writer = FramedWrite<ChildStdin, JsonRpcMessageCodec<TxJsonRpcMessage<RoleClient>>>;

struct BoundedChildTransport {
    child: Option<Child>,
    reader: Reader,
    writer: Arc<Mutex<Option<Writer>>>,
}

impl BoundedChildTransport {
    fn spawn(
        program: &Path,
        args: &[String],
        environment: &BTreeSet<String>,
        workspace: &Path,
        max_message_bytes: usize,
    ) -> Result<Self, McpError> {
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(workspace)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut aggregate_environment = 0_usize;
        for name in environment {
            let value = required_environment_value(name)?;
            aggregate_environment = aggregate_environment.saturating_add(name.len() + value.len());
            if aggregate_environment > MAX_ENV_TOTAL_BYTES {
                return Err(McpError::InvalidManifest(format!(
                    "stdio environment exceeds {MAX_ENV_TOTAL_BYTES} bytes"
                )));
            }
            command.env(name, value);
        }
        let mut child = command.spawn().map_err(|source| McpError::Io {
            path: program.to_path_buf(),
            source,
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            McpError::Transport("stdio server stdout was not captured".to_owned())
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Transport("stdio server stdin was not captured".to_owned()))?;
        Ok(Self {
            child: Some(child),
            reader: FramedRead::new(
                stdout,
                JsonRpcMessageCodec::new_with_max_length(max_message_bytes),
            ),
            writer: Arc::new(Mutex::new(Some(FramedWrite::new(
                stdin,
                JsonRpcMessageCodec::new_with_max_length(max_message_bytes),
            )))),
        })
    }
}

impl Transport<RoleClient> for BoundedChildTransport {
    type Error = WireError;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let writer = self.writer.clone();
        async move {
            let mut guard = writer.lock().await;
            let sink = guard
                .as_mut()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "closed"))?;
            sink.send(item).await.map_err(WireError::from)
        }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        match self.reader.next().await {
            Some(Ok(message)) => Some(message),
            Some(Err(_)) | None => None,
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        self.writer.lock().await.take();
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        if let Ok(result) = tokio::time::timeout(CLOSE_TIMEOUT, child.wait()).await {
            let _status = result?;
        } else {
            child.kill().await?;
            let _status = child.wait().await?;
        }
        Ok(())
    }
}

#[derive(Clone)]
struct BoundedHttpClient {
    client: reqwest::Client,
    max_response_bytes: usize,
}

#[derive(Debug, Error)]
enum HttpClientError {
    #[error("HTTP request failed: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("HTTP response exceeded {limit} bytes")]
    TooLarge { limit: usize },
}

impl StreamableHttpClient for BoundedHttpClient {
    type Error = HttpClientError;

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        let mut request = self
            .client
            .get(uri.as_ref())
            .header(reqwest::header::ACCEPT, EVENT_STREAM_MIME_TYPE)
            .header(HEADER_SESSION_ID, session_id.as_ref());
        if let Some(last_event_id) = last_event_id {
            request = request.header(HEADER_LAST_EVENT_ID, last_event_id);
        }
        if let Some(token) = auth_header {
            request = request.bearer_auth(token);
        }
        request = apply_protocol_headers(request, custom_headers)?;
        let response = request.send().await.map_err(http_client_error)?;
        if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            return Err(StreamableHttpError::ServerDoesNotSupportSse);
        }
        if !response.status().is_success() {
            return Err(unexpected_status(response.status()));
        }
        require_content_type(&response, EVENT_STREAM_MIME_TYPE)?;
        Ok(bounded_sse_stream(response, self.max_response_bytes))
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        let mut request = self
            .client
            .delete(uri.as_ref())
            .header(HEADER_SESSION_ID, session_id.as_ref());
        if let Some(token) = auth_header {
            request = request.bearer_auth(token);
        }
        request = apply_protocol_headers(request, custom_headers)?;
        let response = request.send().await.map_err(http_client_error)?;
        if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            return Ok(());
        }
        if !response.status().is_success() {
            return Err(unexpected_status(response.status()));
        }
        Ok(())
    }

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        let mut request = self
            .client
            .post(uri.as_ref())
            .header(
                reqwest::header::ACCEPT,
                format!("{EVENT_STREAM_MIME_TYPE}, {JSON_MIME_TYPE}"),
            )
            .json(&message);
        if let Some(token) = auth_header {
            request = request.bearer_auth(token);
        }
        if let Some(session) = &session_id {
            request = request.header(HEADER_SESSION_ID, session.as_ref());
        }
        request = apply_protocol_headers(request, custom_headers)?;
        let response = request.send().await.map_err(http_client_error)?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND && session_id.is_some() {
            return Err(StreamableHttpError::SessionExpired);
        }
        if matches!(
            status,
            reqwest::StatusCode::ACCEPTED | reqwest::StatusCode::NO_CONTENT
        ) {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        if !status.is_success() {
            return Err(unexpected_status(status));
        }
        let response_session = response
            .headers()
            .get(HEADER_SESSION_ID)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        match content_type.as_deref() {
            Some(value) if value.starts_with(EVENT_STREAM_MIME_TYPE) => {
                Ok(StreamableHttpPostResponse::Sse(
                    bounded_sse_stream(response, self.max_response_bytes),
                    response_session,
                ))
            }
            Some(value) if value.starts_with(JSON_MIME_TYPE) => {
                let bytes = read_bounded(response, self.max_response_bytes).await?;
                if bytes.is_empty() {
                    return Ok(StreamableHttpPostResponse::Accepted);
                }
                let parsed = serde_json::from_slice::<ServerJsonRpcMessage>(&bytes)?;
                Ok(StreamableHttpPostResponse::Json(parsed, response_session))
            }
            other => Err(StreamableHttpError::UnexpectedContentType(
                other.map(str::to_owned),
            )),
        }
    }
}

fn apply_protocol_headers(
    mut request: reqwest::RequestBuilder,
    headers: HashMap<HeaderName, HeaderValue>,
) -> Result<reqwest::RequestBuilder, StreamableHttpError<HttpClientError>> {
    for (name, value) in headers {
        if name.as_str().eq_ignore_ascii_case(HEADER_PROTOCOL_VERSION) {
            request = request.header(name, value);
        } else {
            return Err(StreamableHttpError::ReservedHeaderConflict(
                name.to_string(),
            ));
        }
    }
    Ok(request)
}

fn bounded_sse_stream(
    response: reqwest::Response,
    limit: usize,
) -> BoxStream<'static, Result<Sse, SseError>> {
    let mut observed = 0_usize;
    let bounded = response.bytes_stream().map(move |item| match item {
        Ok(bytes) => {
            observed = observed.saturating_add(bytes.len());
            if observed > limit {
                Err(HttpClientError::TooLarge { limit })
            } else {
                Ok(bytes)
            }
        }
        Err(error) => Err(HttpClientError::Reqwest(error)),
    });
    SseStream::from_bytes_stream(bounded).boxed()
}

async fn read_bounded(
    response: reqwest::Response,
    limit: usize,
) -> Result<Bytes, StreamableHttpError<HttpClientError>> {
    let mut output = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(http_client_error)?;
        if output.len().saturating_add(chunk.len()) > limit {
            return Err(StreamableHttpError::Client(HttpClientError::TooLarge {
                limit,
            }));
        }
        output.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(output))
}

fn require_content_type(
    response: &reqwest::Response,
    expected: &str,
) -> Result<(), StreamableHttpError<HttpClientError>> {
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    if content_type.is_some_and(|value| value.starts_with(expected)) {
        Ok(())
    } else {
        Err(StreamableHttpError::UnexpectedContentType(
            content_type.map(str::to_owned),
        ))
    }
}

fn http_client_error(error: reqwest::Error) -> StreamableHttpError<HttpClientError> {
    StreamableHttpError::Client(HttpClientError::Reqwest(error))
}

fn unexpected_status(status: reqwest::StatusCode) -> StreamableHttpError<HttpClientError> {
    StreamableHttpError::UnexpectedServerResponse(
        format!("MCP endpoint returned HTTP {}", status.as_u16()).into(),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use pactrail_core::Capability;
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;

    use crate::{McpServerConfig, McpSnapshot, McpToolProfile, McpTransportConfig, discover};

    #[test]
    fn http_runtime_identity_is_stable_and_secret_free() {
        let config = http_config("https://example.test/mcp", 4_096, Some("MCP_TOKEN"));
        let digest = super::transport_runtime_digest(&config)
            .unwrap_or_else(|error| unreachable!("valid digest: {error}"));
        assert_eq!(digest.len(), 64);
        assert!(!digest.contains("MCP_TOKEN"));
    }

    #[tokio::test]
    async fn fragmented_http_catalog_and_call_obey_the_pinned_contract() {
        let (url, server) = spawn_http_server("hello from MCP".to_owned());
        let config = http_config(&url, 8_192, None);
        let workspace =
            TempDir::new().unwrap_or_else(|error| unreachable!("temporary workspace: {error}"));
        let catalog = discover(&config, workspace.path(), CancellationToken::new())
            .await
            .unwrap_or_else(|error| unreachable!("bounded discovery: {error}"));
        let snapshot = McpSnapshot::build(&config, catalog)
            .unwrap_or_else(|error| unreachable!("valid snapshot: {error}"));
        let tool = snapshot
            .tools
            .first()
            .unwrap_or_else(|| unreachable!("one tool"));
        let result = super::invoke(
            &config,
            &snapshot,
            tool,
            serde_json::Map::from_iter([("value".to_owned(), json!("test"))]),
            workspace.path(),
            CancellationToken::new(),
        )
        .await
        .unwrap_or_else(|error| unreachable!("bounded invocation: {error}"));
        assert!(result.succeeded);
        assert_eq!(result.content["content"][0]["text"], "hello from MCP");
        assert!(matches!(server.join(), Ok(Ok(7))));
    }

    #[tokio::test]
    async fn oversized_http_tool_result_fails_without_retry() {
        let (url, server) = spawn_http_server("x".repeat(12_000));
        let config = http_config(&url, 4_096, None);
        let workspace =
            TempDir::new().unwrap_or_else(|error| unreachable!("temporary workspace: {error}"));
        let catalog = discover(&config, workspace.path(), CancellationToken::new())
            .await
            .unwrap_or_else(|error| unreachable!("bounded discovery: {error}"));
        let snapshot = McpSnapshot::build(&config, catalog)
            .unwrap_or_else(|error| unreachable!("valid snapshot: {error}"));
        let tool = snapshot
            .tools
            .first()
            .unwrap_or_else(|| unreachable!("one tool"));
        let result = super::invoke(
            &config,
            &snapshot,
            tool,
            serde_json::Map::from_iter([("value".to_owned(), json!("test"))]),
            workspace.path(),
            CancellationToken::new(),
        )
        .await;
        assert!(matches!(
            result,
            Err(crate::McpError::ResponseTooLarge { limit: 4_096 })
        ));
        // Exactly one tools/call request proves that uncertain remote effects are not retried.
        assert!(matches!(server.join(), Ok(Ok(7))));
    }

    fn http_config(url: &str, max_output_bytes: usize, token: Option<&str>) -> McpServerConfig {
        let mut environment = BTreeSet::new();
        if let Some(token) = token {
            environment.insert(token.to_owned());
        }
        McpServerConfig {
            name: "demo".to_owned(),
            enabled: true,
            transport: McpTransportConfig::StreamableHttp {
                url: url.to_owned(),
                allow_loopback_http: url.starts_with("http://127.0.0.1:"),
                bearer_token_env: token.map(str::to_owned),
            },
            environment,
            startup_timeout_seconds: 10,
            request_timeout_seconds: 10,
            max_output_bytes,
            tools: BTreeMap::from([(
                "echo".to_owned(),
                McpToolProfile {
                    capabilities: BTreeSet::from([Capability::Network]),
                    read_only: true,
                    idempotent: true,
                    parallel_safe: true,
                },
            )]),
            resources: Vec::new(),
            prompts: Vec::new(),
        }
    }

    fn spawn_http_server(
        call_text: String,
    ) -> (String, std::thread::JoinHandle<std::io::Result<usize>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .unwrap_or_else(|error| unreachable!("loopback listener: {error}"));
        listener
            .set_nonblocking(true)
            .unwrap_or_else(|error| unreachable!("nonblocking listener: {error}"));
        let address = listener
            .local_addr()
            .unwrap_or_else(|error| unreachable!("listener address: {error}"));
        let calls = Arc::new(call_text);
        let server = std::thread::spawn(move || -> std::io::Result<usize> {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut requests = 0_usize;
            while requests < 7 && Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                        let body = read_http_body(&mut stream)?;
                        let request: Value =
                            serde_json::from_slice(&body).map_err(std::io::Error::other)?;
                        respond(&mut stream, &request, &calls)?;
                        requests += 1;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(requests)
        });
        (format!("http://{address}/mcp"), server)
    }

    fn read_http_body(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
        const HEADER_LIMIT: usize = 64 * 1024;
        const BODY_LIMIT: usize = 1024 * 1024;
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4_096];
        let header_end = loop {
            let read = stream.read(&mut buffer)?;
            if read == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "HTTP request ended before headers",
                ));
            }
            request.extend_from_slice(&buffer[..read]);
            if request.len() > HEADER_LIMIT + BODY_LIMIT {
                return Err(std::io::Error::other("HTTP request exceeded test bound"));
            }
            if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.strip_prefix("content-length:")
                    .or_else(|| line.strip_prefix("Content-Length:"))
            })
            .map(str::trim)
            .map(str::parse::<usize>)
            .transpose()
            .map_err(std::io::Error::other)?
            .unwrap_or(0);
        if content_length > BODY_LIMIT {
            return Err(std::io::Error::other("HTTP body exceeded test bound"));
        }
        while request.len() < header_end + content_length {
            let read = stream.read(&mut buffer)?;
            if read == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "HTTP request body was truncated",
                ));
            }
            request.extend_from_slice(&buffer[..read]);
        }
        Ok(request[header_end..header_end + content_length].to_vec())
    }

    fn respond(stream: &mut TcpStream, request: &Value, call_text: &str) -> std::io::Result<()> {
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        if method == "notifications/initialized" {
            return write_fragmented_response(stream, "HTTP/1.1 202 Accepted", b"");
        }
        let id = request.get("id").cloned().unwrap_or(json!(null));
        let result = match method {
            "initialize" => json!({
                "protocolVersion": "2025-11-25",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "pactrail-test-mcp", "version": "1.0.0" }
            }),
            "tools/list" => json!({
                "tools": [{
                    "name": "echo",
                    "description": "Echo one value.",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "value": { "type": "string" } },
                        "required": ["value"],
                        "additionalProperties": false
                    }
                }]
            }),
            "tools/call" => json!({
                "content": [{ "type": "text", "text": call_text }],
                "isError": false
            }),
            other => {
                return Err(std::io::Error::other(format!(
                    "unexpected MCP test method {other:?}"
                )));
            }
        };
        let body = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        }))
        .map_err(std::io::Error::other)?;
        write_fragmented_response(stream, "HTTP/1.1 200 OK", &body)
    }

    fn write_fragmented_response(
        stream: &mut TcpStream,
        status: &str,
        body: &[u8],
    ) -> std::io::Result<()> {
        let headers = format!(
            "{status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let response = [headers.as_bytes(), body].concat();
        for chunk in response.chunks(3) {
            stream.write_all(chunk)?;
        }
        stream.flush()
    }
}
