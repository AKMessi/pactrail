use pactrail_models::{CapabilitySource, ModelCapabilities};

const COMPACT_INPUT_CEILING: u64 = 16 * 1024;
const EXPANDED_INPUT_FLOOR: u64 = 96 * 1024;

/// Capacity tier selected from a model's explicit effective capabilities.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdaptiveRuntimeClass {
    /// Small context/output capacity; favor short, serial, focused turns.
    Compact,
    /// General-purpose capacity with moderate batching and discovery.
    Balanced,
    /// Large context/output capacity with wider safe read batches.
    Expanded,
}

impl AdaptiveRuntimeClass {
    /// Stable human and trace label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Balanced => "balanced",
            Self::Expanded => "expanded",
        }
    }
}

/// Deterministic orchestration limits derived without provider-name heuristics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdaptiveRuntimeProfile {
    pub class: AdaptiveRuntimeClass,
    pub capability_source: CapabilitySource,
    pub input_tokens: u64,
    pub turn_output_tokens: u64,
    pub discovery_turn_cap: u16,
    pub max_tool_calls_per_turn: usize,
    pub parallel_read_width: usize,
}

impl AdaptiveRuntimeProfile {
    /// Stable provenance label for UI and durable trace attributes.
    #[must_use]
    pub const fn capability_source_label(self) -> &'static str {
        match self.capability_source {
            CapabilitySource::ConservativeDefault => "conservative_default",
            CapabilitySource::UserDeclared => "user_declared",
            CapabilitySource::Probed => "probed",
        }
    }

    /// Derives one immutable runtime profile from declared or probed features.
    ///
    /// The provider/model name is deliberately absent. Capacity controls the
    /// tier, while parallel execution remains disabled unless the effective
    /// capability profile explicitly enables parallel tool calls.
    #[must_use]
    pub fn from_capabilities(capabilities: &ModelCapabilities) -> Self {
        let input_tokens = capabilities
            .context_tokens
            .saturating_sub(capabilities.max_output_tokens);
        let class = if input_tokens <= COMPACT_INPUT_CEILING
            || capabilities.max_output_tokens <= 1_024
        {
            AdaptiveRuntimeClass::Compact
        } else if input_tokens >= EXPANDED_INPUT_FLOOR && capabilities.max_output_tokens >= 4_096 {
            AdaptiveRuntimeClass::Expanded
        } else {
            AdaptiveRuntimeClass::Balanced
        };
        let (turn_output_cap, discovery_turn_cap, max_tool_calls_per_turn, parallel_cap) =
            match class {
                AdaptiveRuntimeClass::Compact => (2_048, 2, 4, 2),
                AdaptiveRuntimeClass::Balanced => (4_096, 4, 12, 4),
                AdaptiveRuntimeClass::Expanded => (8_192, 6, 24, 8),
            };
        Self {
            class,
            capability_source: capabilities.source,
            input_tokens,
            turn_output_tokens: capabilities.max_output_tokens.min(turn_output_cap),
            discovery_turn_cap,
            max_tool_calls_per_turn,
            parallel_read_width: if capabilities.parallel_tools {
                parallel_cap
            } else {
                1
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_uses_capacity_and_explicit_parallel_support_not_provider_identity() {
        let compact = AdaptiveRuntimeProfile::from_capabilities(&ModelCapabilities {
            context_tokens: 4_096,
            max_output_tokens: 512,
            parallel_tools: true,
            source: CapabilitySource::Probed,
            ..ModelCapabilities::default()
        });
        assert_eq!(compact.class, AdaptiveRuntimeClass::Compact);
        assert_eq!(compact.turn_output_tokens, 512);
        assert_eq!(compact.discovery_turn_cap, 2);
        assert_eq!(compact.parallel_read_width, 2);

        let expanded = AdaptiveRuntimeProfile::from_capabilities(&ModelCapabilities {
            context_tokens: 131_072,
            max_output_tokens: 8_192,
            parallel_tools: false,
            source: CapabilitySource::UserDeclared,
            ..ModelCapabilities::default()
        });
        assert_eq!(expanded.class, AdaptiveRuntimeClass::Expanded);
        assert_eq!(expanded.max_tool_calls_per_turn, 24);
        assert_eq!(expanded.parallel_read_width, 1);
        assert_eq!(expanded.capability_source_label(), "user_declared");
    }

    #[test]
    fn default_capabilities_select_the_balanced_profile() {
        let profile = AdaptiveRuntimeProfile::from_capabilities(&ModelCapabilities::default());
        assert_eq!(profile.class, AdaptiveRuntimeClass::Balanced);
        assert_eq!(profile.turn_output_tokens, 4_096);
        assert_eq!(profile.discovery_turn_cap, 4);
    }
}
