//! Domain model and deterministic state machine for Pactrail.

mod approval;
mod contract;
mod event;
mod evidence;
mod id;
mod policy;
mod receipt;

pub use approval::{ApprovalBinding, ApprovalDecision, ApprovalRecord, ApprovalRequest};
pub use contract::{
    Budget, ContractError, Obligation, ObligationKind, PermissionSet, TaskContract,
};
pub use event::{
    ActionRecord, EventEnvelope, EventHash, RunEvent, RunSnapshot, RunState, StateError,
};
pub use evidence::{Evidence, EvidenceGrade, EvidenceKind, EvidenceStatus};
pub use id::{ApprovalId, EvidenceId, ObligationId, RunId};
pub use policy::{Capability, PolicyDecision, ResourceScope};
pub use receipt::{
    ChangeReceipt, FileChange, ReceiptError, ReceiptInput, ReceiptOutcome, VerificationSummary,
};

/// Version of Pactrail's persisted event envelope.
pub const EVENT_SCHEMA_VERSION: u32 = 1;
