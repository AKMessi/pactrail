//! Read-only binary/state upgrade readiness and deprecation reporting.

use std::path::Path;

use serde::Serialize;

use crate::commands::{CliError, write_json};
use crate::migration::{self, MigrationReport};
use crate::output::write_human_stdout;

const UPGRADE_REPORT_SCHEMA: u32 = 1;
const DEPRECATION_MANIFEST_SCHEMA: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct DeprecationEntry {
    pub id: &'static str,
    pub surface: &'static str,
    pub deprecated_since: &'static str,
    pub removal_version: &'static str,
    pub replacement: &'static str,
    pub rationale: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct DeprecationManifest {
    pub manifest_schema: u32,
    pub pactrail_version: &'static str,
    pub entries: Vec<DeprecationEntry>,
}

#[derive(Debug, Serialize)]
struct UpgradeReport {
    schema: u32,
    pactrail_version: &'static str,
    changes_applied: bool,
    ready_for_current_version: bool,
    state: MigrationReport,
    deprecations: DeprecationManifest,
    next_steps: Vec<String>,
}

#[must_use]
pub(crate) fn deprecation_manifest() -> DeprecationManifest {
    let mut entries = vec![
        DeprecationEntry {
            id: "cli.run.allow_process",
            surface: "pactrail run --allow-process",
            deprecated_since: "0.4.0",
            removal_version: "2.0.0",
            replacement: "--process-backend native --process-approval allow-run",
            rationale: "separate the host-execution boundary from approval authority",
        },
        DeprecationEntry {
            id: "interactive.process_on",
            surface: "/process on",
            deprecated_since: "0.4.0",
            removal_version: "2.0.0",
            replacement: "/process native",
            rationale: "name trusted host execution explicitly",
        },
    ];
    entries.sort_unstable_by_key(|entry| entry.id);
    DeprecationManifest {
        manifest_schema: DEPRECATION_MANIFEST_SCHEMA,
        pactrail_version: env!("CARGO_PKG_VERSION"),
        entries,
    }
}

pub(crate) fn execute(state: &Path, json: bool) -> Result<(), CliError> {
    let state = migration::audit(state)?;
    let deprecations = deprecation_manifest();
    let ready_for_current_version = state.pending_components() == 0;
    let mut next_steps = Vec::new();
    if !ready_for_current_version {
        next_steps.push("review the report, then run `pactrail migrate --apply`".to_owned());
    }
    next_steps.extend(deprecations.entries.iter().map(|entry| {
        format!(
            "replace `{}` with `{}` before {}",
            entry.surface, entry.replacement, entry.removal_version
        )
    }));
    if next_steps.is_empty() {
        next_steps.push("no local upgrade action is required".to_owned());
    }
    let report = UpgradeReport {
        schema: UPGRADE_REPORT_SCHEMA,
        pactrail_version: env!("CARGO_PKG_VERSION"),
        changes_applied: false,
        ready_for_current_version,
        state,
        deprecations,
        next_steps,
    };
    if json {
        write_json(&report)
    } else {
        render_human(&report)
    }
}

fn render_human(report: &UpgradeReport) -> Result<(), CliError> {
    let mut lines = vec![
        format!("Pactrail {} upgrade preflight", report.pactrail_version),
        format!(
            "  state         {}",
            if report.ready_for_current_version {
                "ready"
            } else {
                "migration required"
            }
        ),
        format!(
            "  pending       {} component(s)",
            report.state.pending_components()
        ),
        format!(
            "  deprecations  {} active alias(es)",
            report.deprecations.entries.len()
        ),
    ];
    for entry in &report.deprecations.entries {
        lines.push(format!(
            "  {} -> {} (remove in {})",
            entry.surface, entry.replacement, entry.removal_version
        ));
    }
    lines.push(String::new());
    lines.push("Next steps".to_owned());
    lines.extend(report.next_steps.iter().map(|step| format!("  - {step}")));
    lines.push(String::new());
    lines.push("Read-only preflight complete; no state or source files changed.".to_owned());
    write_human_stdout(&format!("{}\n", lines.join("\n"))).map_err(CliError::Output)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn fixture_pins_the_deprecation_manifest() {
        let expected: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/upgrade/deprecations-v1.json"
        )))
        .unwrap_or_else(|error| unreachable!("deprecation fixture: {error}"));
        let mut actual = serde_json::to_value(deprecation_manifest())
            .unwrap_or_else(|error| unreachable!("deprecation manifest: {error}"));
        actual
            .as_object_mut()
            .unwrap_or_else(|| unreachable!("manifest object"))
            .remove("pactrail_version");
        assert_eq!(actual, expected);
    }

    #[test]
    fn deprecations_are_unique_and_have_explicit_replacements() {
        let manifest = deprecation_manifest();
        let mut ids = BTreeSet::new();
        let mut surfaces = BTreeSet::new();
        for entry in manifest.entries {
            assert!(ids.insert(entry.id));
            assert!(surfaces.insert(entry.surface));
            assert!(!entry.replacement.trim().is_empty());
            assert_eq!(entry.removal_version, "2.0.0");
        }
    }
}
