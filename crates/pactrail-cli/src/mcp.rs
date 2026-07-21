use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pactrail_context::ContextFragment;
use pactrail_core::Capability;
use pactrail_mcp::{
    McpDiscoveredCatalog, McpError, McpManifest, McpServerConfig, McpSnapshot, McpTransportConfig,
    discover, register_snapshot,
};
use pactrail_tools::ToolRegistry;
use serde::Serialize;
use tempfile::NamedTempFile;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::cli::McpCommand;
use crate::output::{
    escape_json_terminal_controls, write_human_stdout, write_stderr, write_stdout,
};

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_SNAPSHOT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_RUNTIME_TOOLS: usize = 256;
const MAX_RUNTIME_CONTEXT_BYTES: usize = 1024 * 1024;
const MANIFEST_TEMPLATE: &str = r"# Pactrail MCP manifest. Server data is untrusted; this file assigns local authority.
# Add a [[servers]] table, run `pactrail mcp inspect <name>`, declare only the
# tools you trust, then run `pactrail mcp snapshot <name>` before enabling it.
schema = 1
";

#[derive(Debug, Error)]
pub(crate) enum McpCliError {
    #[error("{0}")]
    Argument(String),
    #[error("MCP operation failed: {0}")]
    Mcp(#[from] McpError),
    #[error("MCP file operation failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("MCP manifest serialization failed: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
    #[error("MCP snapshot JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("output failed: {0}")]
    Output(std::io::Error),
}

struct RuntimeEntry {
    config: Arc<McpServerConfig>,
    snapshot: Arc<McpSnapshot>,
}

/// Validated, connection-free set of enabled MCP snapshots for one run.
#[derive(Default)]
pub(crate) struct McpRuntime {
    entries: Vec<RuntimeEntry>,
    context: Vec<ContextFragment>,
    required_capabilities: BTreeSet<Capability>,
    snapshot_digests: Vec<String>,
    tool_count: usize,
}

impl McpRuntime {
    pub(crate) fn load(state: &Path) -> Result<Self, McpCliError> {
        let Some(manifest) = load_manifest_optional(state)? else {
            return Ok(Self::default());
        };
        let mut runtime = Self::default();
        let mut context_bytes = 0_usize;
        for config in manifest.enabled_servers() {
            let snapshot = read_snapshot(state, config)?.ok_or_else(|| {
                McpCliError::Argument(format!(
                    "enabled MCP server {:?} has no pinned snapshot; run `pactrail mcp snapshot {}`",
                    config.name, config.name
                ))
            })?;
            runtime.tool_count = runtime
                .tool_count
                .checked_add(snapshot.tools.len())
                .ok_or_else(|| McpCliError::Argument("MCP tool count overflowed".to_owned()))?;
            if runtime.tool_count > MAX_RUNTIME_TOOLS {
                return Err(McpCliError::Argument(format!(
                    "enabled MCP snapshots expose {} tools; the run limit is {MAX_RUNTIME_TOOLS}",
                    runtime.tool_count
                )));
            }
            for tool in &snapshot.tools {
                runtime
                    .required_capabilities
                    .extend(config.required_capabilities(&tool.profile));
            }
            let fragments = snapshot.context_fragments()?;
            for fragment in &fragments {
                context_bytes = context_bytes
                    .checked_add(fragment.source.len())
                    .and_then(|total| total.checked_add(fragment.content.len()))
                    .ok_or_else(|| {
                        McpCliError::Argument("MCP context size overflowed".to_owned())
                    })?;
            }
            if context_bytes > MAX_RUNTIME_CONTEXT_BYTES {
                return Err(McpCliError::Argument(format!(
                    "enabled MCP snapshots contain more than {MAX_RUNTIME_CONTEXT_BYTES} bytes of advisory context"
                )));
            }
            runtime.context.extend(fragments);
            runtime.snapshot_digests.push(snapshot.digest.clone());
            runtime.entries.push(RuntimeEntry {
                config: Arc::new(config.clone()),
                snapshot: Arc::new(snapshot),
            });
        }
        runtime.snapshot_digests.sort();
        Ok(runtime)
    }

    pub(crate) fn register(
        &self,
        registry: &mut ToolRegistry,
        cancellation: &CancellationToken,
    ) -> Result<usize, McpCliError> {
        let mut registered = 0_usize;
        for entry in &self.entries {
            registered = registered
                .checked_add(register_snapshot(
                    registry,
                    &entry.config,
                    &entry.snapshot,
                    cancellation,
                )?)
                .ok_or_else(|| McpCliError::Argument("MCP tool count overflowed".to_owned()))?;
        }
        Ok(registered)
    }

    pub(crate) fn context_fragments(&self) -> Vec<ContextFragment> {
        self.context.clone()
    }

    pub(crate) const fn required_capabilities(&self) -> &BTreeSet<Capability> {
        &self.required_capabilities
    }

    pub(crate) fn snapshot_digests(&self) -> &[String] {
        &self.snapshot_digests
    }

    pub(crate) const fn server_count(&self) -> usize {
        self.entries.len()
    }

    pub(crate) const fn tool_count(&self) -> usize {
        self.tool_count
    }
}

/// Validates the complete local MCP manifest/snapshot set without connecting.
pub(crate) fn validate_local_state(state: &Path) -> Result<usize, McpCliError> {
    let statuses = statuses(state)?;
    let failures = statuses
        .iter()
        .filter(|status| status.error.is_some() || status.enabled && status.snapshot != "valid")
        .map(|status| {
            format!(
                "{} ({})",
                status.name,
                status.error.as_deref().unwrap_or(status.snapshot)
            )
        })
        .collect::<Vec<_>>();
    if !failures.is_empty() {
        return Err(McpCliError::Argument(format!(
            "MCP state validation failed: {}",
            failures.join(", ")
        )));
    }
    let _runtime = McpRuntime::load(state)?;
    Ok(statuses.len())
}

#[derive(Serialize)]
struct ServerStatus {
    name: String,
    enabled: bool,
    transport: &'static str,
    target: String,
    environment_names: Vec<String>,
    snapshot: &'static str,
    tools: usize,
    context_items: usize,
    identity: Option<String>,
    digest: Option<String>,
    error: Option<String>,
}

pub(crate) async fn execute(
    state: &Path,
    workspace: &Path,
    command: McpCommand,
    cancellation: CancellationToken,
) -> Result<(), McpCliError> {
    match command {
        McpCommand::Init => init(state),
        McpCommand::Check { json } => check(state, json),
        McpCommand::List { json } => list(state, json),
        McpCommand::Inspect { server, json } => {
            inspect(state, workspace, &server, json, cancellation).await
        }
        McpCommand::Snapshot { server, json } => {
            snapshot(state, workspace, &server, json, cancellation).await
        }
        McpCommand::Enable { server } => set_enabled(state, &server, true),
        McpCommand::Disable { server } => set_enabled(state, &server, false),
    }
}

fn init(state: &Path) -> Result<(), McpCliError> {
    fs::create_dir_all(state).map_err(|source| McpCliError::Io {
        path: state.to_path_buf(),
        source,
    })?;
    let _lock = lock_state(state)?;
    let path = manifest_path(state);
    if path.exists() {
        return Err(McpCliError::Argument(format!(
            "refusing to overwrite existing MCP manifest {}",
            path.display()
        )));
    }
    write_atomic(&path, MANIFEST_TEMPLATE.as_bytes())?;
    write_human_stdout(&format!(
        "Created {}\nNo server is enabled and no connection was made.\n",
        path.display()
    ))
    .map_err(McpCliError::Output)
}

fn check(state: &Path, json: bool) -> Result<(), McpCliError> {
    let statuses = statuses(state)?;
    let enabled = statuses.iter().filter(|status| status.enabled).count();
    let tools = statuses.iter().map(|status| status.tools).sum::<usize>();
    let failures = statuses
        .iter()
        .filter(|status| status.error.is_some() || status.enabled && status.snapshot != "valid")
        .collect::<Vec<_>>();
    if !failures.is_empty() {
        return Err(McpCliError::Argument(format!(
            "MCP check failed: {}",
            failures
                .iter()
                .map(|status| format!(
                    "{} ({})",
                    status.name,
                    status.error.as_deref().unwrap_or(status.snapshot)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    // Enforce aggregate run bounds as part of the offline preflight.
    let runtime = McpRuntime::load(state)?;
    if json {
        return write_json(&serde_json::json!({
            "valid": true,
            "configured_servers": statuses.len(),
            "enabled_servers": enabled,
            "enabled_tools": runtime.tool_count(),
            "servers": statuses,
        }));
    }
    write_human_stdout(&format!(
        "MCP configuration valid\n  configured  {}\n  enabled     {}\n  tools       {}\n  discovery   not performed\n",
        statuses.len(), enabled, tools
    ))
    .map_err(McpCliError::Output)
}

fn list(state: &Path, json: bool) -> Result<(), McpCliError> {
    let statuses = statuses(state)?;
    if json {
        return write_json(&statuses);
    }
    if statuses.is_empty() {
        return write_human_stdout(
            "MCP\n  No servers configured. Run `pactrail mcp init` to create a manifest.\n",
        )
        .map_err(McpCliError::Output);
    }
    let mut lines = vec!["MCP servers".to_owned()];
    lines.push(format!(
        "{:<18} {:<9} {:<17} {:<10} {}",
        "NAME", "STATE", "TRANSPORT", "SNAPSHOT", "TOOLS"
    ));
    for status in statuses {
        lines.push(format!(
            "{:<18} {:<9} {:<17} {:<10} {}",
            status.name,
            if status.enabled {
                "enabled"
            } else {
                "disabled"
            },
            status.transport,
            status.snapshot,
            status.tools
        ));
        lines.push(format!("  {}", status.target));
        if let Some(error) = status.error {
            lines.push(format!("  error: {}", terminal_safe(&error, 512)));
        } else if let Some(identity) = status.identity {
            lines.push(format!("  pinned: {identity}"));
        }
    }
    write_human_stdout(&format!("{}\n", lines.join("\n"))).map_err(McpCliError::Output)
}

async fn inspect(
    state: &Path,
    workspace: &Path,
    server: &str,
    json: bool,
    cancellation: CancellationToken,
) -> Result<(), McpCliError> {
    let manifest = load_manifest_required(state)?;
    let config = find_server(&manifest, server)?;
    let workspace = canonical_workspace(workspace)?;
    render_connection_notice("Inspecting", config)?;
    let catalog = discover(config, &workspace, cancellation).await?;
    if json {
        return write_json(&catalog);
    }
    render_catalog(config, &catalog)
}

async fn snapshot(
    state: &Path,
    workspace: &Path,
    server: &str,
    json: bool,
    cancellation: CancellationToken,
) -> Result<(), McpCliError> {
    let manifest = load_manifest_required(state)?;
    let original_config = find_server(&manifest, server)?.clone();
    let workspace = canonical_workspace(workspace)?;
    render_connection_notice("Snapshotting", &original_config)?;
    let catalog = discover(&original_config, &workspace, cancellation).await?;
    let snapshot = McpSnapshot::build(&original_config, catalog)?;

    fs::create_dir_all(state).map_err(|source| McpCliError::Io {
        path: state.to_path_buf(),
        source,
    })?;
    let _lock = lock_state(state)?;
    let current = load_manifest_required(state)?;
    let current_config = find_server(&current, server)?;
    if current_config != &original_config {
        return Err(McpCliError::Argument(format!(
            "MCP server {server:?} changed during discovery; inspect and snapshot it again"
        )));
    }
    snapshot.validate(current_config)?;
    let bytes = serde_json::to_vec_pretty(&snapshot)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SNAPSHOT_BYTES {
        return Err(McpCliError::Argument(format!(
            "MCP snapshot exceeds the {MAX_SNAPSHOT_BYTES}-byte safety limit"
        )));
    }
    let path = snapshot_path(state, &original_config.name);
    write_atomic(&path, &bytes)?;
    if json {
        return write_json(&snapshot);
    }
    write_human_stdout(&format!(
        "Pinned MCP snapshot\n  server    {}\n  identity  {} {}\n  tools     {}\n  context   {} items\n  digest    {}\n  file      {}\n",
        original_config.name,
        terminal_safe(&snapshot.identity.name, 256),
        terminal_safe(&snapshot.identity.version, 256),
        snapshot.tools.len(),
        snapshot.context.len(),
        snapshot.digest,
        path.display()
    ))
    .map_err(McpCliError::Output)
}

fn set_enabled(state: &Path, server: &str, enabled: bool) -> Result<(), McpCliError> {
    fs::create_dir_all(state).map_err(|source| McpCliError::Io {
        path: state.to_path_buf(),
        source,
    })?;
    let _lock = lock_state(state)?;
    let mut manifest = load_manifest_required(state)?;
    let config = manifest
        .servers
        .iter_mut()
        .find(|config| config.name == server)
        .ok_or_else(|| McpCliError::Argument(format!("unknown MCP server {server:?}")))?;
    if enabled {
        let _snapshot = read_snapshot(state, config)?.ok_or_else(|| {
            McpCliError::Argument(format!(
                "server {server:?} has no valid snapshot; snapshot it before enabling"
            ))
        })?;
    }
    config.enabled = enabled;
    manifest.validate()?;
    let mut text = toml::to_string_pretty(&manifest)?;
    text.push('\n');
    write_atomic(&manifest_path(state), text.as_bytes())?;
    write_human_stdout(&format!(
        "MCP server {server} {}.\n",
        if enabled { "enabled" } else { "disabled" }
    ))
    .map_err(McpCliError::Output)
}

fn statuses(state: &Path) -> Result<Vec<ServerStatus>, McpCliError> {
    let Some(manifest) = load_manifest_optional(state)? else {
        return Ok(Vec::new());
    };
    Ok(manifest
        .servers
        .iter()
        .map(|config| {
            let (snapshot, tools, context_items, identity, digest, error) =
                match read_snapshot(state, config) {
                    Ok(Some(snapshot)) => (
                        "valid",
                        snapshot.tools.len(),
                        snapshot.context.len(),
                        Some(format!(
                            "{} {} ({})",
                            terminal_safe(&snapshot.identity.name, 256),
                            terminal_safe(&snapshot.identity.version, 256),
                            terminal_safe(&snapshot.identity.protocol_version, 256)
                        )),
                        Some(snapshot.digest),
                        None,
                    ),
                    Ok(None) => ("missing", 0, 0, None, None, None),
                    Err(error) => ("invalid", 0, 0, None, None, Some(error.to_string())),
                };
            ServerStatus {
                name: config.name.clone(),
                enabled: config.enabled,
                transport: config.transport.kind(),
                target: transport_target(&config.transport),
                environment_names: config.environment.iter().cloned().collect(),
                snapshot,
                tools,
                context_items,
                identity,
                digest,
                error,
            }
        })
        .collect())
}

fn render_catalog(
    config: &McpServerConfig,
    catalog: &McpDiscoveredCatalog,
) -> Result<(), McpCliError> {
    let mut lines = vec![format!(
        "MCP catalog (untrusted)\n  configured  {}\n  identity    {} {}\n  protocol    {}\n  tools       {}\n  context     {} selected items",
        config.name,
        terminal_safe(&catalog.identity.name, 256),
        terminal_safe(&catalog.identity.version, 256),
        terminal_safe(&catalog.identity.protocol_version, 256),
        catalog.tools.len(),
        catalog.context.len()
    )];
    if !catalog.tools.is_empty() {
        lines.push("\nAdvertised tools".to_owned());
        for tool in &catalog.tools {
            lines.push(format!(
                "  {}{}",
                terminal_safe(&tool.name, 256),
                if config.tools.contains_key(&tool.name) {
                    "  [profiled]"
                } else {
                    "  [not authorized]"
                }
            ));
        }
    }
    lines.push(
        "\nNo state changed. Add explicit local profiles in mcp.toml, then run `pactrail mcp snapshot <name>`."
            .to_owned(),
    );
    write_human_stdout(&format!("{}\n", lines.join("\n"))).map_err(McpCliError::Output)
}

fn render_connection_notice(action: &str, config: &McpServerConfig) -> Result<(), McpCliError> {
    write_stderr(&format!(
        "{action} MCP server {:?}\n  transport  {}\n  target     {}\n  secrets    {}\n",
        config.name,
        config.transport.kind(),
        transport_target(&config.transport),
        if config.environment.is_empty() {
            "none".to_owned()
        } else {
            config
                .environment
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        }
    ))
    .map_err(McpCliError::Output)
}

fn transport_target(transport: &McpTransportConfig) -> String {
    match transport {
        McpTransportConfig::Stdio { command, args } => {
            format!("{} ({} fixed args)", command.display(), args.len())
        }
        McpTransportConfig::StreamableHttp { url, .. } => url.clone(),
    }
}

fn terminal_safe(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for character in value.chars().take(max_chars) {
        if character.is_control() || character == '\u{1b}' {
            output.extend(character.escape_default());
        } else {
            output.push(character);
        }
    }
    if value.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

fn canonical_workspace(workspace: &Path) -> Result<PathBuf, McpCliError> {
    fs::canonicalize(workspace).map_err(|source| McpCliError::Io {
        path: workspace.to_path_buf(),
        source,
    })
}

fn find_server<'a>(
    manifest: &'a McpManifest,
    server: &str,
) -> Result<&'a McpServerConfig, McpCliError> {
    manifest
        .servers
        .iter()
        .find(|config| config.name == server)
        .ok_or_else(|| McpCliError::Argument(format!("unknown MCP server {server:?}")))
}

fn load_manifest_required(state: &Path) -> Result<McpManifest, McpCliError> {
    load_manifest_optional(state)?.ok_or_else(|| {
        McpCliError::Argument(format!(
            "MCP manifest {} does not exist; run `pactrail mcp init`",
            manifest_path(state).display()
        ))
    })
}

fn load_manifest_optional(state: &Path) -> Result<Option<McpManifest>, McpCliError> {
    let path = manifest_path(state);
    if !optional_regular_file(&path)? {
        return Ok(None);
    }
    let bytes = read_bounded(&path, MAX_MANIFEST_BYTES)?;
    let text = std::str::from_utf8(&bytes).map_err(|error| {
        McpCliError::Argument(format!(
            "MCP manifest {} is not UTF-8: {error}",
            path.display()
        ))
    })?;
    Ok(Some(McpManifest::from_toml(text)?))
}

fn read_snapshot(
    state: &Path,
    config: &McpServerConfig,
) -> Result<Option<McpSnapshot>, McpCliError> {
    let path = snapshot_path(state, &config.name);
    if !optional_regular_file(&path)? {
        return Ok(None);
    }
    let bytes = read_bounded(&path, MAX_SNAPSHOT_BYTES)?;
    let snapshot: McpSnapshot = serde_json::from_slice(&bytes)?;
    snapshot.validate(config)?;
    Ok(Some(snapshot))
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, McpCliError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| McpCliError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(McpCliError::Argument(format!(
            "MCP path {} is not a regular, non-symlink file",
            path.display()
        )));
    }
    if metadata.len() > limit {
        return Err(McpCliError::Argument(format!(
            "MCP file {} exceeds its {limit}-byte safety limit",
            path.display()
        )));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    fs::File::open(path)
        .and_then(|file| file.take(limit.saturating_add(1)).read_to_end(&mut bytes))
        .map_err(|source| McpCliError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(McpCliError::Argument(format!(
            "MCP file {} grew beyond its safety limit while being read",
            path.display()
        )));
    }
    Ok(bytes)
}

fn optional_regular_file(path: &Path) -> Result<bool, McpCliError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(McpCliError::Argument(format!(
                "MCP path {} is not a regular, non-symlink file",
                path.display()
            )))
        }
        Ok(_) => Ok(true),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(McpCliError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn lock_state(state: &Path) -> Result<fs::File, McpCliError> {
    let path = state.join("mcp.lock");
    let _exists = optional_regular_file(&path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|source| McpCliError::Io {
            path: path.clone(),
            source,
        })?;
    file.try_lock().map_err(|error| match error {
        fs::TryLockError::WouldBlock => McpCliError::Argument(
            "another Pactrail process is updating MCP state; retry after it finishes".to_owned(),
        ),
        fs::TryLockError::Error(source) => McpCliError::Io { path, source },
    })?;
    Ok(file)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), McpCliError> {
    let parent = path.parent().ok_or_else(|| {
        McpCliError::Argument(format!("MCP path {} has no parent", path.display()))
    })?;
    fs::create_dir_all(parent).map_err(|source| McpCliError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let parent_metadata = fs::symlink_metadata(parent).map_err(|source| McpCliError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(McpCliError::Argument(format!(
            "MCP directory {} is not a real local directory",
            parent.display()
        )));
    }
    let backup = path.with_extension(format!(
        "{}.bak",
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("file")
    ));
    let mut path_exists = optional_regular_file(path)?;
    let backup_exists = optional_regular_file(&backup)?;
    if backup_exists {
        if path_exists {
            fs::remove_file(&backup).map_err(|source| McpCliError::Io {
                path: backup.clone(),
                source,
            })?;
        } else {
            fs::rename(&backup, path).map_err(|source| McpCliError::Io {
                path: backup.clone(),
                source,
            })?;
            path_exists = true;
        }
    }
    let mut temporary = NamedTempFile::new_in(parent).map_err(|source| McpCliError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    temporary
        .write_all(bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|source| McpCliError::Io {
            path: temporary.path().to_path_buf(),
            source,
        })?;
    if path_exists {
        fs::rename(path, &backup).map_err(|source| McpCliError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    }
    if let Err(error) = temporary.persist(path) {
        if backup.exists() {
            let _restore = fs::rename(&backup, path);
        }
        return Err(McpCliError::Io {
            path: path.to_path_buf(),
            source: error.error,
        });
    }
    if backup.exists() {
        fs::remove_file(&backup).map_err(|source| McpCliError::Io {
            path: backup,
            source,
        })?;
    }
    Ok(())
}

fn manifest_path(state: &Path) -> PathBuf {
    state.join("mcp.toml")
}

fn snapshot_path(state: &Path, server: &str) -> PathBuf {
    state.join("mcp").join(format!("{server}.snapshot.json"))
}

fn write_json<T: Serialize + ?Sized>(value: &T) -> Result<(), McpCliError> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    write_stdout(&escape_json_terminal_controls(&text)).map_err(McpCliError::Output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_is_safe_and_refuses_to_overwrite() {
        let state = tempfile::tempdir().unwrap_or_else(|error| unreachable!("tempdir: {error}"));
        init(state.path()).unwrap_or_else(|error| unreachable!("init: {error}"));
        let manifest = load_manifest_required(state.path())
            .unwrap_or_else(|error| unreachable!("manifest: {error}"));
        assert!(manifest.servers.is_empty());
        assert!(init(state.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn manifest_reads_reject_symlinks() {
        use std::os::unix::fs::symlink;

        let state = tempfile::tempdir().unwrap_or_else(|error| unreachable!("state: {error}"));
        let outside = tempfile::tempdir().unwrap_or_else(|error| unreachable!("outside: {error}"));
        let external = outside.path().join("manifest.toml");
        fs::write(&external, "schema = 1\nservers = []\n")
            .unwrap_or_else(|error| unreachable!("external: {error}"));
        symlink(&external, manifest_path(state.path()))
            .unwrap_or_else(|error| unreachable!("manifest symlink: {error}"));
        assert!(matches!(
            load_manifest_optional(state.path()),
            Err(McpCliError::Argument(message)) if message.contains("non-symlink")
        ));
    }

    #[test]
    fn terminal_output_escapes_control_characters() {
        assert_eq!(terminal_safe("ok\u{1b}[31m", 32), "ok\\u{1b}[31m");
    }
}
