use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use pactrail_core::Capability;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::McpError;

pub const MCP_MANIFEST_SCHEMA: u32 = 1;
const MAX_SERVERS: usize = 32;
const MAX_TOOLS_PER_SERVER: usize = 256;
const MAX_CONTEXT_SELECTIONS: usize = 64;
const MAX_ARGUMENTS: usize = 64;
const MIN_TIMEOUT_SECONDS: u64 = 1;
const MAX_TIMEOUT_SECONDS: u64 = 600;
const MIN_OUTPUT_BYTES: usize = 1_024;
const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

/// Workspace-owned MCP configuration. Secret values are never represented here.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct McpManifest {
    pub schema: u32,
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

impl McpManifest {
    /// Parses and validates a manifest without starting a server or using the network.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid TOML or any unsupported, duplicate, unsafe, or
    /// over-budget configuration.
    pub fn from_toml(input: &str) -> Result<Self, McpError> {
        let manifest: Self = toml::from_str(input)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validates bounds, names, transports, and local authority profiles.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest violates a version, uniqueness, authority,
    /// transport, or resource-bound invariant.
    pub fn validate(&self) -> Result<(), McpError> {
        if self.schema != MCP_MANIFEST_SCHEMA {
            return Err(McpError::InvalidManifest(format!(
                "unsupported schema {}; expected {MCP_MANIFEST_SCHEMA}",
                self.schema
            )));
        }
        if self.servers.len() > MAX_SERVERS {
            return Err(McpError::InvalidManifest(format!(
                "at most {MAX_SERVERS} servers may be configured"
            )));
        }
        let mut names = BTreeSet::new();
        let mut slugs = BTreeMap::<String, String>::new();
        for server in &self.servers {
            server.validate()?;
            if !names.insert(server.name.clone()) {
                return Err(McpError::InvalidManifest(format!(
                    "duplicate server name {:?}",
                    server.name
                )));
            }
            let slug = namespace_component(&server.name)?;
            if let Some(previous) = slugs.insert(slug.clone(), server.name.clone()) {
                return Err(McpError::InvalidManifest(format!(
                    "server names {previous:?} and {:?} normalize to the same namespace {slug:?}",
                    server.name
                )));
            }
        }
        Ok(())
    }

    pub fn enabled_servers(&self) -> impl Iterator<Item = &McpServerConfig> {
        self.servers.iter().filter(|server| server.enabled)
    }
}

/// One explicitly configured MCP server and its locally assigned authority.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct McpServerConfig {
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    pub transport: McpTransportConfig,
    #[serde(default)]
    pub environment: BTreeSet<String>,
    #[serde(default = "default_timeout_seconds")]
    pub startup_timeout_seconds: u64,
    #[serde(default = "default_timeout_seconds")]
    pub request_timeout_seconds: u64,
    #[serde(default = "default_output_bytes")]
    pub max_output_bytes: usize,
    #[serde(default)]
    pub tools: BTreeMap<String, McpToolProfile>,
    #[serde(default)]
    pub resources: Vec<String>,
    #[serde(default)]
    pub prompts: Vec<McpPromptSelection>,
}

impl McpServerConfig {
    /// Validates this server without starting it or contacting its endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe transport, invalid bounds, ambiguous names,
    /// or contradictory local authority profile.
    pub fn validate(&self) -> Result<(), McpError> {
        namespace_component(&self.name)?;
        self.transport.validate()?;
        validate_timeout("startup_timeout_seconds", self.startup_timeout_seconds)?;
        validate_timeout("request_timeout_seconds", self.request_timeout_seconds)?;
        if !(MIN_OUTPUT_BYTES..=MAX_OUTPUT_BYTES).contains(&self.max_output_bytes) {
            return Err(McpError::InvalidManifest(format!(
                "server {:?} max_output_bytes must be between {MIN_OUTPUT_BYTES} and {MAX_OUTPUT_BYTES}",
                self.name
            )));
        }
        if self.tools.len() > MAX_TOOLS_PER_SERVER {
            return Err(McpError::InvalidManifest(format!(
                "server {:?} declares more than {MAX_TOOLS_PER_SERVER} tool profiles",
                self.name
            )));
        }
        if self.resources.len() + self.prompts.len() > MAX_CONTEXT_SELECTIONS {
            return Err(McpError::InvalidManifest(format!(
                "server {:?} selects more than {MAX_CONTEXT_SELECTIONS} context items",
                self.name
            )));
        }
        for name in &self.environment {
            validate_env_name(name)?;
        }
        if let McpTransportConfig::StreamableHttp {
            bearer_token_env: Some(name),
            ..
        } = &self.transport
        {
            validate_env_name(name)?;
            if !self.environment.contains(name) {
                return Err(McpError::InvalidManifest(format!(
                    "bearer token variable {name:?} must also appear in the environment allowlist"
                )));
            }
        }
        let mut tool_slugs = BTreeMap::<String, String>::new();
        for (name, profile) in &self.tools {
            let slug = namespace_component(name)?;
            if let Some(previous) = tool_slugs.insert(slug.clone(), name.clone()) {
                return Err(McpError::InvalidManifest(format!(
                    "tool names {previous:?} and {name:?} normalize to the same component {slug:?}"
                )));
            }
            profile.validate(name)?;
        }
        let mut resources = BTreeSet::new();
        for uri in &self.resources {
            validate_bounded_text("resource URI", uri, 2_048)?;
            if !resources.insert(uri) {
                return Err(McpError::InvalidManifest(format!(
                    "duplicate resource selection {uri:?}"
                )));
            }
        }
        let mut prompts = BTreeSet::new();
        for prompt in &self.prompts {
            prompt.validate()?;
            if !prompts.insert(prompt.name.clone()) {
                return Err(McpError::InvalidManifest(format!(
                    "duplicate prompt selection {:?}",
                    prompt.name
                )));
            }
        }
        Ok(())
    }

    /// Computes the secret-free identity of the transport and environment allowlist.
    ///
    /// # Errors
    ///
    /// Returns an error if the canonical identity cannot be serialized.
    pub fn transport_digest(&self) -> Result<String, McpError> {
        #[derive(Serialize)]
        struct TransportIdentity<'a> {
            transport: &'a McpTransportConfig,
            environment: &'a BTreeSet<String>,
        }
        let encoded = serde_json::to_vec(&TransportIdentity {
            transport: &self.transport,
            environment: &self.environment,
        })?;
        Ok(blake3::hash(&encoded).to_hex().to_string())
    }

    /// Produces the deterministic model-facing namespace for a remote tool.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid component or a name over the provider-safe bound.
    pub fn namespaced_tool_name(&self, remote_name: &str) -> Result<String, McpError> {
        let server = namespace_component(&self.name)?;
        let tool = namespace_component(remote_name)?;
        let name = format!("mcp__{server}__{tool}");
        if name.len() > 64 {
            return Err(McpError::InvalidManifest(format!(
                "namespaced tool name {name:?} exceeds 64 bytes"
            )));
        }
        Ok(name)
    }

    /// Returns every capability that must be authorized before invoking a profile.
    #[must_use]
    pub fn required_capabilities(&self, profile: &McpToolProfile) -> BTreeSet<Capability> {
        let mut capabilities = profile.capabilities.clone();
        capabilities.insert(self.transport.primary_capability());
        if !self.environment.is_empty() {
            capabilities.insert(Capability::SecretUse);
        }
        capabilities
    }

    /// Returns the strongest capability for descriptor-level discovery and UX.
    #[must_use]
    pub fn strongest_capability(&self, profile: &McpToolProfile) -> Capability {
        self.required_capabilities(profile)
            .into_iter()
            .max_by_key(capability_rank)
            .unwrap_or_else(|| self.transport.primary_capability())
    }
}

/// Protocol transport. Exactly one explicit boundary is selected per server.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum McpTransportConfig {
    Stdio {
        command: PathBuf,
        #[serde(default)]
        args: Vec<String>,
    },
    StreamableHttp {
        url: String,
        #[serde(default)]
        allow_loopback_http: bool,
        #[serde(default)]
        bearer_token_env: Option<String>,
    },
}

impl McpTransportConfig {
    fn validate(&self) -> Result<(), McpError> {
        match self {
            Self::Stdio { command, args } => {
                validate_command(command)?;
                if args.len() > MAX_ARGUMENTS {
                    return Err(McpError::InvalidManifest(format!(
                        "stdio accepts at most {MAX_ARGUMENTS} arguments"
                    )));
                }
                for argument in args {
                    validate_bounded_text("stdio argument", argument, 4_096)?;
                }
            }
            Self::StreamableHttp {
                url,
                allow_loopback_http,
                ..
            } => validate_http_url(url, *allow_loopback_http)?,
        }
        Ok(())
    }

    #[must_use]
    pub const fn primary_capability(&self) -> Capability {
        match self {
            Self::Stdio { .. } => Capability::ProcessSpawn,
            Self::StreamableHttp { .. } => Capability::Network,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Stdio { .. } => "stdio",
            Self::StreamableHttp { .. } => "streamable_http",
        }
    }
}

/// Local, authoritative risk and capability assignment for one remote tool.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct McpToolProfile {
    pub capabilities: BTreeSet<Capability>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub idempotent: bool,
    #[serde(default)]
    pub parallel_safe: bool,
}

impl McpToolProfile {
    fn validate(&self, tool: &str) -> Result<(), McpError> {
        if self.capabilities.is_empty() {
            return Err(McpError::InvalidManifest(format!(
                "tool profile {tool:?} must declare at least one capability"
            )));
        }
        if self.parallel_safe && !(self.read_only && self.idempotent) {
            return Err(McpError::InvalidManifest(format!(
                "tool profile {tool:?} may be parallel-safe only when read-only and idempotent"
            )));
        }
        if self.read_only
            && self.capabilities.iter().any(|capability| {
                matches!(
                    capability,
                    Capability::FileWrite | Capability::ExternalWrite
                )
            })
        {
            return Err(McpError::InvalidManifest(format!(
                "tool profile {tool:?} is marked read-only but declares a mutation capability"
            )));
        }
        Ok(())
    }
}

/// Explicit prompt invocation captured into advisory context during snapshotting.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct McpPromptSelection {
    pub name: String,
    #[serde(default)]
    pub arguments: BTreeMap<String, String>,
}

impl McpPromptSelection {
    fn validate(&self) -> Result<(), McpError> {
        validate_bounded_text("prompt name", &self.name, 256)?;
        if self.arguments.len() > MAX_ARGUMENTS {
            return Err(McpError::InvalidManifest(format!(
                "prompt {:?} has more than {MAX_ARGUMENTS} arguments",
                self.name
            )));
        }
        for (name, value) in &self.arguments {
            validate_bounded_text("prompt argument name", name, 256)?;
            validate_bounded_text("prompt argument value", value, 16 * 1024)?;
        }
        Ok(())
    }
}

fn namespace_component(value: &str) -> Result<String, McpError> {
    validate_bounded_text("MCP name", value, 48)?;
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            'A'..='Z' => output.push(character.to_ascii_lowercase()),
            'a'..='z' | '0'..='9' | '_' => output.push(character),
            '-' => output.push('_'),
            _ => {
                return Err(McpError::InvalidManifest(format!(
                    "MCP name {value:?} must use only ASCII letters, digits, '_' or '-'"
                )));
            }
        }
    }
    if output.is_empty()
        || output.starts_with(|character: char| character.is_ascii_digit())
        || output.starts_with('_')
    {
        return Err(McpError::InvalidManifest(format!(
            "MCP name {value:?} must start with an ASCII letter"
        )));
    }
    Ok(output)
}

fn validate_command(command: &Path) -> Result<(), McpError> {
    if !command.is_absolute() {
        return Err(McpError::InvalidManifest(format!(
            "stdio command {} must be an absolute path",
            command.display()
        )));
    }
    let rendered = command.to_string_lossy();
    validate_bounded_text("stdio command", &rendered, 4_096)
}

fn validate_http_url(value: &str, allow_loopback_http: bool) -> Result<(), McpError> {
    validate_bounded_text("MCP URL", value, 2_048)?;
    let url = Url::parse(value)
        .map_err(|error| McpError::InvalidManifest(format!("invalid MCP URL: {error}")))?;
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(McpError::InvalidManifest(
            "MCP URLs cannot contain user information or fragments".to_owned(),
        ));
    }
    match url.scheme() {
        "https" => {}
        "http" if allow_loopback_http && is_literal_loopback(&url) => {}
        "http" => {
            return Err(McpError::InvalidManifest(
                "plain HTTP is allowed only for an explicitly enabled literal loopback address"
                    .to_owned(),
            ));
        }
        scheme => {
            return Err(McpError::InvalidManifest(format!(
                "unsupported MCP URL scheme {scheme:?}; expected https"
            )));
        }
    }
    if url.host_str().is_none() {
        return Err(McpError::InvalidManifest(
            "MCP URL must contain a host".to_owned(),
        ));
    }
    Ok(())
}

fn is_literal_loopback(url: &Url) -> bool {
    url.host_str()
        .and_then(|host| host.parse::<IpAddr>().ok())
        .is_some_and(|address| address.is_loopback())
}

fn validate_env_name(value: &str) -> Result<(), McpError> {
    if value.is_empty()
        || value.len() > 128
        || !value.chars().enumerate().all(|(index, character)| {
            character == '_'
                || character.is_ascii_uppercase()
                || (index > 0 && character.is_ascii_digit())
        })
    {
        return Err(McpError::InvalidManifest(format!(
            "environment name {value:?} must match [A-Z_][A-Z0-9_]{{0,127}}"
        )));
    }
    Ok(())
}

fn validate_bounded_text(label: &str, value: &str, max_bytes: usize) -> Result<(), McpError> {
    if value.trim().is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(McpError::InvalidManifest(format!(
            "{label} must be non-empty, at most {max_bytes} bytes, and contain no control characters"
        )));
    }
    Ok(())
}

fn validate_timeout(field: &str, value: u64) -> Result<(), McpError> {
    if !(MIN_TIMEOUT_SECONDS..=MAX_TIMEOUT_SECONDS).contains(&value) {
        return Err(McpError::InvalidManifest(format!(
            "{field} must be between {MIN_TIMEOUT_SECONDS} and {MAX_TIMEOUT_SECONDS}"
        )));
    }
    Ok(())
}

const fn default_timeout_seconds() -> u64 {
    30
}

const fn default_output_bytes() -> usize {
    256 * 1024
}

const fn capability_rank(capability: &Capability) -> u8 {
    match capability {
        Capability::FileRead | Capability::MemoryRead => 0,
        Capability::FileWrite => 1,
        Capability::Network => 2,
        Capability::SecretUse => 3,
        Capability::ProcessSpawn => 4,
        Capability::ExternalWrite => 5,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    use pactrail_core::Capability;

    use super::{
        MCP_MANIFEST_SCHEMA, McpManifest, McpServerConfig, McpToolProfile, McpTransportConfig,
    };

    fn server() -> McpServerConfig {
        McpServerConfig {
            name: "example".to_owned(),
            enabled: true,
            transport: McpTransportConfig::Stdio {
                command: if cfg!(windows) {
                    PathBuf::from(r"C:\Program Files\example\server.exe")
                } else {
                    PathBuf::from("/usr/local/bin/example")
                },
                args: vec!["--stdio".to_owned()],
            },
            environment: BTreeSet::new(),
            startup_timeout_seconds: 10,
            request_timeout_seconds: 10,
            max_output_bytes: 8_192,
            tools: BTreeMap::from([(
                "read-data".to_owned(),
                McpToolProfile {
                    capabilities: BTreeSet::from([Capability::FileRead]),
                    read_only: true,
                    idempotent: true,
                    parallel_safe: true,
                },
            )]),
            resources: Vec::new(),
            prompts: Vec::new(),
        }
    }

    #[test]
    fn manifest_rejects_namespace_collisions() {
        let mut first = server();
        first.name = "one-two".to_owned();
        let mut second = server();
        second.name = "one_two".to_owned();
        let manifest = McpManifest {
            schema: MCP_MANIFEST_SCHEMA,
            servers: vec![first, second],
        };
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn manifest_rejects_unsafe_http_and_secret_mismatch() {
        let mut configured = server();
        configured.transport = McpTransportConfig::StreamableHttp {
            url: "http://169.254.169.254/mcp".to_owned(),
            allow_loopback_http: true,
            bearer_token_env: Some("MCP_TOKEN".to_owned()),
        };
        let manifest = McpManifest {
            schema: MCP_MANIFEST_SCHEMA,
            servers: vec![configured],
        };
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn manifest_rejects_untyped_and_contradictory_profiles() {
        let mut configured = server();
        configured.tools.insert(
            "write".to_owned(),
            McpToolProfile {
                capabilities: BTreeSet::from([Capability::ExternalWrite]),
                read_only: true,
                idempotent: true,
                parallel_safe: true,
            },
        );
        let manifest = McpManifest {
            schema: MCP_MANIFEST_SCHEMA,
            servers: vec![configured],
        };
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn transport_digest_contains_no_environment_value() {
        let configured = server();
        let first = configured
            .transport_digest()
            .unwrap_or_else(|error| unreachable!("digest: {error}"));
        // SAFETY: environment mutation is intentionally avoided; the digest API has no value input.
        let second = configured
            .transport_digest()
            .unwrap_or_else(|error| unreachable!("digest: {error}"));
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
    }
}
