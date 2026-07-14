use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{EvidenceId, ObligationId};

/// Strength of the support represented by an evidence record.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceGrade {
    /// No supporting check is available.
    Unverified,
    /// A model or heuristic assessed the result.
    ModelAssessed,
    /// Pactrail captured concrete runtime or filesystem behavior.
    Observed,
    /// A reproducible compiler, test, diagnostic, or policy check produced it.
    Deterministic,
}

/// What produced an evidence record.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Test,
    Build,
    Diagnostic,
    Policy,
    FileObservation,
    ProcessObservation,
    ModelReview,
    UserAssertion,
    Other,
}

/// Whether the observed result supports an obligation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    Passed,
    Failed,
    Inconclusive,
    Skipped,
}

/// Provenance-bearing support for one task obligation.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Evidence {
    pub id: EvidenceId,
    pub obligation_id: ObligationId,
    pub grade: EvidenceGrade,
    pub kind: EvidenceKind,
    pub status: EvidenceStatus,
    pub summary: String,
    /// Optional content-addressed artifact containing full output.
    pub artifact_digest: Option<String>,
    /// Reproduction command when one exists.
    pub reproduction: Option<String>,
}

impl Evidence {
    /// Creates a deterministic passing evidence record.
    #[must_use]
    pub fn deterministic_pass(
        obligation_id: ObligationId,
        kind: EvidenceKind,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            id: EvidenceId::new(),
            obligation_id,
            grade: EvidenceGrade::Deterministic,
            kind,
            status: EvidenceStatus::Passed,
            summary: summary.into(),
            artifact_digest: None,
            reproduction: None,
        }
    }
}
