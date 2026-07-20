//! Execution and orchestration engine for Pactrail.

mod checkpoint;
mod context_window;
mod engine;
mod verification;

pub use checkpoint::{
    CheckpointError, CheckpointIdentity, CheckpointStore, ResumePhase, RunCheckpoint,
    contract_digest,
};
pub use engine::{EngineError, RunEngine, RunObserver, RunOutcome, RunProgress};
pub use verification::{VerificationCommand, detect_verification_commands};
