//! Experimental run-local agent contracts. Roles are labels, never grants.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::Capability;

/// Current experimental agent configuration schema.
pub const AGENT_CONFIG_SCHEMA_VERSION: u32 = 1;

/// Stable, bounded identity within a parent run.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct AgentId(String);

impl AgentId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for AgentId {
    type Error = AgentContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(AgentContractError::Invalid(
                "agent id must be 1-64 lowercase ASCII letters, digits or hyphens",
            ));
        }
        Ok(Self(value))
    }
}

impl From<AgentId> for String {
    fn from(value: AgentId) -> Self {
        value.0
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Reasoning specialization. Capability sets are independently enforced.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Localizer,
    Solver,
    Critic,
    Implementer,
    Verifier,
}

/// Explicit endpoint selection without provider-specific configuration.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentModelRoute {
    #[default]
    Primary,
    Investigation,
}

/// Internal communication transport; no implicit fallback is permitted.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunicationMode {
    #[default]
    Single,
    Text,
    Latent,
}

/// Explicit research intervention; never selected by an agent or by default.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommunicationIntervention {
    #[default]
    Correct,
    None,
    Zero,
    Random {
        seed: u64,
    },
    Shuffled {
        seed: u64,
    },
    /// Digest of an explicitly prepared donor descriptor, not a tensor array.
    WrongTask {
        descriptor_digest: String,
    },
}

/// Additional authority ceiling, intersected with the run contract.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    pub id: AgentId,
    pub role: AgentRole,
    #[serde(default)]
    pub model_route: AgentModelRoute,
    pub capabilities: BTreeSet<Capability>,
    pub max_turns: u16,
}

/// Hard limits; every field is finite and positive, including byte budgets.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommunicationBudget {
    pub max_agents: u16,
    pub max_model_attempts: u16,
    pub max_rounds: u16,
    pub max_messages: u64,
    pub max_text_bytes: u64,
    pub max_latent_logical_bytes: u64,
    pub max_latent_stored_bytes: u64,
    pub max_bytes_per_message: u64,
    pub max_latent_slots: u64,
}

impl Default for CommunicationBudget {
    fn default() -> Self {
        Self {
            max_agents: 5,
            max_model_attempts: 24,
            max_rounds: 1,
            max_messages: 32,
            max_text_bytes: 65_536,
            max_latent_logical_bytes: 1_048_576,
            max_latent_stored_bytes: 1_048_576,
            max_bytes_per_message: 262_144,
            max_latent_slots: 128,
        }
    }
}

/// Schema-one opt-in fixed-order topology. No agent may add participants.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRunConfig {
    pub schema_version: u32,
    pub communication: CommunicationMode,
    pub agents: Vec<AgentSpec>,
    pub budget: CommunicationBudget,
    pub latent_slots_per_message: u32,
    #[serde(default)]
    pub intervention: CommunicationIntervention,
}

impl AgentRunConfig {
    /// Builds the bounded localizer → solver → critic → implementer profile.
    ///
    /// # Errors
    /// Single mode must use the ordinary engine without a profile.
    pub fn coding(communication: CommunicationMode) -> Result<Self, AgentContractError> {
        let agents = [
            ("localizer", AgentRole::Localizer, 4),
            ("solver", AgentRole::Solver, 4),
            ("critic", AgentRole::Critic, 4),
            ("implementer", AgentRole::Implementer, 12),
        ]
        .into_iter()
        .map(|(id, role, max_turns)| {
            let mut capabilities = BTreeSet::from([Capability::FileRead, Capability::MemoryRead]);
            if role == AgentRole::Implementer {
                capabilities.extend([Capability::FileWrite, Capability::ProcessSpawn]);
            }
            Ok(AgentSpec {
                id: AgentId::try_from(id.to_owned())?,
                role,
                model_route: AgentModelRoute::Primary,
                capabilities,
                max_turns,
            })
        })
        .collect::<Result<Vec<_>, AgentContractError>>()?;
        let config = Self {
            schema_version: AGENT_CONFIG_SCHEMA_VERSION,
            communication,
            agents,
            budget: CommunicationBudget::default(),
            latent_slots_per_message: 8,
            intervention: CommunicationIntervention::Correct,
        };
        config.validate()?;
        Ok(config)
    }

    /// Validates finite resource bounds and explicit topology/authority.
    ///
    /// # Errors
    /// Rejects unsafe, unsupported or ambiguous configuration.
    pub fn validate(&self) -> Result<(), AgentContractError> {
        if self.schema_version != AGENT_CONFIG_SCHEMA_VERSION {
            return Err(AgentContractError::UnsupportedSchema(self.schema_version));
        }
        if self.communication != CommunicationMode::Latent
            && self.intervention != CommunicationIntervention::Correct
        {
            return Err(AgentContractError::Invalid(
                "latent interventions require latent communication",
            ));
        }
        if let CommunicationIntervention::WrongTask { descriptor_digest } = &self.intervention
            && (descriptor_digest.len() != 64
                || !descriptor_digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        {
            return Err(AgentContractError::Invalid(
                "wrong-task intervention needs a valid donor descriptor digest",
            ));
        }
        if self.communication == CommunicationMode::Single {
            return Err(AgentContractError::Invalid(
                "single mode uses the ordinary engine without agent configuration",
            ));
        }
        if !(1..=64).contains(&self.latent_slots_per_message) {
            return Err(AgentContractError::Invalid(
                "latent slots per message must be 1-64",
            ));
        }
        let b = &self.budget;
        if !(2..=8).contains(&b.max_agents)
            || self.agents.len() < 2
            || self.agents.len() > usize::from(b.max_agents)
        {
            return Err(AgentContractError::Invalid(
                "agent count must be 2-8 and within max_agents",
            ));
        }
        if !(1..=200).contains(&b.max_model_attempts)
            || !(1..=8).contains(&b.max_rounds)
            || !(1..=256).contains(&b.max_messages)
            || !(1..=1_048_576).contains(&b.max_text_bytes)
            || !(1..=16_777_216).contains(&b.max_latent_logical_bytes)
            || !(1..=16_777_216).contains(&b.max_latent_stored_bytes)
            || !(1..=1_048_576).contains(&b.max_bytes_per_message)
            || !(1..=4096).contains(&b.max_latent_slots)
        {
            return Err(AgentContractError::Invalid(
                "communication budgets must be positive and within runtime ceilings",
            ));
        }
        let mut ids = BTreeSet::new();
        let mut implementers = 0;
        for agent in &self.agents {
            if !ids.insert(&agent.id)
                || agent.max_turns == 0
                || agent.max_turns > b.max_model_attempts
            {
                return Err(AgentContractError::Invalid(
                    "agent IDs must be unique and turn limits within the global attempt limit",
                ));
            }
            if agent.role == AgentRole::Implementer {
                implementers += 1;
            }
            for cap in &agent.capabilities {
                if !matches!(
                    cap,
                    Capability::FileRead
                        | Capability::MemoryRead
                        | Capability::FileWrite
                        | Capability::ProcessSpawn
                ) || (agent.role != AgentRole::Implementer
                    && matches!(cap, Capability::FileWrite))
                    || (agent.role != AgentRole::Implementer
                        && matches!(cap, Capability::ProcessSpawn))
                {
                    return Err(AgentContractError::Invalid(
                        "agent capability exceeds the conservative role ceiling",
                    ));
                }
            }
        }
        if implementers != 1
            || self
                .agents
                .last()
                .is_none_or(|agent| agent.role != AgentRole::Implementer)
        {
            return Err(AgentContractError::Invalid(
                "exactly one implementer is required and must be the final participant",
            ));
        }
        Ok(())
    }
}

/// Invalid experimental configuration or identity.
#[derive(Debug, Error)]
pub enum AgentContractError {
    #[error("invalid agent contract: {0}")]
    Invalid(&'static str),
    #[error("unsupported agent configuration schema {0}")]
    UnsupportedSchema(u32),
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn agent_config_compatibility_fixture_is_validated_by_the_production_reader() {
        let config: AgentRunConfig = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/compatibility/agent-config-v1.json"
        )))
        .unwrap_or_else(|e| unreachable!("fixture: {e}"));
        assert!(config.validate().is_ok());
        let mut future = config;
        future.schema_version = 2;
        assert!(future.validate().is_err());
    }

    #[test]
    fn configuration_rejects_privilege_fanout_round_and_future_schema_changes() {
        let config = AgentRunConfig::coding(CommunicationMode::Text)
            .unwrap_or_else(|e| unreachable!("profile: {e}"));
        assert!(config.validate().is_ok());
        let mut invalid = config.clone();
        invalid.agents[0].capabilities.insert(Capability::FileWrite);
        assert!(invalid.validate().is_err());
        let mut invalid = config.clone();
        invalid.agents[0].id = invalid.agents[1].id.clone();
        assert!(invalid.validate().is_err());
        let mut invalid = config.clone();
        invalid.budget.max_agents = 2;
        assert!(invalid.validate().is_err());
        let mut invalid = config.clone();
        invalid.budget.max_rounds = 0;
        assert!(invalid.validate().is_err());
        let mut invalid = config;
        invalid.schema_version = 2;
        assert!(invalid.validate().is_err());
    }

    proptest! {
        #[test]
        fn identities_validate_on_deserialization(value in ".{0,100}") {
            let expected = !value.is_empty() && value.len() <= 64 && value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            let json = serde_json::to_string(&value).unwrap_or_else(|e| unreachable!("string: {e}"));
            prop_assert_eq!(serde_json::from_str::<AgentId>(&json).is_ok(), expected);
        }
    }
}
