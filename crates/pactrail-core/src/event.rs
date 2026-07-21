use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;

use crate::{
    ApprovalRecord, EVENT_SCHEMA_VERSION, Evidence, MIN_EVENT_SCHEMA_VERSION, PolicyDecision,
    RunId, TaskContract,
};

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
    pub attributes: BTreeMap<String, String>,
}

/// Write-ahead fence proving that one model-requested effect was admitted
/// before the tool implementation received control.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct EffectPrepared {
    pub call_id: String,
    pub tool: String,
    pub arguments_digest: String,
    pub candidate_digest_before: String,
    pub risk: String,
    pub runtime_profile_digest: String,
}

/// Reconciliation fence written after the normalized tool result and resulting
/// candidate state are both available.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct EffectCompleted {
    pub call_id: String,
    pub result_digest: String,
    pub candidate_digest_after: String,
    pub succeeded: bool,
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
    ApprovalDecided(ApprovalRecord),
    EffectPrepared(EffectPrepared),
    EffectCompleted(EffectCompleted),
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
    fn compute_hash(
        schema_version: u32,
        run_id: RunId,
        sequence: u64,
        timestamp: OffsetDateTime,
        previous_hash: &EventHash,
        event: &RunEvent,
    ) -> Result<EventHash, serde_json::Error> {
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

        let bytes = serde_json::to_vec(&Hashable {
            schema_version,
            run_id,
            sequence,
            timestamp,
            previous_hash,
            event,
        })?;
        Ok(EventHash(blake3::hash(&bytes).to_hex().to_string()))
    }

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
        let hash = Self::compute_hash(
            EVENT_SCHEMA_VERSION,
            run_id,
            sequence,
            timestamp,
            &previous_hash,
            &event,
        )?;
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
        Self::compute_hash(
            self.schema_version,
            self.run_id,
            self.sequence,
            self.timestamp,
            &self.previous_hash,
            &self.event,
        )
        .map(|expected| expected == self.hash)
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
    pub approvals: Vec<ApprovalRecord>,
    pub pending_effects: BTreeMap<String, EffectPrepared>,
    pub completed_effects: Vec<EffectCompleted>,
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
            approvals: Vec::new(),
            pending_effects: BTreeMap::new(),
            completed_effects: Vec::new(),
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
        if !(MIN_EVENT_SCHEMA_VERSION..=EVENT_SCHEMA_VERSION).contains(&envelope.schema_version) {
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
            RunEvent::ApprovalDecided(approval) => self.approvals.push(approval.clone()),
            RunEvent::EffectPrepared(effect) => {
                if self.pending_effects.contains_key(&effect.call_id)
                    || self
                        .completed_effects
                        .iter()
                        .any(|completed| completed.call_id == effect.call_id)
                {
                    return Err(StateError::DuplicateEffect(effect.call_id.clone()));
                }
                self.pending_effects
                    .insert(effect.call_id.clone(), effect.clone());
            }
            RunEvent::EffectCompleted(effect) => {
                if self.pending_effects.remove(&effect.call_id).is_none() {
                    return Err(StateError::UnpreparedEffect(effect.call_id.clone()));
                }
                self.completed_effects.push(effect.clone());
            }
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
    #[error("effect call id {0:?} was prepared more than once")]
    DuplicateEffect(String),
    #[error("effect call id {0:?} completed without a matching preparation")]
    UnpreparedEffect(String),
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
    use proptest::prelude::*;

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

    proptest! {
        #[test]
        fn arbitrary_note_chains_replay_identically_after_serialization(
            notes in prop::collection::vec(prop::collection::vec(any::<char>(), 0..64), 0..64)
        ) {
            let run_id = RunId::new();
            let mut previous = EventHash::genesis();
            let mut envelopes = Vec::with_capacity(notes.len());
            for (sequence, note) in notes.into_iter().enumerate() {
                let envelope = event(
                    run_id,
                    u64::try_from(sequence).unwrap_or_default(),
                    previous,
                    RunEvent::NoteRecorded { message: note.into_iter().collect() },
                );
                previous = envelope.hash.clone();
                envelopes.push(envelope);
            }

            let encoded = serde_json::to_vec(&envelopes)
                .unwrap_or_else(|error| unreachable!("event chain serializes: {error}"));
            let decoded: Vec<EventEnvelope> = serde_json::from_slice(&encoded)
                .unwrap_or_else(|error| unreachable!("event chain decodes: {error}"));
            let mut original = RunSnapshot::new(run_id);
            let mut replayed = RunSnapshot::new(run_id);
            for (left, right) in envelopes.iter().zip(&decoded) {
                prop_assert!(left.verify().is_ok_and(|valid| valid));
                prop_assert!(right.verify().is_ok_and(|valid| valid));
                prop_assert!(original.apply(left).is_ok());
                prop_assert!(replayed.apply(right).is_ok());
            }
            prop_assert_eq!(original, replayed);
        }

        #[test]
        fn changing_any_hashed_note_is_detected(message in prop::collection::vec(any::<char>(), 0..256)) {
            let run_id = RunId::new();
            let original: String = message.into_iter().collect();
            let mut envelope = event(
                run_id,
                0,
                EventHash::genesis(),
                RunEvent::NoteRecorded { message: original.clone() },
            );
            envelope.event = RunEvent::NoteRecorded {
                message: format!("{original}\0tampered"),
            };
            prop_assert!(envelope.verify().is_ok_and(|valid| !valid));
            prop_assert!(matches!(
                RunSnapshot::new(run_id).apply(&envelope),
                Err(StateError::InvalidHash)
            ));
        }
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
    fn effects_must_be_prepared_once_and_completed_once() {
        let run_id = RunId::new();
        let mut snapshot = RunSnapshot::new(run_id);
        let prepared = event(
            run_id,
            0,
            EventHash::genesis(),
            RunEvent::EffectPrepared(EffectPrepared {
                call_id: "call-1".to_owned(),
                tool: "write_file".to_owned(),
                arguments_digest: "a".repeat(64),
                candidate_digest_before: "b".repeat(64),
                risk: "workspace_mutation".to_owned(),
                runtime_profile_digest: "c".repeat(64),
            }),
        );
        snapshot
            .apply(&prepared)
            .unwrap_or_else(|error| unreachable!("prepare: {error}"));
        assert!(snapshot.pending_effects.contains_key("call-1"));
        let completed = event(
            run_id,
            1,
            prepared.hash,
            RunEvent::EffectCompleted(EffectCompleted {
                call_id: "call-1".to_owned(),
                result_digest: "d".repeat(64),
                candidate_digest_after: "e".repeat(64),
                succeeded: true,
            }),
        );
        snapshot
            .apply(&completed)
            .unwrap_or_else(|error| unreachable!("complete: {error}"));
        assert!(snapshot.pending_effects.is_empty());
        assert_eq!(snapshot.completed_effects.len(), 1);

        let duplicate = event(
            run_id,
            2,
            completed.hash,
            RunEvent::EffectCompleted(EffectCompleted {
                call_id: "call-1".to_owned(),
                result_digest: "f".repeat(64),
                candidate_digest_after: "0".repeat(64),
                succeeded: true,
            }),
        );
        assert!(matches!(
            snapshot.apply(&duplicate),
            Err(StateError::UnpreparedEffect(call_id)) if call_id == "call-1"
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

    #[test]
    fn schema_one_envelopes_remain_hash_verifiable_and_projectable() {
        let run_id = RunId::new();
        let timestamp = OffsetDateTime::UNIX_EPOCH;
        let previous_hash = EventHash::genesis();
        let event = RunEvent::NoteRecorded {
            message: "written by v0.4".to_owned(),
        };
        let hash = EventEnvelope::compute_hash(1, run_id, 0, timestamp, &previous_hash, &event)
            .unwrap_or_else(|error| unreachable!("legacy event hashes: {error}"));
        let envelope = EventEnvelope {
            schema_version: 1,
            run_id,
            sequence: 0,
            timestamp,
            previous_hash,
            event,
            hash,
        };

        assert!(envelope.verify().unwrap_or(false));
        let mut snapshot = RunSnapshot::new(run_id);
        snapshot
            .apply(&envelope)
            .unwrap_or_else(|error| unreachable!("legacy event projects: {error}"));
        assert_eq!(snapshot.last_sequence, Some(0));
    }

    #[test]
    fn future_event_schema_is_rejected_before_projection() {
        let run_id = RunId::new();
        let mut envelope = event(
            run_id,
            0,
            EventHash::genesis(),
            RunEvent::NoteRecorded {
                message: "future".to_owned(),
            },
        );
        envelope.schema_version = EVENT_SCHEMA_VERSION + 1;

        let mut snapshot = RunSnapshot::new(run_id);
        assert!(matches!(
            snapshot.apply(&envelope),
            Err(StateError::UnsupportedSchema(version)) if version == EVENT_SCHEMA_VERSION + 1
        ));
    }
}
