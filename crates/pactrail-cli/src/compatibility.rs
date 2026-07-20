//! Machine-readable compatibility contracts for every persisted Pactrail boundary.

use serde::Serialize;

use crate::commands::{MIN_RUN_MANIFEST_SCHEMA_VERSION, RUN_MANIFEST_SCHEMA_VERSION};
use crate::settings::{MIN_SETTINGS_SCHEMA, SETTINGS_SCHEMA};

const COMPATIBILITY_MANIFEST_SCHEMA: u32 = 1;

/// How Pactrail handles a known non-current version of a format.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CompatibilityStrategy {
    /// The reader accepts the format without rewriting it.
    ReadCompatible,
    /// The reader upgrades the format atomically before normal use.
    MigrateAtomically,
    /// The data is derived and is discarded/rebuilt on a version mismatch.
    RebuildDerived,
    /// Only the exact current schema is accepted.
    ExactVersion,
}

impl CompatibilityStrategy {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::ReadCompatible => "read-compatible",
            Self::MigrateAtomically => "atomic migration",
            Self::RebuildDerived => "safe rebuild",
            Self::ExactVersion => "exact version",
        }
    }
}

/// One compile-time compatibility promise.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct FormatContract {
    pub id: &'static str,
    pub owner: &'static str,
    pub current_schema: u64,
    pub minimum_readable_schema: u64,
    pub strategy: CompatibilityStrategy,
    pub durable: bool,
}

/// Stable JSON envelope printed by `pactrail compatibility --json`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct CompatibilityManifest {
    pub manifest_schema: u32,
    pub pactrail_version: &'static str,
    pub formats: Vec<FormatContract>,
}

#[must_use]
#[allow(clippy::too_many_lines)] // Declarative inventory; splitting obscures its one-to-one audit.
pub(crate) fn manifest() -> CompatibilityManifest {
    let mut formats = vec![
        format(
            "approval_record",
            "pactrail-core",
            pactrail_core::ApprovalRecord::SCHEMA_VERSION,
            pactrail_core::ApprovalRecord::SCHEMA_VERSION,
            CompatibilityStrategy::ExactVersion,
            true,
        ),
        format(
            "capability_probe_report",
            "pactrail-models",
            pactrail_models::CAPABILITY_PROBE_SCHEMA_VERSION,
            pactrail_models::CAPABILITY_PROBE_SCHEMA_VERSION,
            CompatibilityStrategy::ExactVersion,
            false,
        ),
        format(
            "change_receipt",
            "pactrail-core",
            pactrail_core::ChangeReceipt::SCHEMA_VERSION,
            pactrail_core::ChangeReceipt::SCHEMA_VERSION,
            CompatibilityStrategy::ExactVersion,
            true,
        ),
        format(
            "event_database",
            "pactrail-store",
            pactrail_store::EVENT_DATABASE_SCHEMA_VERSION,
            pactrail_store::MIN_EVENT_DATABASE_SCHEMA_VERSION,
            CompatibilityStrategy::MigrateAtomically,
            true,
        ),
        format(
            "event_envelope",
            "pactrail-core",
            pactrail_core::EVENT_SCHEMA_VERSION,
            pactrail_core::MIN_EVENT_SCHEMA_VERSION,
            CompatibilityStrategy::ReadCompatible,
            true,
        ),
        format(
            "interactive_settings",
            "pactrail",
            SETTINGS_SCHEMA,
            MIN_SETTINGS_SCHEMA,
            CompatibilityStrategy::MigrateAtomically,
            true,
        ),
        format(
            "lsp_reference_snapshot",
            "pactrail-context",
            pactrail_context::LSP_REFERENCE_SNAPSHOT_SCHEMA_VERSION,
            pactrail_context::LSP_REFERENCE_SNAPSHOT_SCHEMA_VERSION,
            CompatibilityStrategy::ExactVersion,
            true,
        ),
        format(
            "mcp_manifest",
            "pactrail-mcp",
            pactrail_mcp::MCP_MANIFEST_SCHEMA,
            pactrail_mcp::MCP_MANIFEST_SCHEMA,
            CompatibilityStrategy::ExactVersion,
            true,
        ),
        format(
            "mcp_snapshot",
            "pactrail-mcp",
            pactrail_mcp::MCP_SNAPSHOT_SCHEMA,
            pactrail_mcp::MCP_SNAPSHOT_SCHEMA,
            CompatibilityStrategy::ExactVersion,
            true,
        ),
        format(
            "memory_database",
            "pactrail-memory",
            pactrail_memory::MEMORY_DATABASE_SCHEMA_VERSION,
            pactrail_memory::MIN_MEMORY_DATABASE_SCHEMA_VERSION,
            CompatibilityStrategy::ExactVersion,
            true,
        ),
        format(
            "model_ir",
            "pactrail-models",
            pactrail_models::MODEL_IR_SCHEMA_VERSION,
            pactrail_models::MODEL_IR_SCHEMA_VERSION,
            CompatibilityStrategy::ExactVersion,
            false,
        ),
        format(
            "repository_index_cache",
            "pactrail-context",
            pactrail_context::INDEX_CACHE_SCHEMA_VERSION,
            pactrail_context::INDEX_CACHE_SCHEMA_VERSION,
            CompatibilityStrategy::RebuildDerived,
            false,
        ),
        format(
            "run_checkpoint",
            "pactrail-engine",
            pactrail_engine::CHECKPOINT_SCHEMA_VERSION,
            pactrail_engine::MIN_CHECKPOINT_SCHEMA_VERSION,
            CompatibilityStrategy::ExactVersion,
            true,
        ),
        format(
            "run_manifest",
            "pactrail",
            RUN_MANIFEST_SCHEMA_VERSION,
            MIN_RUN_MANIFEST_SCHEMA_VERSION,
            CompatibilityStrategy::ExactVersion,
            true,
        ),
        format(
            "task_contract",
            "pactrail-core",
            pactrail_core::TaskContract::SCHEMA_VERSION,
            pactrail_core::TaskContract::SCHEMA_VERSION,
            CompatibilityStrategy::ExactVersion,
            true,
        ),
        format(
            "tool_descriptor",
            "pactrail-tools",
            pactrail_tools::TOOL_DESCRIPTOR_SCHEMA_VERSION,
            pactrail_tools::TOOL_DESCRIPTOR_SCHEMA_VERSION,
            CompatibilityStrategy::ExactVersion,
            false,
        ),
        format(
            "transaction_metadata",
            "pactrail-workspace",
            pactrail_workspace::TRANSACTION_SCHEMA_VERSION,
            pactrail_workspace::MIN_TRANSACTION_SCHEMA_VERSION,
            CompatibilityStrategy::ExactVersion,
            true,
        ),
    ];
    formats.sort_unstable_by_key(|format| format.id);
    CompatibilityManifest {
        manifest_schema: COMPATIBILITY_MANIFEST_SCHEMA,
        pactrail_version: env!("CARGO_PKG_VERSION"),
        formats,
    }
}

fn format(
    id: &'static str,
    owner: &'static str,
    current_schema: impl TryInto<u64>,
    minimum_readable_schema: impl TryInto<u64>,
    strategy: CompatibilityStrategy,
    durable: bool,
) -> FormatContract {
    FormatContract {
        id,
        owner,
        current_schema: current_schema.try_into().unwrap_or(u64::MAX),
        minimum_readable_schema: minimum_readable_schema.try_into().unwrap_or(u64::MAX),
        strategy,
        durable,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn inventory_is_unique_ordered_and_has_sane_ranges() {
        let manifest = manifest();
        let mut ids = BTreeSet::new();
        let mut previous = None;
        for contract in &manifest.formats {
            assert!(ids.insert(contract.id), "duplicate format {}", contract.id);
            assert!(contract.minimum_readable_schema > 0);
            assert!(contract.minimum_readable_schema <= contract.current_schema);
            if let Some(previous) = previous {
                assert!(previous < contract.id, "inventory must be lexical");
            }
            previous = Some(contract.id);
        }
    }

    #[test]
    fn fixture_pins_the_public_manifest_shape() {
        let expected: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/compatibility/manifest-v1.json"
        )))
        .unwrap_or_else(|error| unreachable!("compatibility fixture: {error}"));
        let mut actual = serde_json::to_value(manifest())
            .unwrap_or_else(|error| unreachable!("compatibility manifest: {error}"));
        actual
            .as_object_mut()
            .unwrap_or_else(|| unreachable!("manifest object"))
            .remove("pactrail_version");
        assert_eq!(actual, expected);
    }
}
