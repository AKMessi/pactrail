use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use pactrail_core::{Capability, PolicyDecision};
use pactrail_workspace::{TransactionError, WorkspaceTransaction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::PolicyEngine;

/// Machine-readable contract presented to a model for one tool.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub required_capability: Capability,
}

/// Bounded result of executing one tool call.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct ToolOutput {
    pub content: Value,
    pub summary: String,
    pub observed_effects: Vec<String>,
    pub succeeded: bool,
    pub truncated: bool,
}

/// Per-run capabilities made available to tools.
pub struct ToolContext<'a> {
    pub workspace: &'a WorkspaceTransaction,
    pub policy: &'a PolicyEngine,
}

impl ToolContext<'_> {
    /// Requires an allowed policy decision for one effect.
    ///
    /// # Errors
    ///
    /// Returns a policy error unless the effect is explicitly allowed.
    pub fn authorize(
        &self,
        capability: &Capability,
        resource: impl Into<String>,
        actor: &str,
    ) -> Result<(), ToolError> {
        match self
            .policy
            .evaluate(capability, resource, Some(actor.to_owned()))
        {
            PolicyDecision::Allow { .. } => Ok(()),
            PolicyDecision::Deny { reason } => Err(ToolError::Denied(reason)),
            PolicyDecision::Ask { scope, reason } => Err(ToolError::ApprovalRequired {
                capability: scope.capability,
                resource: scope.resource,
                reason,
            }),
        }
    }
}

/// A typed operation available to a model.
#[async_trait]
pub trait Tool: Send + Sync + 'static {
    /// Returns the stable tool contract.
    fn descriptor(&self) -> ToolDescriptor;

    /// Executes one validated JSON input value.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError`] for invalid input, policy rejection, unsafe paths,
    /// execution failure, or invalid output serialization.
    async fn execute(
        &self,
        context: &ToolContext<'_>,
        input: Value,
    ) -> Result<ToolOutput, ToolError>;
}

/// Deterministically ordered set of callable tools.
#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            tools: BTreeMap::new(),
        }
    }

    /// Registers a tool under its descriptor name.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError::DuplicateTool`] when the name is already registered.
    pub fn register<T: Tool>(&mut self, tool: T) -> Result<(), ToolError> {
        let name = tool.descriptor().name;
        if self.tools.contains_key(&name) {
            return Err(ToolError::DuplicateTool(name));
        }
        self.tools.insert(name, Arc::new(tool));
        Ok(())
    }

    /// Returns descriptors in stable name order.
    #[must_use]
    pub fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.tools.values().map(|tool| tool.descriptor()).collect()
    }

    /// Executes a registered tool.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError::UnknownTool`] or the tool's execution error.
    pub async fn execute(
        &self,
        name: &str,
        context: &ToolContext<'_>,
        input: Value,
    ) -> Result<ToolOutput, ToolError> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| ToolError::UnknownTool(name.to_owned()))?;
        tool.execute(context, input).await
    }
}

/// Tool registration, validation, policy, or execution failure.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("tool {0:?} is already registered")]
    DuplicateTool(String),
    #[error("unknown tool {0:?}")]
    UnknownTool(String),
    #[error("invalid input for {tool}: {source}")]
    InvalidInput {
        tool: &'static str,
        source: serde_json::Error,
    },
    #[error("tool output serialization failed: {0}")]
    Serialization(serde_json::Error),
    #[error("tool effect was denied: {0}")]
    Denied(String),
    #[error("{capability} on {resource:?} requires approval: {reason}")]
    ApprovalRequired {
        capability: Capability,
        resource: String,
        reason: String,
    },
    #[error("workspace operation failed: {0}")]
    Workspace(#[from] TransactionError),
    #[error("tool I/O failed at {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("process {program:?} could not be started: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("process {program:?} timed out after {seconds} seconds")]
    Timeout { program: String, seconds: u64 },
    #[error("process output task failed: {0}")]
    Join(tokio::task::JoinError),
    #[error("file is not valid UTF-8: {0}")]
    NonUtf8(std::path::PathBuf),
    #[error("requested range is invalid: {0}")]
    InvalidRange(String),
    #[error("replacement count was {actual}, expected {expected}")]
    ReplacementCount { expected: usize, actual: usize },
}
