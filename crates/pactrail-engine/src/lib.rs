//! Execution and orchestration engine for Pactrail.

mod context_window;
mod engine;
mod verification;

pub use engine::{EngineError, RunEngine, RunObserver, RunOutcome, RunProgress};
pub use verification::{VerificationCommand, detect_verification_commands};
