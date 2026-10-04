//! Experimental hidden-state extension. No tensor enters the textual model IR.

use std::sync::Arc;

use async_trait::async_trait;
use pactrail_core::{RunId, agent::AgentId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{ModelRequest, ModelResponse};

/// Current experimental internal-state descriptor schema.
pub const LATENT_DESCRIPTOR_SCHEMA_VERSION: u32 = 1;

/// Exact shared representation, including receiver fusion semantics.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LatentCapabilities {
    pub schema_version: u32,
    pub checkpoint_digest: String,
    pub architecture: String,
    pub tokenizer_digest: String,
    pub representation: String,
    pub layer: u32,
    pub hidden_width: u32,
    pub max_slots: u32,
}

impl LatentCapabilities {
    /// Validates identity and allocation ceilings before backend use.
    ///
    /// # Errors
    /// Rejects unknown versions, unbound identities and oversized dimensions.
    pub fn validate(&self) -> Result<(), LatentError> {
        if self.schema_version != 1
            || !digest_valid(&self.checkpoint_digest)
            || !digest_valid(&self.tokenizer_digest)
            || self.layer > 256
            || !(1..=16_384).contains(&self.hidden_width)
            || !(1..=64).contains(&self.max_slots)
        {
            return Err(LatentError::Invalid(
                "unsupported schema or invalid model identity/dimensions",
            ));
        }
        for text in [&self.architecture, &self.representation] {
            if text.is_empty()
                || text.len() > 128
                || !text
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'))
            {
                return Err(LatentError::Invalid(
                    "invalid architecture/representation identifier",
                ));
            }
        }
        Ok(())
    }
}

/// Only explicitly tested numerical representations are admitted.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LatentDType {
    F32Le,
}

/// Dense selected hidden-state slots; selection occurs in the model backend.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LatentCodec {
    SelectedSlotsV1,
}

/// Integrity and provenance envelope; payload is stored separately by digest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LatentDescriptor {
    pub schema_version: u32,
    pub run_id: RunId,
    pub source_agent: AgentId,
    pub model: LatentCapabilities,
    pub dtype: LatentDType,
    pub codec: LatentCodec,
    pub slots: u32,
    pub payload_digest: String,
    pub logical_bytes: u64,
}

impl LatentDescriptor {
    /// Checks identity, shape and exact byte product without allocation.
    ///
    /// # Errors
    /// Rejects malformed, incompatible and cross-run descriptors.
    pub fn validate(
        &self,
        run_id: RunId,
        model: &LatentCapabilities,
        max_bytes: u64,
    ) -> Result<(), LatentError> {
        self.model.validate()?;
        model.validate()?;
        if self.schema_version != LATENT_DESCRIPTOR_SCHEMA_VERSION || self.run_id != run_id {
            return Err(LatentError::Invalid(
                "latent state schema or parent run mismatch",
            ));
        }
        if &self.model != model {
            return Err(LatentError::IncompatibleModel);
        }
        let bytes = u64::from(self.slots)
            .checked_mul(u64::from(model.hidden_width))
            .and_then(|v| v.checked_mul(4))
            .ok_or(LatentError::Invalid("latent dimensions overflow"))?;
        if self.slots == 0
            || self.slots > model.max_slots
            || bytes != self.logical_bytes
            || bytes > max_bytes
            || !digest_valid(&self.payload_digest)
        {
            return Err(LatentError::Invalid(
                "latent shape, digest or payload size exceeds the admitted bounds",
            ));
        }
        Ok(())
    }
}

/// Validated immutable slots shared without copying between transport consumers.
#[derive(Clone, Debug)]
pub struct LatentState {
    descriptor: LatentDescriptor,
    bytes: Arc<[u8]>,
}

impl LatentState {
    /// Seals bounded finite f32 slots against their descriptor.
    ///
    /// # Errors
    /// Rejects any byte, digest, dimension, model or parent-run mismatch.
    pub fn from_bytes(
        descriptor: LatentDescriptor,
        bytes: Arc<[u8]>,
        run_id: RunId,
        model: &LatentCapabilities,
        max_bytes: u64,
    ) -> Result<Self, LatentError> {
        descriptor.validate(run_id, model, max_bytes)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != descriptor.logical_bytes
            || blake3::hash(&bytes).to_hex().as_str() != descriptor.payload_digest
        {
            return Err(LatentError::Integrity);
        }
        for chunk in bytes.chunks_exact(4) {
            let value = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            if !value.is_finite() {
                return Err(LatentError::Invalid("non-finite latent value"));
            }
        }
        Ok(Self { descriptor, bytes })
    }

    #[must_use]
    pub const fn descriptor(&self) -> &LatentDescriptor {
        &self.descriptor
    }
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// A complete model turn plus optional internal-state export. Human/tool output
/// remains subject to the ordinary engine response and effect boundaries.
pub struct LatentTurn {
    pub response: ModelResponse,
    pub exported_slots: Option<LatentState>,
    /// Backend-measured inference time; absent means not measured.
    pub inference_ms: Option<u64>,
}

/// Trusted run-local decode boundary supplied by the coordinator, not peer text.
/// This metadata grants no tool capability.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LatentTurnContext {
    pub run_id: RunId,
    pub agent: AgentId,
    pub round: u16,
    pub max_rounds: u16,
    pub allow_human_output: bool,
}

/// Optional extension implemented only by genuinely state-capable backends.
///
/// Implementations must fuse imported slots into inference according to the
/// exact representation identity, never decode/embed a peer prose message, and
/// release in-flight resources when the invocation future is dropped. No hidden
/// downloads or fallback transport are permitted by this contract.
#[async_trait]
pub trait LatentModelDriver: Send + Sync {
    fn latent_capabilities(&self) -> &LatentCapabilities;
    async fn invoke_latent(
        &self,
        request: &ModelRequest,
        context: &LatentTurnContext,
        imported: &[LatentState],
        export_slots: u32,
    ) -> Result<LatentTurn, LatentError>;
}

/// Explicit latent boundary failure; no text fallback is implied.
#[derive(Debug, Error)]
pub enum LatentError {
    #[error(
        "latent communication requested but backend does not support latent-state export/import"
    )]
    Unsupported,
    #[error("latent state was produced by an incompatible model/representation identity")]
    IncompatibleModel,
    #[error("latent artifact digest or payload length mismatch")]
    Integrity,
    #[error("invalid latent state: {0}")]
    Invalid(&'static str),
    #[error("latent model invocation failed: {0}")]
    Backend(String),
}

fn digest_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn latent_descriptor_compatibility_fixture_uses_the_strict_production_validator() {
        let descriptor: LatentDescriptor = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/compatibility/latent-descriptor-v1.json"
        )))
        .unwrap_or_else(|e| unreachable!("fixture: {e}"));
        assert!(
            descriptor
                .validate(descriptor.run_id, &descriptor.model, 1024)
                .is_ok()
        );
        let mut future = descriptor;
        future.schema_version = 2;
        assert!(future.validate(future.run_id, &future.model, 1024).is_err());
    }

    fn model() -> LatentCapabilities {
        LatentCapabilities {
            schema_version: 1,
            checkpoint_digest: "a".repeat(64),
            architecture: "test".to_owned(),
            tokenizer_digest: "b".repeat(64),
            representation: "last-layer-prepend-v1".to_owned(),
            layer: 1,
            hidden_width: 2,
            max_slots: 8,
        }
    }

    fn fixture(bytes: &[u8], run: RunId) -> LatentDescriptor {
        LatentDescriptor {
            schema_version: 1,
            run_id: run,
            source_agent: AgentId::try_from("solver".to_owned())
                .unwrap_or_else(|e| unreachable!("id: {e}")),
            model: model(),
            dtype: LatentDType::F32Le,
            codec: LatentCodec::SelectedSlotsV1,
            slots: 1,
            payload_digest: blake3::hash(bytes).to_hex().to_string(),
            logical_bytes: 8,
        }
    }

    #[test]
    fn rejects_corruption_cross_run_wrong_model_and_non_finite_values() {
        let run = RunId::new();
        let bytes = [1.0_f32.to_le_bytes(), 2.0_f32.to_le_bytes()].concat();
        let descriptor = fixture(&bytes, run);
        assert!(
            LatentState::from_bytes(descriptor.clone(), bytes.clone().into(), run, &model(), 8)
                .is_ok()
        );
        assert!(
            LatentState::from_bytes(
                descriptor.clone(),
                bytes.clone().into(),
                RunId::new(),
                &model(),
                8
            )
            .is_err()
        );
        let mut wrong = model();
        wrong.checkpoint_digest = "c".repeat(64);
        assert!(matches!(
            descriptor.validate(run, &wrong, 8),
            Err(LatentError::IncompatibleModel)
        ));
        let mut corrupted = bytes.clone();
        corrupted[0] ^= 1;
        assert!(matches!(
            LatentState::from_bytes(descriptor, corrupted.into(), run, &model(), 8),
            Err(LatentError::Integrity)
        ));
        let nan = [f32::NAN.to_le_bytes(), 0.0_f32.to_le_bytes()].concat();
        assert!(LatentState::from_bytes(fixture(&nan, run), nan.into(), run, &model(), 8).is_err());
    }

    proptest! {
        #[test]
        fn untrusted_dimensions_never_admit_unbounded_payloads(slots in any::<u32>(), width in any::<u32>(), length in any::<u64>()) {
            let run = RunId::new(); let mut d = fixture(&[0; 8], run);
            d.slots = slots; d.model.hidden_width = width; d.logical_bytes = length;
            if d.validate(run, &d.model, 1024).is_ok() {
                prop_assert!(length <= 1024);
                prop_assert!(slots > 0 && slots <= 8);
                prop_assert!(width > 0 && width <= 16_384);
                prop_assert_eq!(length, u64::from(slots) * u64::from(width) * 4);
            }
        }
    }
}
