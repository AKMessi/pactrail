use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;

use crate::{Evidence, EvidenceGrade, EvidenceStatus, ObligationId, RunId, TaskContract};

/// How one workspace path changed.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct FileChange {
    pub path: String,
    pub before_digest: Option<String>,
    pub after_digest: Option<String>,
    /// Unix permission bits before the change, when available.
    pub before_unix_mode: Option<u32>,
    /// Unix permission bits after the change, when available.
    pub after_unix_mode: Option<u32>,
    pub bytes_added: u64,
    pub bytes_removed: u64,
}

/// Terminal outcome represented by a change receipt.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptOutcome {
    /// A read-only task returned an evidence-backed answer with no candidate changes.
    Answered,
    ReadyToApply,
    Applied,
    Discarded,
    Failed,
    Cancelled,
}

/// Aggregated verification status without hiding individual evidence.
#[derive(Clone, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct VerificationSummary {
    pub passed: u32,
    pub failed: u32,
    pub inconclusive: u32,
    pub skipped: u32,
    pub highest_grade: Option<EvidenceGrade>,
}

/// Portable, evidence-backed summary of a completed run.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct ChangeReceipt {
    pub schema_version: u32,
    pub run_id: RunId,
    #[schemars(with = "String")]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub contract: TaskContract,
    pub outcome: ReceiptOutcome,
    pub baseline_digest: String,
    pub final_event_hash: String,
    pub changes: Vec<FileChange>,
    pub evidence: Vec<Evidence>,
    pub verification: VerificationSummary,
    pub unresolved_risks: Vec<String>,
    pub metadata: BTreeMap<String, String>,
    pub integrity_hash: String,
}

/// Inputs required to construct a [`ChangeReceipt`].
#[derive(Clone, Debug)]
pub struct ReceiptInput {
    pub run_id: RunId,
    pub contract: TaskContract,
    pub outcome: ReceiptOutcome,
    pub baseline_digest: String,
    pub final_event_hash: String,
    pub changes: Vec<FileChange>,
    pub evidence: Vec<Evidence>,
    pub unresolved_risks: Vec<String>,
}

impl ChangeReceipt {
    /// Current receipt schema version.
    pub const SCHEMA_VERSION: u32 = 2;

    /// Builds and signs a receipt with a content integrity hash.
    ///
    /// # Errors
    ///
    /// Returns a [`ReceiptError`] when evidence references an unknown obligation,
    /// a required obligation has no evidence, or canonical serialization fails.
    pub fn build(input: ReceiptInput) -> Result<Self, ReceiptError> {
        let required_ids: std::collections::BTreeSet<ObligationId> = input
            .contract
            .obligations
            .iter()
            .filter(|obligation| obligation.required)
            .map(|obligation| obligation.id)
            .collect();
        for record in &input.evidence {
            if !input
                .contract
                .obligations
                .iter()
                .any(|obligation| obligation.id == record.obligation_id)
            {
                return Err(ReceiptError::UnknownObligation(record.obligation_id));
            }
        }
        let evidenced_ids: std::collections::BTreeSet<ObligationId> = input
            .evidence
            .iter()
            .map(|record| record.obligation_id)
            .collect();
        let missing: Vec<ObligationId> = required_ids.difference(&evidenced_ids).copied().collect();
        if !missing.is_empty() {
            return Err(ReceiptError::MissingEvidence(missing));
        }

        let verification = summarize(&input.evidence);
        let mut receipt = Self {
            schema_version: Self::SCHEMA_VERSION,
            run_id: input.run_id,
            created_at: OffsetDateTime::now_utc(),
            contract: input.contract,
            outcome: input.outcome,
            baseline_digest: input.baseline_digest,
            final_event_hash: input.final_event_hash,
            changes: input.changes,
            evidence: input.evidence,
            verification,
            unresolved_risks: input.unresolved_risks,
            metadata: BTreeMap::new(),
            integrity_hash: String::new(),
        };
        receipt.integrity_hash = receipt.compute_hash()?;
        Ok(receipt)
    }

    /// Checks that the receipt has not been modified since construction.
    ///
    /// # Errors
    ///
    /// Returns a serialization error if the receipt cannot be canonicalized.
    pub fn verify_integrity(&self) -> Result<bool, ReceiptError> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(ReceiptError::UnsupportedSchema(self.schema_version));
        }
        self.compute_hash()
            .map(|expected| expected == self.integrity_hash)
    }

    fn compute_hash(&self) -> Result<String, ReceiptError> {
        let mut hashable = self.clone();
        hashable.integrity_hash.clear();
        serde_json::to_vec(&hashable)
            .map(|bytes| blake3::hash(&bytes).to_hex().to_string())
            .map_err(ReceiptError::Serialization)
    }
}

fn summarize(evidence: &[Evidence]) -> VerificationSummary {
    let mut summary = VerificationSummary::default();
    for record in evidence {
        match record.status {
            EvidenceStatus::Passed => summary.passed += 1,
            EvidenceStatus::Failed => summary.failed += 1,
            EvidenceStatus::Inconclusive => summary.inconclusive += 1,
            EvidenceStatus::Skipped => summary.skipped += 1,
        }
        summary.highest_grade = Some(
            summary
                .highest_grade
                .map_or(record.grade, |current| current.max(record.grade)),
        );
    }
    summary
}

/// Receipt construction or verification failed.
#[derive(Debug, Error)]
pub enum ReceiptError {
    #[error("unsupported change receipt schema version {0}")]
    UnsupportedSchema(u32),
    #[error("evidence refers to unknown obligation {0}")]
    UnknownObligation(ObligationId),
    #[error("required obligations are missing evidence: {0:?}")]
    MissingEvidence(Vec<ObligationId>),
    #[error("failed to serialize receipt: {0}")]
    Serialization(serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EvidenceKind;

    #[test]
    fn receipt_requires_evidence_for_every_required_obligation() {
        let contract = TaskContract::new("fix bug", ".");
        let result = ChangeReceipt::build(ReceiptInput {
            run_id: RunId::new(),
            contract,
            outcome: ReceiptOutcome::ReadyToApply,
            baseline_digest: "baseline".to_owned(),
            final_event_hash: "event".to_owned(),
            changes: Vec::new(),
            evidence: Vec::new(),
            unresolved_risks: Vec::new(),
        });
        assert!(matches!(result, Err(ReceiptError::MissingEvidence(_))));
    }

    #[test]
    fn answered_receipts_are_integrity_checked_without_changes() {
        let contract = TaskContract::new("explain this repository", ".");
        let obligation_id = contract.obligations[0].id;
        let receipt = ChangeReceipt::build(ReceiptInput {
            run_id: RunId::new(),
            contract,
            outcome: ReceiptOutcome::Answered,
            baseline_digest: "baseline".to_owned(),
            final_event_hash: "event".to_owned(),
            changes: Vec::new(),
            evidence: vec![Evidence {
                id: crate::EvidenceId::new(),
                obligation_id,
                kind: EvidenceKind::FileObservation,
                grade: EvidenceGrade::Observed,
                status: EvidenceStatus::Passed,
                summary: "repository files were inspected".to_owned(),
                artifact_digest: None,
                reproduction: None,
            }],
            unresolved_risks: Vec::new(),
        })
        .unwrap_or_else(|error| unreachable!("receipt: {error}"));

        assert_eq!(receipt.outcome, ReceiptOutcome::Answered);
        assert!(
            receipt
                .verify_integrity()
                .unwrap_or_else(|error| unreachable!("integrity: {error}"))
        );
    }

    #[test]
    fn receipt_detects_tampering() {
        let contract = TaskContract::new("fix bug", ".");
        let evidence = vec![Evidence::deterministic_pass(
            contract.obligations[0].id,
            EvidenceKind::Test,
            "tests passed",
        )];
        let mut receipt = ChangeReceipt::build(ReceiptInput {
            run_id: RunId::new(),
            contract,
            outcome: ReceiptOutcome::ReadyToApply,
            baseline_digest: "baseline".to_owned(),
            final_event_hash: "event".to_owned(),
            changes: Vec::new(),
            evidence,
            unresolved_risks: Vec::new(),
        })
        .unwrap_or_else(|error| unreachable!("valid receipt: {error}"));
        assert_eq!(receipt.verify_integrity().ok(), Some(true));
        receipt.baseline_digest = "tampered".to_owned();
        assert_eq!(receipt.verify_integrity().ok(), Some(false));
    }

    #[test]
    fn receipt_rejects_unknown_schema() {
        let contract = TaskContract::new("fix bug", ".");
        let evidence = vec![Evidence::deterministic_pass(
            contract.obligations[0].id,
            EvidenceKind::Test,
            "tests passed",
        )];
        let mut receipt = ChangeReceipt::build(ReceiptInput {
            run_id: RunId::new(),
            contract,
            outcome: ReceiptOutcome::ReadyToApply,
            baseline_digest: "baseline".to_owned(),
            final_event_hash: "event".to_owned(),
            changes: Vec::new(),
            evidence,
            unresolved_risks: Vec::new(),
        })
        .unwrap_or_else(|error| unreachable!("valid receipt: {error}"));
        receipt.schema_version = u32::MAX;
        assert!(matches!(
            receipt.verify_integrity(),
            Err(ReceiptError::UnsupportedSchema(u32::MAX))
        ));
    }
}
