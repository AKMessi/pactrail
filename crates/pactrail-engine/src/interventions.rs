//! Explicit, reproducible research manipulations of an otherwise valid channel.

use pactrail_core::agent::CommunicationIntervention;
use pactrail_models::latent::{LatentDescriptor, LatentState};
use pactrail_store::ArtifactStore;

use crate::agents::{AgentMessage, AgentRuntimeError, LatentInterventionReport};

pub(crate) fn intervene(
    state: &LatentState,
    intervention: &CommunicationIntervention,
    store: &ArtifactStore,
    max_bytes: u64,
) -> Result<AgentMessage, AgentRuntimeError> {
    let original = state.descriptor();
    let original_artifact = store.put(state.bytes())?;
    if *intervention == CommunicationIntervention::Correct {
        return Ok(AgentMessage::Latent {
            descriptor: original.clone(),
            stored_bytes: original_artifact.compressed_bytes,
            intervention: None,
        });
    }
    let report = LatentInterventionReport {
        intervention: intervention.clone(),
        original: original.clone(),
        original_stored_bytes: original_artifact.compressed_bytes,
    };
    if *intervention == CommunicationIntervention::None {
        return Ok(AgentMessage::Suppressed { report });
    }
    let mut bytes = state.bytes().to_vec();
    match intervention {
        CommunicationIntervention::Zero => bytes.fill(0),
        CommunicationIntervention::Random { seed } => {
            let mut random = Seeded::new(*seed);
            for chunk in bytes.chunks_exact_mut(4) {
                let bits = 0x3f80_0000 | (random.next() & 0x007f_ffff);
                chunk.copy_from_slice(&(f32::from_bits(bits) - 1.5).to_le_bytes());
            }
        }
        CommunicationIntervention::Shuffled { seed } => {
            let mut random = Seeded::new(*seed);
            for index in (1..bytes.len() / 4).rev() {
                let range = u32::try_from(index + 1).map_err(|_| {
                    AgentRuntimeError::Invalid("intervention size overflow".to_owned())
                })?;
                let target = usize::try_from(random.next() % range).map_err(|_| {
                    AgentRuntimeError::Invalid("intervention index overflow".to_owned())
                })?;
                for offset in 0..4 {
                    bytes.swap(index * 4 + offset, target * 4 + offset);
                }
            }
        }
        CommunicationIntervention::WrongTask { descriptor_digest } => {
            let metadata = store.get_bounded(descriptor_digest, 16_384)?;
            let donor: LatentDescriptor = serde_json::from_slice(&metadata).map_err(|e| {
                AgentRuntimeError::Invalid(format!("invalid wrong-task donor: {e}"))
            })?;
            donor
                .validate(donor.run_id, &original.model, max_bytes)
                .map_err(|e| AgentRuntimeError::Invalid(e.to_string()))?;
            if donor.run_id == original.run_id || donor.slots != original.slots {
                return Err(AgentRuntimeError::Invalid(
                    "wrong-task donor must belong to another run and have the same slot count"
                        .to_owned(),
                ));
            }
            let donor_bytes = store.get_bounded(&donor.payload_digest, donor.logical_bytes)?;
            let donor_run = donor.run_id;
            let donor_state = LatentState::from_bytes(
                donor,
                donor_bytes.into(),
                donor_run,
                &original.model,
                max_bytes,
            )
            .map_err(|e| AgentRuntimeError::Invalid(e.to_string()))?;
            bytes.copy_from_slice(donor_state.bytes());
        }
        CommunicationIntervention::Correct | CommunicationIntervention::None => {
            return Err(AgentRuntimeError::Invalid(
                "invalid intervention dispatch".to_owned(),
            ));
        }
    }
    let mut descriptor = original.clone();
    descriptor.payload_digest = blake3::hash(&bytes).to_hex().to_string();
    let altered = LatentState::from_bytes(
        descriptor,
        bytes.into(),
        original.run_id,
        &original.model,
        max_bytes,
    )
    .map_err(|e| AgentRuntimeError::Invalid(e.to_string()))?;
    let artifact = store.put(altered.bytes())?;
    Ok(AgentMessage::Latent {
        descriptor: altered.descriptor().clone(),
        stored_bytes: artifact.compressed_bytes,
        intervention: Some(report),
    })
}

pub(crate) fn validate_donor(
    store: &ArtifactStore,
    digest: &str,
    run_id: pactrail_core::RunId,
    model: Option<&pactrail_models::latent::LatentCapabilities>,
    max_slots: u32,
    max_bytes: u64,
) -> Result<(), AgentRuntimeError> {
    let bytes = store.get_bounded(digest, 16_384)?;
    let descriptor: LatentDescriptor = serde_json::from_slice(&bytes)
        .map_err(|e| AgentRuntimeError::Invalid(format!("invalid donor descriptor: {e}")))?;
    if descriptor.run_id == run_id || descriptor.slots > max_slots {
        return Err(AgentRuntimeError::Invalid(
            "donor must belong to another run and fit the slot budget".to_owned(),
        ));
    }
    let expected_model = model.unwrap_or(&descriptor.model);
    descriptor
        .validate(descriptor.run_id, expected_model, max_bytes)
        .map_err(|e| AgentRuntimeError::Invalid(e.to_string()))?;
    let payload = store.get_bounded(&descriptor.payload_digest, descriptor.logical_bytes)?;
    LatentState::from_bytes(
        descriptor.clone(),
        payload.into(),
        descriptor.run_id,
        expected_model,
        max_bytes,
    )
    .map_err(|e| AgentRuntimeError::Invalid(e.to_string()))?;
    Ok(())
}

struct Seeded(u64);
impl Seeded {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9e37_79b9_7f4a_7c15)
    }
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        u32::try_from((value ^ (value >> 31)) & u64::from(u32::MAX)).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pactrail_core::{RunId, agent::AgentId};
    use pactrail_models::latent::{LatentCapabilities, LatentCodec, LatentDType};

    fn state(run_id: RunId, values: &[f32]) -> LatentState {
        let bytes: Vec<_> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let model = LatentCapabilities {
            schema_version: 1,
            checkpoint_digest: "a".repeat(64),
            architecture: "test".to_owned(),
            tokenizer_digest: "b".repeat(64),
            representation: "test-v1".to_owned(),
            layer: 1,
            hidden_width: u32::try_from(values.len()).unwrap_or(0),
            max_slots: 8,
        };
        let descriptor = LatentDescriptor {
            schema_version: 1,
            run_id,
            source_agent: AgentId::try_from("sender".to_owned())
                .unwrap_or_else(|e| unreachable!("id: {e}")),
            model: model.clone(),
            dtype: LatentDType::F32Le,
            codec: LatentCodec::SelectedSlotsV1,
            slots: 1,
            payload_digest: blake3::hash(&bytes).to_hex().to_string(),
            logical_bytes: u64::try_from(bytes.len()).unwrap_or(0),
        };
        LatentState::from_bytes(descriptor, bytes.into(), run_id, &model, 1024)
            .unwrap_or_else(|e| unreachable!("state: {e}"))
    }

    #[test]
    fn interventions_are_deterministic_finite_and_retain_the_original() {
        let root = tempfile::tempdir().unwrap_or_else(|e| unreachable!("temp: {e}"));
        let store = ArtifactStore::open(root.path()).unwrap_or_else(|e| unreachable!("store: {e}"));
        let original = state(RunId::new(), &[1.0, 2.0, 3.0, 4.0]);
        for intervention in [
            CommunicationIntervention::Correct,
            CommunicationIntervention::None,
            CommunicationIntervention::Zero,
            CommunicationIntervention::Random { seed: 42 },
            CommunicationIntervention::Shuffled { seed: 42 },
        ] {
            let first = intervene(&original, &intervention, &store, 1024)
                .unwrap_or_else(|e| unreachable!("intervene: {e}"));
            let second = intervene(&original, &intervention, &store, 1024)
                .unwrap_or_else(|e| unreachable!("intervene: {e}"));
            assert_eq!(first, second);
            assert_eq!(
                store
                    .get_bounded(&original.descriptor().payload_digest, 1024)
                    .unwrap_or_default(),
                original.bytes()
            );
            if let AgentMessage::Latent { descriptor, .. } = first {
                let bytes = store
                    .get_bounded(&descriptor.payload_digest, 1024)
                    .unwrap_or_default();
                assert_eq!(bytes.len(), original.bytes().len());
                assert!(
                    bytes
                        .chunks_exact(4)
                        .all(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]).is_finite())
                );
                if intervention == CommunicationIntervention::Zero {
                    assert!(bytes.iter().all(|b| *b == 0));
                }
            } else {
                assert_eq!(intervention, CommunicationIntervention::None);
            }
        }
    }

    #[test]
    fn wrong_task_requires_explicit_compatible_foreign_state_and_rebinds_provenance() {
        let root = tempfile::tempdir().unwrap_or_else(|e| unreachable!("temp: {e}"));
        let store = ArtifactStore::open(root.path()).unwrap_or_else(|e| unreachable!("store: {e}"));
        let original = state(RunId::new(), &[1.0, 2.0]);
        for donor in [
            original.clone(),
            state(RunId::new(), &[8.0, 9.0]),
            state(RunId::new(), &[1.0, 2.0, 3.0]),
        ] {
            store
                .put(donor.bytes())
                .unwrap_or_else(|e| unreachable!("store: {e}"));
            let metadata = serde_json::to_vec(donor.descriptor()).unwrap_or_default();
            let artifact = store
                .put(&metadata)
                .unwrap_or_else(|e| unreachable!("metadata: {e}"));
            let intervention = CommunicationIntervention::WrongTask {
                descriptor_digest: artifact.digest,
            };
            let result = intervene(&original, &intervention, &store, 1024);
            if donor.descriptor().run_id == original.descriptor().run_id
                || donor.descriptor().model != original.descriptor().model
            {
                assert!(result.is_err());
            } else {
                let AgentMessage::Latent {
                    descriptor,
                    intervention,
                    ..
                } = result.unwrap_or_else(|e| unreachable!("valid donor: {e}"))
                else {
                    unreachable!("latent")
                };
                assert_eq!(descriptor.run_id, original.descriptor().run_id);
                assert_eq!(descriptor.source_agent, original.descriptor().source_agent);
                assert_eq!(descriptor.payload_digest, donor.descriptor().payload_digest);
                assert!(intervention.is_some());
            }
        }
    }
}
