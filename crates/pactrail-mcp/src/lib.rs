//! Governed Model Context Protocol integration for Pactrail.
//!
//! MCP servers are untrusted protocol peers. This crate converts an explicitly
//! captured, integrity-checked catalog into Pactrail tool contracts; it never
//! treats server descriptions, annotations, resources, or prompts as authority.

mod manifest;
mod schema;
mod snapshot;

pub use manifest::{
    MCP_MANIFEST_SCHEMA, McpManifest, McpPromptSelection, McpServerConfig, McpToolProfile,
    McpTransportConfig,
};
pub use schema::{MAX_MCP_SCHEMA_BYTES, validate_arguments, validate_input_schema};
pub use snapshot::{
    MCP_PROTOCOL_VERSION, MCP_SNAPSHOT_SCHEMA, McpContextCapture, McpContextKind,
    McpDiscoveredCatalog, McpDiscoveredTool, McpServerIdentity, McpSnapshot, McpSnapshotTool,
};

use thiserror::Error;

/// Configuration, protocol, validation, or execution failure at the MCP boundary.
#[derive(Debug, Error)]
pub enum McpError {
    #[error("MCP manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("MCP schema for {tool:?} is invalid: {reason}")]
    InvalidSchema { tool: String, reason: String },
    #[error("MCP snapshot is invalid: {0}")]
    InvalidSnapshot(String),
    #[error("MCP snapshot integrity failed: expected {expected}, computed {actual}")]
    SnapshotIntegrity { expected: String, actual: String },
    #[error("MCP server identity changed: {0}")]
    IdentityChanged(String),
    #[error("MCP transport failed: {0}")]
    Transport(String),
    #[error("MCP request timed out after {seconds} seconds during {operation}")]
    Timeout {
        operation: &'static str,
        seconds: u64,
    },
    #[error("MCP response exceeded the {limit}-byte limit")]
    ResponseTooLarge { limit: usize },
    #[error("MCP JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("MCP TOML parsing failed: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("MCP file operation failed at {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}
