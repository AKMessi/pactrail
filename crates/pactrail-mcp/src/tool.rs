use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use pactrail_core::{ApprovalBinding, ApprovalRequest, Capability};
use pactrail_tools::{Tool, ToolContext, ToolDescriptor, ToolError, ToolOutput, ToolRegistry};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::{
    McpError, McpServerConfig, McpSnapshot, McpSnapshotTool, McpTransportConfig, validate_arguments,
};

/// One pinned MCP tool executing through Pactrail's standard Tool Kernel.
pub struct McpTool {
    config: Arc<McpServerConfig>,
    snapshot: Arc<McpSnapshot>,
    tool: McpSnapshotTool,
    cancellation: CancellationToken,
}

impl McpTool {
    /// Creates a remote tool only after revalidating its manifest and snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the configuration or snapshot is stale, tampered with,
    /// or does not contain the requested public tool name.
    pub fn new(
        config: Arc<McpServerConfig>,
        snapshot: Arc<McpSnapshot>,
        public_name: &str,
        cancellation: CancellationToken,
    ) -> Result<Self, McpError> {
        config.validate()?;
        snapshot.validate(&config)?;
        let tool = snapshot
            .tools
            .iter()
            .find(|tool| tool.public_name == public_name)
            .cloned()
            .ok_or_else(|| {
                McpError::InvalidSnapshot(format!(
                    "snapshot contains no tool named {public_name:?}"
                ))
            })?;
        Ok(Self {
            config,
            snapshot,
            tool,
            cancellation,
        })
    }

    fn authorize(&self, context: &ToolContext<'_>, arguments: &Value) -> Result<(), ToolError> {
        for capability in self.config.required_capabilities(&self.tool.profile) {
            let resource = approval_resource(&self.config, &self.tool, &capability, arguments);
            let actor = json!({
                "snapshot": self.snapshot.digest,
                "tool": self.tool.public_name,
                "capability": capability,
                "arguments_digest": blake3::hash(arguments.to_string().as_bytes()).to_hex().to_string(),
            })
            .to_string();
            if let Some(run_id) = context.run_id {
                context.authorize_request(ApprovalRequest {
                    binding: ApprovalBinding {
                        run_id,
                        capability: capability.clone(),
                        resource: resource.clone(),
                        actor_fingerprint: blake3::hash(actor.as_bytes()).to_hex().to_string(),
                        backend_kind: format!("mcp_{}", self.config.transport.kind()),
                        backend_identity: Some(self.snapshot.transport_runtime_digest.clone()),
                        profile_digest: self.snapshot.digest.clone(),
                    },
                    reason: format!(
                        "the model requested pinned MCP tool {} using {}",
                        self.tool.public_name, capability
                    ),
                    presentation: approval_presentation(
                        &self.config,
                        &self.tool,
                        &capability,
                        &resource,
                    ),
                })?;
            } else {
                context.authorize(&capability, resource, &self.tool.public_name)?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Tool for McpTool {
    fn descriptor(&self) -> ToolDescriptor {
        self.tool.descriptor(&self.config)
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        input: Value,
    ) -> Result<ToolOutput, ToolError> {
        validate_arguments(&self.tool.remote_name, &self.tool.input_schema, &input)
            .map_err(|error| adapter_error(&error))?;
        self.authorize(context, &input)?;
        if self.cancellation.is_cancelled() {
            return Err(ToolError::Cancelled {
                operation: self.tool.public_name.clone(),
            });
        }
        let arguments = input
            .as_object()
            .cloned()
            .ok_or_else(|| ToolError::Adapter {
                adapter: "mcp",
                message: "validated MCP arguments unexpectedly lost their object shape".to_owned(),
            })?;
        let result = crate::transport::invoke(
            &self.config,
            &self.snapshot,
            &self.tool,
            arguments,
            context.workspace.workspace_root(),
            self.cancellation.clone(),
        )
        .await;
        if self.cancellation.is_cancelled() {
            return Err(ToolError::Cancelled {
                operation: self.tool.public_name.clone(),
            });
        }
        let result = result.map_err(|error| adapter_error(&error))?;
        Ok(ToolOutput {
            content: result.content,
            summary: result.summary,
            observed_effects: result.observed_effects,
            succeeded: result.succeeded,
            truncated: false,
        })
    }
}

/// Registers all pinned tools in deterministic snapshot order.
///
/// # Errors
///
/// Returns an error for an invalid snapshot or any collision with an existing tool.
pub fn register_snapshot(
    registry: &mut ToolRegistry,
    config: &Arc<McpServerConfig>,
    snapshot: &Arc<McpSnapshot>,
    cancellation: &CancellationToken,
) -> Result<usize, McpError> {
    config.validate()?;
    snapshot.validate(config)?;
    let mut names = snapshot
        .tools
        .iter()
        .map(|tool| tool.public_name.clone())
        .collect::<Vec<_>>();
    names.sort();
    for name in &names {
        registry.register(McpTool::new(
            config.clone(),
            snapshot.clone(),
            name,
            cancellation.clone(),
        )?)?;
    }
    Ok(names.len())
}

fn approval_resource(
    config: &McpServerConfig,
    tool: &McpSnapshotTool,
    capability: &Capability,
    arguments: &Value,
) -> String {
    match capability {
        Capability::ProcessSpawn => match &config.transport {
            McpTransportConfig::Stdio { command, args } => json!({
                "command": command,
                "args": args,
                "environment_names": config.environment,
            })
            .to_string(),
            McpTransportConfig::StreamableHttp { .. } => tool.public_name.clone(),
        },
        Capability::Network => match &config.transport {
            McpTransportConfig::StreamableHttp { url, .. } => url.clone(),
            McpTransportConfig::Stdio { .. } => tool.public_name.clone(),
        },
        Capability::SecretUse => config
            .environment
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(","),
        Capability::ExternalWrite => json!({
            "tool": tool.public_name,
            "arguments_digest": blake3::hash(arguments.to_string().as_bytes()).to_hex().to_string(),
        })
        .to_string(),
        Capability::FileRead | Capability::FileWrite | Capability::MemoryRead => ".".to_owned(),
    }
}

fn approval_presentation(
    config: &McpServerConfig,
    tool: &McpSnapshotTool,
    capability: &Capability,
    resource: &str,
) -> BTreeMap<String, String> {
    let boundary = match &config.transport {
        McpTransportConfig::Stdio { command, args } => {
            std::iter::once(command.display().to_string())
                .chain(args.iter().cloned())
                .collect::<Vec<_>>()
                .join(" ")
        }
        McpTransportConfig::StreamableHttp { url, .. } => url.clone(),
    };
    BTreeMap::from([
        ("server".to_owned(), config.name.clone()),
        ("tool".to_owned(), tool.public_name.clone()),
        ("capability".to_owned(), capability.to_string()),
        ("transport".to_owned(), config.transport.kind().to_owned()),
        ("boundary".to_owned(), boundary),
        ("resource".to_owned(), resource.to_owned()),
        (
            "environment_names".to_owned(),
            if config.environment.is_empty() {
                "none".to_owned()
            } else {
                config
                    .environment
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            },
        ),
    ])
}

fn adapter_error(error: &McpError) -> ToolError {
    let message = error.to_string();
    ToolError::Adapter {
        adapter: "mcp",
        message: bounded_message(&message, 2_048),
    }
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    use pactrail_core::Capability;

    use crate::{McpServerConfig, McpSnapshotTool, McpToolProfile, McpTransportConfig};

    #[test]
    fn remote_write_approval_never_contains_raw_arguments() {
        let config = McpServerConfig {
            name: "demo".to_owned(),
            enabled: true,
            transport: McpTransportConfig::Stdio {
                command: PathBuf::from(if cfg!(windows) {
                    r"C:\tools\demo.exe"
                } else {
                    "/tools/demo"
                }),
                args: Vec::new(),
            },
            environment: BTreeSet::new(),
            startup_timeout_seconds: 10,
            request_timeout_seconds: 10,
            max_output_bytes: 4_096,
            tools: BTreeMap::new(),
            resources: Vec::new(),
            prompts: Vec::new(),
        };
        let tool = McpSnapshotTool {
            remote_name: "write".to_owned(),
            public_name: "mcp__demo__write".to_owned(),
            description: "untrusted".to_owned(),
            input_schema: serde_json::json!({"type":"object"}),
            output_schema: None,
            schema_digest: "a".repeat(64),
            profile: McpToolProfile {
                capabilities: BTreeSet::from([Capability::ExternalWrite]),
                read_only: false,
                idempotent: false,
                parallel_safe: false,
            },
        };
        let resource = super::approval_resource(
            &config,
            &tool,
            &Capability::ExternalWrite,
            &serde_json::json!({"password":"do-not-log"}),
        );
        assert!(!resource.contains("do-not-log"));
        assert!(resource.contains("arguments_digest"));
    }
}
