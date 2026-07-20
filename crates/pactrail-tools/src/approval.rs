use std::sync::Mutex;

use pactrail_core::{ApprovalDecision, ApprovalRecord, ApprovalRequest, PolicyDecision};

use crate::ToolError;

/// Frontend boundary for resolving a narrowly scoped approval request.
pub trait ApprovalResolver: Send + Sync {
    /// Returns the user's decision for this exact, immutable request binding.
    fn resolve(&self, request: &ApprovalRequest) -> ApprovalDecision;
}

/// Policy and approval facts collected during one tool execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyAuditEntry {
    Evaluation(PolicyDecision),
    Approval(ApprovalRecord),
}

/// Bounded per-call audit buffer drained into the hash-linked run journal.
#[derive(Debug, Default)]
pub struct PolicyAuditLog {
    entries: Mutex<Vec<PolicyAuditEntry>>,
}

impl PolicyAuditLog {
    pub(crate) fn push(&self, entry: PolicyAuditEntry) -> Result<(), ToolError> {
        self.entries
            .lock()
            .map_err(|_| ToolError::PolicyAuditUnavailable)?
            .push(entry);
        Ok(())
    }

    /// Removes all entries in deterministic insertion order.
    ///
    /// # Errors
    ///
    /// Fails closed if another thread poisoned the audit buffer.
    pub fn drain(&self) -> Result<Vec<PolicyAuditEntry>, ToolError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| ToolError::PolicyAuditUnavailable)?;
        Ok(std::mem::take(&mut *entries))
    }
}
