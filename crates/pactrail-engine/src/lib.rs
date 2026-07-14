//! Execution and orchestration engine for Pactrail.

mod engine;
mod verification;

pub use engine::{EngineError, RunEngine, RunOutcome};
pub use verification::{VerificationCommand, detect_verification_commands};
