use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;

use crate::{EVENT_SCHEMA_VERSION, Evidence, PolicyDecision, RunId, TaskContract};

/// Integrity hash of an event envelope.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(transparent)]
pub struct EventHash(pub String);

impl EventHash {
    /// Hash used before the first event in a run.
    #[must_use]
    pub fn genesis() -> Self {
        Self("0".repeat(64))
    }
}

/// Lifecycle state of a Pactrail run.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Created,
    Contracting,
    Investigating,
    Planning,
    Executing,
    Verifying,
    Reviewing,
    /// A read-only task produced a final answer and has nothing to apply.
    Completed,
    AwaitingApply,
    Applied,
    Discarded,
    Failed,
    Cancelled,
}

impl RunState {
    /// Whether no further state transition is valid.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Applied | Self::Discarded | Self::Failed | Self::Cancelled
        )
    }

    fn can_transition_to(self, next: Self) -> bool {
        if next == Self::Failed || next == Self::Cancelled {
            return !self.is_terminal();
        }
        matches!(
            (self, next),
            (Self::Created, Self::Contracting)
                | (Self::Contracting, Self::Investigating)
                | (Self::Investigating, Self::Planning | Self::Executing)
                | (
                    Self::Planning | Self::Verifying | Self::Reviewing,
                    Self::Executing
                )
                | (Self::Executing, Self::Verifying)
                | (Self::Verifying, Self::Reviewing)
                | (Self::Reviewing, Self::Completed | Self::AwaitingApply)
                | (Self::AwaitingApply, Self::Applied | Self::Discarded)
        )
    }
}

/// Auditable description of one tool or model action.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct ActionRecord {
    pub actor: String,
    pub action: String,
    pub summary: String,
    #[serde(default)]
    pub declared_effects: Vec<String>,
    #[serde(default)]
    pub observed_effects: Vec<String>,
    pub succeeded: bool,
    /// End-to-end action duration. Zero denotes an older event without timing data.
    #[serde(default)]
    pub duration_ms: u64,
    /// Bounded non-sensitive diagnostic fields with stable string values.
    #[serde(default)]
    pub attributes: std::collections::BTreeMap<String, String>,
}

/// Payload variants accepted by the deterministic run reducer.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum RunEvent {
    ContractRegistered(TaskContract),
    StateChanged { from: RunState, to: RunState },
    ActionCompleted(ActionRecord),
    EvidenceRecorded(Evidence),
    PolicyEvaluated(PolicyDecision),
    CheckpointCreated { checkpoint: String },
    NoteRecorded { message: String },
}

/// Durable, hash-linked event record.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct EventEnvelope {
    pub schema_version: u32,
    pub run_id: RunId,
    pub sequence: u64,
    #[schemars(with = "String")]
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
    pub previous_hash: EventHash,
    pub event: RunEvent,
    pub hash: EventHash,
}

impl EventEnvelope {
    /// Constructs and hashes an envelope from canonical JSON fields.
    ///
    /// # Errors
    ///
    /// Returns an error if the event cannot be serialized as canonical JSON.
    pub fn new(
        run_id: RunId,
        sequence: u64,
        timestamp: OffsetDateTime,
        previous_hash: EventHash,
        event: RunEvent,
    ) -> Result<Self, serde_json::Error> {
        #[derive(Serialize)]
        struct Hashable<'a> {
            schema_version: u32,
            run_id: RunId,
            sequence: u64,
            #[serde(with = "time::serde::rfc3339")]
            timestamp: OffsetDateTime,
            previous_hash: &'a EventHash,
            event: &'a RunEvent,
        }

        let hashable = Hashable {
            schema_version: EVENT_SCHEMA_VERSION,
            run_id,
            sequence,
            timestamp,
            previous_hash: &previous_hash,
            event: &event,
        };
        let bytes = serde_json::to_vec(&hashable)?;
        let hash = EventHash(blake3::hash(&bytes).to_hex().to_string());
        Ok(Self {
            schema_version: EVENT_SCHEMA_VERSION,
            run_id,
            sequence,
            timestamp,
            previous_hash,
            event,
            hash,
        })
    }

    /// Recomputes the envelope hash and compares it to the stored value.
    ///
    /// # Errors
    ///
    /// Returns an error if the event cannot be serialized for verification.
    pub fn verify(&self) -> Result<bool, serde_json::Error> {
        Self::new(
            self.run_id,
            self.sequence,
            self.timestamp,
            self.previous_hash.clone(),
            self.event.clone(),
        )
        .map(|expected| expected.hash == self.hash)
    }
}

/// Deterministic projection of all events in a run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunSnapshot {
    pub run_id: RunId,
    pub state: RunState,
    pub contract: Option<TaskContract>,
    pub evidence: Vec<Evidence>,
    pub actions: Vec<ActionRecord>,
    pub last_sequence: Option<u64>,
    pub last_hash: EventHash,
}

impl RunSnapshot {
    /// Creates an empty projection.
    #[must_use]
    pub fn new(run_id: RunId) -> Self {
        Self {
            run_id,
            state: RunState::Created,
            contract: None,
            evidence: Vec::new(),
            actions: Vec::new(),
            last_sequence: None,
            last_hash: EventHash::genesis(),
        }
    }

    /// Applies one verified event while enforcing sequence and lifecycle invariants.
    ///
    /// # Errors
    ///
    /// Returns a [`StateError`] when integrity, sequence, contract, or lifecycle invariants fail.
    pub fn apply(&mut self, envelope: &EventEnvelope) -> Result<(), StateError> {
        if envelope.schema_version != EVENT_SCHEMA_VERSION {
            return Err(StateError::UnsupportedSchema(envelope.schema_version));
        }
        if envelope.run_id != self.run_id {
            return Err(StateError::WrongRun {
                expected: self.run_id,
                actual: envelope.run_id,
            });
        }
        let expected_sequence = self.last_sequence.map_or(0, |value| value + 1);
        if envelope.sequence != expected_sequence {
            return Err(StateError::UnexpectedSequence {
                expected: expected_sequence,
                actual: envelope.sequence,
            });
        }
        if envelope.previous_hash != self.last_hash {
            return Err(StateError::BrokenHashChain);
        }
        if !envelope.verify().map_err(StateError::Serialization)? {
            return Err(StateError::InvalidHash);
        }

        match &envelope.event {
            RunEvent::ContractRegistered(contract) => {
                if self.contract.is_some() {
                    return Err(StateError::DuplicateContract);
                }
                contract.validate().map_err(StateError::InvalidContract)?;
                self.contract = Some(contract.clone());
            }
            RunEvent::StateChanged { from, to } => {
                if *from != self.state {
                    return Err(StateError::StaleState {
                        expected: self.state,
                        actual: *from,
                    });
                }
                if !from.can_transition_to(*to) {
                    return Err(StateError::InvalidTransition {
                        from: *from,
                        to: *to,
                    });
                }
                self.state = *to;
            }
            RunEvent::ActionCompleted(action) => self.actions.push(action.clone()),
            RunEvent::EvidenceRecorded(evidence) => self.evidence.push(evidence.clone()),
            RunEvent::PolicyEvaluated(_)
            | RunEvent::CheckpointCreated { .. }
            | RunEvent::NoteRecorded { .. } => {}
        }

        self.last_sequence = Some(envelope.sequence);
        self.last_hash = envelope.hash.clone();
        Ok(())
    }
}

/// A run event violated deterministic projection invariants.
#[derive(Debug, Error)]
pub enum StateError {
    #[error("unsupported event schema version {0}")]
    UnsupportedSchema(u32),
    #[error("event belongs to run {actual}, expected {expected}")]
    WrongRun { expected: RunId, actual: RunId },
    #[error("event sequence {actual} was received, expected {expected}")]
    UnexpectedSequence { expected: u64, actual: u64 },
    #[error("event does not continue the previous hash chain")]
    BrokenHashChain,
    #[error("event integrity hash is invalid")]
    InvalidHash,
    #[error("run contract was registered more than once")]
    DuplicateContract,
    #[error("task contract is invalid: {0}")]
    InvalidContract(#[from] crate::ContractError),
    #[error("state transition used stale state {actual:?}; current state is {expected:?}")]
    StaleState {
        expected: RunState,
        actual: RunState,
    },
    #[error("invalid run state transition from {from:?} to {to:?}")]
    InvalidTransition { from: RunState, to: RunState },
    #[error("failed to canonicalize event: {0}")]
    Serialization(serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(run_id: RunId, sequence: u64, previous: EventHash, event: RunEvent) -> EventEnvelope {
        EventEnvelope::new(
            run_id,
            sequence,
            OffsetDateTime::UNIX_EPOCH,
            previous,
            event,
        )
        .unwrap_or_else(|error| unreachable!("test event serializes: {error}"))
    }

    #[test]
    fn state_machine_accepts_fast_path() {
        let run_id = RunId::new();
        let mut snapshot = RunSnapshot::new(run_id);
        let contract = event(
            run_id,
            0,
            EventHash::genesis(),
            RunEvent::ContractRegistered(TaskContract::new("fix bug", ".")),
        );
        snapshot
            .apply(&contract)
            .unwrap_or_else(|error| unreachable!("valid contract event: {error}"));

        let contracting = event(
            run_id,
            1,
            contract.hash,
            RunEvent::StateChanged {
                from: RunState::Created,
                to: RunState::Contracting,
            },
        );
        snapshot
            .apply(&contracting)
            .unwrap_or_else(|error| unreachable!("valid transition: {error}"));
        assert_eq!(snapshot.state, RunState::Contracting);
    }

    #[test]
    fn read_only_runs_can_complete_without_an_apply_state() {
        let run_id = RunId::new();
        let mut snapshot = RunSnapshot::new(run_id);
        let states = [
            RunState::Contracting,
            RunState::Investigating,
            RunState::Planning,
            RunState::Executing,
            RunState::Verifying,
            RunState::Reviewing,
            RunState::Completed,
        ];
        let contract = event(
            run_id,
            0,
            EventHash::genesis(),
            RunEvent::ContractRegistered(TaskContract::new("explain this repository", ".")),
        );
        snapshot
            .apply(&contract)
            .unwrap_or_else(|error| unreachable!("contract: {error}"));
        let mut previous_hash = contract.hash;
        let mut previous_state = RunState::Created;
        for (index, state) in states.into_iter().enumerate() {
            let transition = event(
                run_id,
                u64::try_from(index).unwrap_or_default() + 1,
                previous_hash,
                RunEvent::StateChanged {
                    from: previous_state,
                    to: state,
                },
            );
            previous_hash = transition.hash.clone();
            previous_state = state;
            snapshot
                .apply(&transition)
                .unwrap_or_else(|error| unreachable!("transition: {error}"));
        }

        assert_eq!(snapshot.state, RunState::Completed);
        assert!(snapshot.state.is_terminal());
    }

    #[test]
    fn tampered_event_is_rejected() {
        let run_id = RunId::new();
        let mut envelope = event(
            run_id,
            0,
            EventHash::genesis(),
            RunEvent::NoteRecorded {
                message: "original".to_owned(),
            },
        );
        envelope.event = RunEvent::NoteRecorded {
            message: "tampered".to_owned(),
        };
        let mut snapshot = RunSnapshot::new(run_id);
        assert!(matches!(
            snapshot.apply(&envelope),
            Err(StateError::InvalidHash)
        ));
    }

    #[test]
    fn illegal_transition_is_rejected() {
        let run_id = RunId::new();
        let envelope = event(
            run_id,
            0,
            EventHash::genesis(),
            RunEvent::StateChanged {
                from: RunState::Created,
                to: RunState::Applied,
            },
        );
        let mut snapshot = RunSnapshot::new(run_id);
        assert!(matches!(
            snapshot.apply(&envelope),
            Err(StateError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn legacy_action_records_default_trace_fields() {
        let action: ActionRecord = serde_json::from_value(serde_json::json!({
            "actor": "tool:read_file",
            "action": "read_file",
            "summary": "read one file",
            "declared_effects": ["file_read"],
            "observed_effects": ["fs.read:README.md"],
            "succeeded": true
        }))
        .unwrap_or_else(|error| unreachable!("legacy action: {error}"));
        assert_eq!(action.duration_ms, 0);
        assert!(action.attributes.is_empty());
    }
}
