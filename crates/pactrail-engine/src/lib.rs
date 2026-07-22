//! Execution and orchestration engine for Pactrail.

mod adaptive;
mod checkpoint;
mod context_window;
mod controller;
mod engine;
mod verification;

pub use adaptive::{AdaptiveRuntimeClass, AdaptiveRuntimeProfile};
pub use checkpoint::{
    CHECKPOINT_SCHEMA_VERSION, CheckpointError, CheckpointIdentity, CheckpointStore,
    MIN_CHECKPOINT_SCHEMA_VERSION, ResumePhase, RunCheckpoint, contract_digest,
};
pub use controller::ControllerPhase;
pub use engine::{EngineError, RunEngine, RunObserver, RunOutcome, RunProgress};
pub use verification::{VerificationCommand, detect_verification_commands};
