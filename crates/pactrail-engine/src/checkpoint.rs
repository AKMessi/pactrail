use std::collections::BTreeSet;
use std::path::Path;

use pactrail_core::{EventHash, RunEvent, RunId, TaskContract};
use pactrail_models::{ConversationItem, Usage};
use pactrail_store::{ArtifactError, ArtifactStore, EventStore, StoreError, StoredArtifact};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const CHECKPOINT_SCHEMA_VERSION: u32 = 1;
const CHECKPOINT_EVENT_PREFIX: &str = "session:";
const MAX_CONVERSATION_ITEMS: usize = 16_384;
const MAX_CALL_IDS: usize = 65_536;
const MAX_CONTROL_STRING_BYTES: usize = 1_048_576;

/// Safe engine location represented by a durable session checkpoint.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumePhase {
    BeforeModel,
    BeforeTools,
    BeforeVerification,
}

/// Provider-neutral execution state required to continue a run without replaying
/// already completed tool effects.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunCheckpoint {
    pub schema_version: u32,
    pub run_id: RunId,
    pub event_sequence: u64,
    pub event_hash: EventHash,
    pub contract_digest: String,
    pub candidate_digest: String,
    pub model_profile_digest: String,
    pub tool_profile_digest: String,
    pub context_digest: String,
    pub project_profile: String,
    pub phase: ResumePhase,
    pub next_turn: u16,
    pub elapsed_active_ms: u64,
    pub conversation: Vec<ConversationItem>,
    pub usage: Usage,
    pub call_ids: BTreeSet<String>,
    pub previous_tool_signature: Option<Vec<(String, String)>>,
    pub repeated_tool_turns: u16,
    pub consecutive_failed_tool_turns: u16,
    pub automatic_repair_cycles: u16,
    pub final_text: String,
    pub recovery_risk: Option<String>,
}

/// Inputs whose values are fixed for the lifetime of a resumable run.
pub struct CheckpointIdentity<'a> {
    pub run_id: RunId,
    pub event_sequence: u64,
    pub event_hash: EventHash,
    pub contract: &'a TaskContract,
    pub candidate_digest: String,
    pub model_profile_digest: String,
    pub tool_profile_digest: String,
    pub context_digest: String,
}

impl RunCheckpoint {
    /// Builds a validated checkpoint with empty loop-controller state.
    ///
    /// # Errors
    ///
    /// Returns an error if a digest or bounded field is invalid.
    pub fn initial(
        identity: CheckpointIdentity<'_>,
        conversation: Vec<ConversationItem>,
    ) -> Result<Self, CheckpointError> {
        let checkpoint = Self {
            schema_version: CHECKPOINT_SCHEMA_VERSION,
            run_id: identity.run_id,
            event_sequence: identity.event_sequence,
            event_hash: identity.event_hash,
            contract_digest: contract_digest(identity.contract)?,
            candidate_digest: identity.candidate_digest,
            model_profile_digest: identity.model_profile_digest,
            tool_profile_digest: identity.tool_profile_digest,
            context_digest: identity.context_digest,
            project_profile: String::new(),
            phase: ResumePhase::BeforeModel,
            next_turn: 0,
            elapsed_active_ms: 0,
            conversation,
            usage: Usage::default(),
            call_ids: BTreeSet::new(),
            previous_tool_signature: None,
            repeated_tool_turns: 0,
            consecutive_failed_tool_turns: 0,
            automatic_repair_cycles: 0,
            final_text: String::new(),
            recovery_risk: None,
        };
        checkpoint.validate()?;
        Ok(checkpoint)
    }

    /// Validates schema, bounds, hashes, and controller invariants.
    ///
    /// # Errors
    ///
    /// Returns a fail-closed checkpoint diagnostic.
    pub fn validate(&self) -> Result<(), CheckpointError> {
        if self.schema_version != CHECKPOINT_SCHEMA_VERSION {
            return Err(CheckpointError::UnsupportedSchema(self.schema_version));
        }
        for (field, digest) in [
            ("event_hash", self.event_hash.0.as_str()),
            ("contract_digest", self.contract_digest.as_str()),
            ("candidate_digest", self.candidate_digest.as_str()),
            ("model_profile_digest", self.model_profile_digest.as_str()),
            ("tool_profile_digest", self.tool_profile_digest.as_str()),
            ("context_digest", self.context_digest.as_str()),
        ] {
            validate_digest(field, digest)?;
        }
        if self.conversation.len() > MAX_CONVERSATION_ITEMS {
            return Err(CheckpointError::BoundExceeded {
                field: "conversation",
                actual: self.conversation.len(),
                limit: MAX_CONVERSATION_ITEMS,
            });
        }
        if self.call_ids.len() > MAX_CALL_IDS {
            return Err(CheckpointError::BoundExceeded {
                field: "call_ids",
                actual: self.call_ids.len(),
                limit: MAX_CALL_IDS,
            });
        }
        for call_id in &self.call_ids {
            validate_control_string("call_id", call_id)?;
        }
        if let Some(risk) = &self.recovery_risk {
            validate_control_string("recovery_risk", risk)?;
        }
        validate_control_string("final_text", &self.final_text)?;
        validate_control_string("project_profile", &self.project_profile)?;
        if self.phase == ResumePhase::BeforeTools {
            let has_pending_calls = self
                .conversation
                .last()
                .is_some_and(|item| matches!(item, ConversationItem::AssistantToolCalls { .. }));
            if !has_pending_calls {
                return Err(CheckpointError::InvalidPhase(
                    "before_tools requires assistant tool calls at the conversation tail",
                ));
            }
        }
        Ok(())
    }
}

/// Content-addressed checkpoint persistence bound to the durable event head.
#[derive(Clone, Debug)]
pub struct CheckpointStore {
    artifacts: ArtifactStore,
}

impl CheckpointStore {
    /// Opens the checkpoint artifact directory.
    ///
    /// # Errors
    ///
    /// Returns an artifact I/O error when the directory cannot be initialized.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, CheckpointError> {
        Ok(Self {
            artifacts: ArtifactStore::open(root.as_ref())?,
        })
    }

    /// Persists canonical checkpoint bytes before the caller appends the naming
    /// event to its run journal.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid state, JSON encoding, or artifact I/O.
    pub fn put(&self, checkpoint: &RunCheckpoint) -> Result<StoredArtifact, CheckpointError> {
        checkpoint.validate()?;
        let bytes = serde_json::to_vec(checkpoint).map_err(CheckpointError::Encoding)?;
        self.artifacts
            .put(&bytes)
            .map_err(CheckpointError::Artifact)
    }

    /// Loads the checkpoint named by the current hash-linked event head.
    ///
    /// # Errors
    ///
    /// Returns an error when the run has no checkpoint, the checkpoint event is
    /// stale, or the artifact fails integrity, schema, identity, or head binding.
    pub fn load_head(
        &self,
        events: &EventStore,
        run_id: RunId,
    ) -> Result<RunCheckpoint, CheckpointError> {
        let envelopes = events.load(run_id)?;
        let head = envelopes.last().ok_or(CheckpointError::NotFound(run_id))?;
        let RunEvent::CheckpointCreated { checkpoint } = &head.event else {
            let snapshot = events.snapshot(run_id)?;
            if let Some(effect) = snapshot.pending_effects.values().next() {
                return Err(CheckpointError::UncertainEffect {
                    run_id,
                    call_id: effect.call_id.clone(),
                    tool: effect.tool.clone(),
                    risk: effect.risk.clone(),
                });
            }
            return Err(CheckpointError::NotAtHead {
                run_id,
                head_sequence: head.sequence,
            });
        };
        let digest = checkpoint
            .strip_prefix(CHECKPOINT_EVENT_PREFIX)
            .ok_or_else(|| CheckpointError::InvalidEventReference(checkpoint.clone()))?;
        validate_digest("checkpoint", digest)?;
        let bytes = self.artifacts.get(digest)?;
        let checkpoint: RunCheckpoint =
            serde_json::from_slice(&bytes).map_err(CheckpointError::Decoding)?;
        checkpoint.validate()?;
        if checkpoint.run_id != run_id {
            return Err(CheckpointError::WrongRun {
                expected: run_id,
                actual: checkpoint.run_id,
            });
        }
        if checkpoint.event_sequence.saturating_add(1) != head.sequence
            || checkpoint.event_hash != head.previous_hash
        {
            return Err(CheckpointError::HeadBinding {
                checkpoint_sequence: checkpoint.event_sequence,
                event_sequence: head.sequence,
            });
        }
        Ok(checkpoint)
    }

    /// Formats the event reference for a previously persisted artifact.
    #[must_use]
    pub fn event_reference(artifact: &StoredArtifact) -> String {
        format!("{CHECKPOINT_EVENT_PREFIX}{}", artifact.digest)
    }
}

/// Computes the canonical task-contract digest used in resume identity checks.
///
/// # Errors
///
/// Returns a JSON encoding error if the contract cannot be serialized.
pub fn contract_digest(contract: &TaskContract) -> Result<String, CheckpointError> {
    let bytes = serde_json::to_vec(contract).map_err(CheckpointError::Encoding)?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn validate_digest(field: &'static str, digest: &str) -> Result<(), CheckpointError> {
    if digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(CheckpointError::InvalidDigest {
            field,
            value: digest.to_owned(),
        })
    }
}

fn validate_control_string(field: &'static str, value: &str) -> Result<(), CheckpointError> {
    if value.len() <= MAX_CONTROL_STRING_BYTES && !value.contains(['\0', '\r']) {
        Ok(())
    } else {
        Err(CheckpointError::InvalidString { field })
    }
}

/// Durable checkpoint persistence or binding failure.
#[derive(Debug, Error)]
pub enum CheckpointError {
    #[error("checkpoint schema {0} is unsupported")]
    UnsupportedSchema(u32),
    #[error("checkpoint field {field} is not a lowercase BLAKE3 digest: {value:?}")]
    InvalidDigest { field: &'static str, value: String },
    #[error("checkpoint field {field} exceeds its safety bound: {actual} > {limit}")]
    BoundExceeded {
        field: &'static str,
        actual: usize,
        limit: usize,
    },
    #[error("checkpoint field {field} contains invalid or oversized text")]
    InvalidString { field: &'static str },
    #[error("checkpoint phase is inconsistent: {0}")]
    InvalidPhase(&'static str),
    #[error("run {0} has no durable session checkpoint")]
    NotFound(RunId),
    #[error("run {run_id} event head {head_sequence} is not a safe checkpoint")]
    NotAtHead { run_id: RunId, head_sequence: u64 },
    #[error(
        "run {run_id} stopped with uncertain {risk} effect {tool}/{call_id}; automatic replay is forbidden"
    )]
    UncertainEffect {
        run_id: RunId,
        call_id: String,
        tool: String,
        risk: String,
    },
    #[error("invalid checkpoint event reference {0:?}")]
    InvalidEventReference(String),
    #[error("checkpoint belongs to run {actual}, expected {expected}")]
    WrongRun { expected: RunId, actual: RunId },
    #[error(
        "checkpoint/event head binding failed: checkpoint head {checkpoint_sequence}, naming event {event_sequence}"
    )]
    HeadBinding {
        checkpoint_sequence: u64,
        event_sequence: u64,
    },
    #[error("checkpoint JSON encoding failed: {0}")]
    Encoding(serde_json::Error),
    #[error("checkpoint JSON decoding failed: {0}")]
    Decoding(serde_json::Error),
    #[error("checkpoint artifact failed: {0}")]
    Artifact(#[from] ArtifactError),
    #[error("checkpoint event load failed: {0}")]
    Store(#[from] StoreError),
}

#[cfg(test)]
mod tests {
    use pactrail_core::{EffectPrepared, RunEvent, RunState};
    use pactrail_models::Message;

    use super::*;

    fn checkpoint(run_id: RunId, sequence: u64, hash: EventHash) -> RunCheckpoint {
        RunCheckpoint::initial(
            CheckpointIdentity {
                run_id,
                event_sequence: sequence,
                event_hash: hash,
                contract: &TaskContract::new("fix it", "."),
                candidate_digest: "1".repeat(64),
                model_profile_digest: "2".repeat(64),
                tool_profile_digest: "3".repeat(64),
                context_digest: "4".repeat(64),
            },
            vec![ConversationItem::Message(Message::user("fix it"))],
        )
        .unwrap_or_else(|error| unreachable!("checkpoint: {error}"))
    }

    #[test]
    fn only_a_head_bound_artifact_is_resumable() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        let checkpoints = CheckpointStore::open(root.path())
            .unwrap_or_else(|error| unreachable!("checkpoint store: {error}"));
        let mut events = EventStore::open_in_memory()
            .unwrap_or_else(|error| unreachable!("event store: {error}"));
        let run_id = RunId::new();
        let contract = events
            .append(
                run_id,
                0,
                RunEvent::ContractRegistered(TaskContract::new("fix it", ".")),
            )
            .unwrap_or_else(|error| unreachable!("contract: {error}"));
        let checkpoint = checkpoint(run_id, contract.sequence, contract.hash);
        let artifact = checkpoints
            .put(&checkpoint)
            .unwrap_or_else(|error| unreachable!("artifact: {error}"));

        assert!(matches!(
            checkpoints.load_head(&events, run_id),
            Err(CheckpointError::NotAtHead { .. })
        ));
        events
            .append(
                run_id,
                1,
                RunEvent::CheckpointCreated {
                    checkpoint: CheckpointStore::event_reference(&artifact),
                },
            )
            .unwrap_or_else(|error| unreachable!("checkpoint event: {error}"));
        assert_eq!(
            checkpoints.load_head(&events, run_id).ok(),
            Some(checkpoint)
        );

        events
            .append(
                run_id,
                2,
                RunEvent::StateChanged {
                    from: RunState::Created,
                    to: RunState::Contracting,
                },
            )
            .unwrap_or_else(|error| unreachable!("later event: {error}"));
        assert!(matches!(
            checkpoints.load_head(&events, run_id),
            Err(CheckpointError::NotAtHead { .. })
        ));
    }

    #[test]
    fn wrong_run_and_future_schema_fail_closed() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        let checkpoints = CheckpointStore::open(root.path())
            .unwrap_or_else(|error| unreachable!("checkpoint store: {error}"));
        let run_id = RunId::new();
        let mut value = checkpoint(run_id, 0, EventHash("0".repeat(64)));
        value.schema_version = CHECKPOINT_SCHEMA_VERSION + 1;
        assert!(matches!(
            checkpoints.put(&value),
            Err(CheckpointError::UnsupportedSchema(_))
        ));
        value.schema_version = CHECKPOINT_SCHEMA_VERSION;
        value.run_id = RunId::new();
        assert_ne!(value.run_id, run_id);
    }

    #[test]
    fn uncertain_effect_is_reported_instead_of_replayed() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        let checkpoints = CheckpointStore::open(root.path())
            .unwrap_or_else(|error| unreachable!("checkpoint store: {error}"));
        let mut events = EventStore::open_in_memory()
            .unwrap_or_else(|error| unreachable!("event store: {error}"));
        let run_id = RunId::new();
        let contract = events
            .append(
                run_id,
                0,
                RunEvent::ContractRegistered(TaskContract::new("fix it", ".")),
            )
            .unwrap_or_else(|error| unreachable!("contract: {error}"));
        let checkpoint = checkpoint(run_id, contract.sequence, contract.hash);
        let artifact = checkpoints
            .put(&checkpoint)
            .unwrap_or_else(|error| unreachable!("artifact: {error}"));
        events
            .append(
                run_id,
                1,
                RunEvent::CheckpointCreated {
                    checkpoint: CheckpointStore::event_reference(&artifact),
                },
            )
            .unwrap_or_else(|error| unreachable!("checkpoint event: {error}"));
        events
            .append(
                run_id,
                2,
                RunEvent::EffectPrepared(EffectPrepared {
                    call_id: "call-1".to_owned(),
                    tool: "write_file".to_owned(),
                    arguments_digest: "a".repeat(64),
                    candidate_digest_before: "b".repeat(64),
                    risk: "workspace_mutation".to_owned(),
                    runtime_profile_digest: "c".repeat(64),
                }),
            )
            .unwrap_or_else(|error| unreachable!("effect: {error}"));

        assert!(matches!(
            checkpoints.load_head(&events, run_id),
            Err(CheckpointError::UncertainEffect { call_id, tool, .. })
                if call_id == "call-1" && tool == "write_file"
        ));
    }
}
