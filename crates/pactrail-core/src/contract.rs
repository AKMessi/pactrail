use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{Capability, ObligationId};

/// A limit applied to one run.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Budget {
    /// Maximum wall-clock duration in seconds.
    pub wall_time_seconds: u64,
    /// Maximum aggregate model tokens. Zero means no explicit token limit.
    pub model_tokens: u64,
    /// Maximum provider cost in millionths of a US dollar. Zero means no cost limit.
    pub cost_microusd: u64,
    /// Maximum number of concurrently executing task nodes.
    pub max_concurrency: u16,
    /// Maximum number of model attempts across the run.
    pub max_model_attempts: u16,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            wall_time_seconds: 3_600,
            model_tokens: 250_000,
            cost_microusd: 10_000_000,
            max_concurrency: 4,
            max_model_attempts: 24,
        }
    }
}

/// Capabilities the task may request from Pactrail.
#[derive(Clone, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PermissionSet {
    /// Capabilities approved without another interactive decision.
    #[serde(default)]
    pub allow: BTreeSet<Capability>,
    /// Capabilities that must always be denied.
    #[serde(default)]
    pub deny: BTreeSet<Capability>,
}

impl PermissionSet {
    /// Returns an error if a capability appears in both sets.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::ConflictingPermission`] for an ambiguous capability.
    pub fn validate(&self) -> Result<(), ContractError> {
        if let Some(capability) = self.allow.intersection(&self.deny).next() {
            return Err(ContractError::ConflictingPermission(capability.clone()));
        }
        Ok(())
    }
}

/// The class of acceptance condition represented by an obligation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObligationKind {
    /// Behavior explicitly requested by the user.
    Functional,
    /// A check that protects existing behavior.
    Regression,
    /// A security or policy condition.
    Security,
    /// A quality, style, or maintainability condition.
    Quality,
    /// An obligation inferred by Pactrail rather than stated by the user.
    Inferred,
}

/// One independently verifiable acceptance condition.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Obligation {
    /// Stable identity used to connect evidence to this condition.
    pub id: ObligationId,
    /// Human-readable, testable condition.
    pub description: String,
    /// Condition category.
    pub kind: ObligationKind,
    /// Whether an unsatisfied obligation must fail the run.
    pub required: bool,
}

impl Obligation {
    /// Creates a required obligation.
    #[must_use]
    pub fn required(description: impl Into<String>, kind: ObligationKind) -> Self {
        Self {
            id: ObligationId::new(),
            description: description.into(),
            kind,
            required: true,
        }
    }
}

/// Versioned input contract governing a Pactrail run.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct TaskContract {
    /// Contract schema version.
    pub schema_version: u32,
    /// User-visible goal.
    pub goal: String,
    /// Absolute or invocation-relative workspace root.
    pub workspace_root: String,
    /// Workspace-relative path prefixes the run may modify.
    #[serde(default)]
    pub allowed_write_paths: Vec<String>,
    /// Explicitly excluded behavior or paths.
    #[serde(default)]
    pub out_of_scope: Vec<String>,
    /// Independently verifiable acceptance conditions.
    pub obligations: Vec<Obligation>,
    /// Run resource limits.
    #[serde(default)]
    pub budget: Budget,
    /// Baseline capability policy.
    #[serde(default)]
    pub permissions: PermissionSet,
    /// Optional pinned provider name.
    pub provider: Option<String>,
    /// Optional pinned model identifier.
    pub model: Option<String>,
}

impl TaskContract {
    /// Current task-contract schema version.
    pub const SCHEMA_VERSION: u32 = 1;

    /// Builds a minimal contract with a required functional obligation.
    #[must_use]
    pub fn new(goal: impl Into<String>, workspace_root: impl Into<String>) -> Self {
        let goal = goal.into();
        Self {
            schema_version: Self::SCHEMA_VERSION,
            obligations: vec![Obligation::required(
                goal.clone(),
                ObligationKind::Functional,
            )],
            goal,
            workspace_root: workspace_root.into(),
            allowed_write_paths: vec![".".to_owned()],
            out_of_scope: Vec::new(),
            budget: Budget::default(),
            permissions: PermissionSet::default(),
            provider: None,
            model: None,
        }
    }

    /// Validates invariants required by the execution kernel.
    ///
    /// # Errors
    ///
    /// Returns a [`ContractError`] describing the first invalid invariant.
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(ContractError::UnsupportedSchema(self.schema_version));
        }
        if self.goal.trim().is_empty() {
            return Err(ContractError::EmptyGoal);
        }
        if self.workspace_root.trim().is_empty() {
            return Err(ContractError::EmptyWorkspaceRoot);
        }
        if self.allowed_write_paths.is_empty() {
            return Err(ContractError::NoWriteScope);
        }
        if self.obligations.is_empty() {
            return Err(ContractError::NoObligations);
        }
        if self.budget.wall_time_seconds == 0 {
            return Err(ContractError::ZeroWallTime);
        }
        if self.budget.max_concurrency == 0 {
            return Err(ContractError::ZeroConcurrency);
        }
        if self.budget.max_model_attempts == 0 {
            return Err(ContractError::ZeroModelAttempts);
        }

        let mut obligation_ids = BTreeSet::new();
        for obligation in &self.obligations {
            if obligation.description.trim().is_empty() {
                return Err(ContractError::EmptyObligation(obligation.id));
            }
            if !obligation_ids.insert(obligation.id) {
                return Err(ContractError::DuplicateObligation(obligation.id));
            }
        }
        self.permissions.validate()
    }
}

/// Validation failure for a task contract.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ContractError {
    /// The serialized contract uses a schema this build cannot interpret.
    #[error("unsupported task contract schema version {0}")]
    UnsupportedSchema(u32),
    /// No meaningful goal was supplied.
    #[error("task goal cannot be empty")]
    EmptyGoal,
    /// No workspace root was supplied.
    #[error("workspace root cannot be empty")]
    EmptyWorkspaceRoot,
    /// No writable scope was supplied.
    #[error("at least one allowed write path is required")]
    NoWriteScope,
    /// No acceptance conditions were supplied.
    #[error("at least one obligation is required")]
    NoObligations,
    /// An obligation has no meaningful description.
    #[error("obligation {0} has an empty description")]
    EmptyObligation(ObligationId),
    /// Two obligations have the same identity.
    #[error("duplicate obligation id {0}")]
    DuplicateObligation(ObligationId),
    /// A capability was both allowed and denied.
    #[error("capability {0:?} cannot be both allowed and denied")]
    ConflictingPermission(Capability),
    /// A zero wall-time budget could never execute.
    #[error("wall-time budget must be greater than zero")]
    ZeroWallTime,
    /// A zero concurrency limit could never execute.
    #[error("maximum concurrency must be greater than zero")]
    ZeroConcurrency,
    /// A zero attempt limit could never invoke a model.
    #[error("maximum model attempts must be greater than zero")]
    ZeroModelAttempts,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_contract_is_valid() {
        let contract = TaskContract::new("repair the parser", ".");
        assert_eq!(contract.validate(), Ok(()));
    }

    #[test]
    fn duplicate_obligations_are_rejected() {
        let mut contract = TaskContract::new("repair the parser", ".");
        contract.obligations.push(contract.obligations[0].clone());
        assert!(matches!(
            contract.validate(),
            Err(ContractError::DuplicateObligation(_))
        ));
    }

    #[test]
    fn conflicting_permissions_are_rejected() {
        let mut contract = TaskContract::new("repair the parser", ".");
        contract.permissions.allow.insert(Capability::Network);
        contract.permissions.deny.insert(Capability::Network);
        assert_eq!(
            contract.validate(),
            Err(ContractError::ConflictingPermission(Capability::Network))
        );
    }
}
