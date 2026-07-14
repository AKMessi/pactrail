use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A security-sensitive effect requested by a tool or model.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Read files inside the workspace.
    FileRead,
    /// Write files inside the isolated workspace transaction.
    FileWrite,
    /// Spawn a local process.
    ProcessSpawn,
    /// Connect to a network resource.
    Network,
    /// Receive a named secret from the secret broker.
    SecretUse,
    /// Mutate a service outside the local execution boundary.
    ExternalWrite,
}

/// A resource restriction attached to a policy decision.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct ResourceScope {
    /// Capability being constrained.
    pub capability: Capability,
    /// Canonical resource selector, such as a path prefix or network host.
    pub resource: String,
    /// Optional tool or executable fingerprint.
    pub actor_fingerprint: Option<String>,
}

/// Result of evaluating one requested effect.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PolicyDecision {
    /// The effect is permitted within the returned scope.
    Allow {
        scope: ResourceScope,
        reason: String,
    },
    /// The effect is forbidden.
    Deny { reason: String },
    /// An interactive, narrowly scoped approval is required.
    Ask {
        scope: ResourceScope,
        reason: String,
    },
}

impl fmt::Display for Capability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::FileRead => "file_read",
            Self::FileWrite => "file_write",
            Self::ProcessSpawn => "process_spawn",
            Self::Network => "network",
            Self::SecretUse => "secret_use",
            Self::ExternalWrite => "external_write",
        };
        formatter.write_str(value)
    }
}
