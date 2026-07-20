use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use pactrail_context::ContextError;
use pactrail_core::{
    ApprovalDecision, ApprovalRecord, ApprovalRequest, Capability, PolicyDecision, RunId,
};
use pactrail_memory::{MemoryError, MemoryStore};
use pactrail_workspace::{TransactionError, WorkspaceTransaction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::PolicyEngine;

pub(crate) fn replace_checked_preserving_newlines(
    text: &str,
    old: &str,
    new: &str,
    expected: usize,
) -> Result<(String, usize), usize> {
    let exact = text.matches(old).count();
    if exact == expected {
        return Ok((text.replace(old, new), exact));
    }
    if exact != 0 || (!old.contains('\r') && !old.contains('\n')) {
        return Err(exact);
    }

    let line_ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let adapted_old = adapt_line_endings(old, line_ending);
    if adapted_old == old {
        return Err(exact);
    }
    let adapted = text.matches(&adapted_old).count();
    if adapted != expected {
        return Err(adapted);
    }
    let adapted_new = adapt_line_endings(new, line_ending);
    Ok((text.replace(&adapted_old, &adapted_new), adapted))
}

fn adapt_line_endings(value: &str, line_ending: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', line_ending)
}

/// Machine-readable contract presented to a model for one tool.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub required_capability: Capability,
    pub annotations: ToolAnnotations,
}

/// Runtime and UX hints that do not weaken policy enforcement.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct ToolAnnotations {
    /// Whether the tool only observes bounded local state.
    pub read_only: bool,
    /// Whether repeating an identical call is expected to have the same effect.
    pub idempotent: bool,
    /// Whether calls may execute concurrently with other parallel-safe calls.
    pub parallel_safe: bool,
    /// Coarse risk shown in traces and tool discovery.
    pub risk: ToolRisk,
}

impl ToolAnnotations {
    /// Standard annotation set for deterministic read-only tools.
    pub const READ_ONLY: Self = Self {
        read_only: true,
        idempotent: true,
        parallel_safe: true,
        risk: ToolRisk::ReadOnly,
    };

    /// Standard annotation set for isolated workspace mutations.
    pub const WORKSPACE_MUTATION: Self = Self {
        read_only: false,
        idempotent: false,
        parallel_safe: false,
        risk: ToolRisk::WorkspaceMutation,
    };

    /// Standard annotation set for unsandboxed native processes.
    pub const HOST_EXECUTION: Self = Self {
        read_only: false,
        idempotent: false,
        parallel_safe: false,
        risk: ToolRisk::HostExecution,
    };

    /// Standard annotation set for a restricted external process boundary.
    pub const RESTRICTED_EXECUTION: Self = Self {
        read_only: false,
        idempotent: false,
        parallel_safe: false,
        risk: ToolRisk::RestrictedExecution,
    };
}

/// Human-readable tool risk class.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRisk {
    ReadOnly,
    WorkspaceMutation,
    RestrictedExecution,
    HostExecution,
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
    pub memory: Option<&'a MemoryStore>,
    pub run_id: Option<RunId>,
    pub approval_resolver: Option<&'a dyn crate::ApprovalResolver>,
    pub policy_audit: Option<&'a crate::PolicyAuditLog>,
}

impl<'a> ToolContext<'a> {
    /// Creates a context with no interactive approval resolver.
    #[must_use]
    pub const fn new(
        workspace: &'a WorkspaceTransaction,
        policy: &'a PolicyEngine,
        memory: Option<&'a MemoryStore>,
    ) -> Self {
        ToolContext {
            workspace,
            policy,
            memory,
            run_id: None,
            approval_resolver: None,
            policy_audit: None,
        }
    }

    /// Attaches run-scoped approval resolution and policy auditing.
    #[must_use]
    pub const fn with_policy_audit(
        mut self,
        run_id: RunId,
        resolver: Option<&'a dyn crate::ApprovalResolver>,
        audit: &'a crate::PolicyAuditLog,
    ) -> Self {
        self.run_id = Some(run_id);
        self.approval_resolver = resolver;
        self.policy_audit = Some(audit);
        self
    }

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

    /// Resolves a request whose exact actor and enforcement boundary are approval-bound.
    ///
    /// # Errors
    ///
    /// Denies missing, expired, malformed, or negative decisions and fails closed if the
    /// policy audit record cannot be retained.
    pub fn authorize_request(&self, request: ApprovalRequest) -> Result<(), ToolError> {
        let decision = self.policy.evaluate(
            &request.binding.capability,
            request.binding.resource.clone(),
            Some(request.binding.actor_fingerprint.clone()),
        );
        match decision {
            PolicyDecision::Allow { .. } => Ok(()),
            PolicyDecision::Deny { reason } => Err(ToolError::Denied(reason)),
            PolicyDecision::Ask { scope, reason } => {
                if scope.capability != request.binding.capability
                    || scope.resource != request.binding.resource
                    || scope.actor_fingerprint.as_deref()
                        != Some(request.binding.actor_fingerprint.as_str())
                    || self.run_id != Some(request.binding.run_id)
                {
                    return Err(ToolError::Denied(
                        "approval request does not match the evaluated policy scope".to_owned(),
                    ));
                }
                self.audit(crate::PolicyAuditEntry::Evaluation(PolicyDecision::Ask {
                    scope,
                    reason,
                }))?;
                let Some(resolver) = self.approval_resolver else {
                    return Err(ToolError::ApprovalRequired {
                        capability: request.binding.capability,
                        resource: request.binding.resource,
                        reason: request.reason,
                    });
                };
                let record =
                    ApprovalRecord::new(request.binding.clone(), resolver.resolve(&request));
                self.audit(crate::PolicyAuditEntry::Approval(record.clone()))?;
                if !record.is_valid_at(record.created_at) {
                    return Err(ToolError::Denied(
                        "approval decision is expired or has an unsupported schema".to_owned(),
                    ));
                }
                match record.decision {
                    ApprovalDecision::AllowOnce | ApprovalDecision::AllowRun => Ok(()),
                    ApprovalDecision::Deny => Err(ToolError::Denied(
                        "the scoped approval request was denied".to_owned(),
                    )),
                }
            }
        }
    }

    fn audit(&self, entry: crate::PolicyAuditEntry) -> Result<(), ToolError> {
        self.policy_audit.map_or_else(
            || Err(ToolError::PolicyAuditUnavailable),
            |audit| audit.push(entry),
        )
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

    /// Returns one registered descriptor without executing the tool.
    #[must_use]
    pub fn descriptor(&self, name: &str) -> Option<ToolDescriptor> {
        self.tools.get(name).map(|tool| tool.descriptor())
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
    #[error("policy audit buffer is unavailable")]
    PolicyAuditUnavailable,
    #[error("workspace operation failed: {0}")]
    Workspace(#[from] TransactionError),
    #[error("repository evidence graph failed: {0}")]
    RepositoryGraph(#[from] ContextError),
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
    #[error("process backend failed: {0}")]
    ProcessBackend(#[from] crate::ProcessBackendError),
    #[error("file is not valid UTF-8: {0}")]
    NonUtf8(std::path::PathBuf),
    #[error("requested range is invalid: {0}")]
    InvalidRange(String),
    #[error("workspace memory is unavailable for this run")]
    MemoryUnavailable,
    #[error("workspace memory failed: {0}")]
    Memory(#[from] MemoryError),
    #[error("replacement count was {actual}, expected {expected}")]
    ReplacementCount { expected: usize, actual: usize },
    #[error("edit {index} replacement count was {actual}, expected {expected}")]
    InvalidEditCount {
        index: usize,
        expected: usize,
        actual: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::replace_checked_preserving_newlines;

    #[test]
    fn checked_replacement_accepts_lf_model_text_for_crlf_file() {
        let source = "before\r\nold line\r\nafter\r\n";
        let (result, count) = replace_checked_preserving_newlines(
            source,
            "before\nold line\nafter",
            "before\nnew line\nafter",
            1,
        )
        .unwrap_or_else(|actual| unreachable!("newline-equivalent replacement: {actual}"));
        assert_eq!(count, 1);
        assert_eq!(result, "before\r\nnew line\r\nafter\r\n");
    }

    #[test]
    fn checked_replacement_does_not_relax_content_matching() {
        let source = "before\r\nold line\r\nafter\r\n";
        assert_eq!(
            replace_checked_preserving_newlines(
                source,
                "before\nwrong line\nafter",
                "replacement",
                1,
            ),
            Err(0)
        );
    }
}
