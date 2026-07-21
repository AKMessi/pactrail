//! Explicit, preflighted migration and integrity audit for local Pactrail state.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use pactrail_core::RunId;
use pactrail_memory::{MEMORY_DATABASE_SCHEMA_VERSION, MemoryStore};
use pactrail_store::{EVENT_DATABASE_SCHEMA_VERSION, EventStore};
use serde::Serialize;

use crate::commands::{CliError, completed_runs, validate_run_artifacts, write_json};
use crate::output::write_human_stdout;
use crate::settings::{MIN_SETTINGS_SCHEMA, SETTINGS_SCHEMA, SettingsStore};

const MIGRATION_REPORT_SCHEMA: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ComponentStatus {
    Missing,
    Current,
    MigrationRequired,
    Incompatible,
}

#[derive(Debug, Serialize)]
struct MigrationComponent {
    id: &'static str,
    path: PathBuf,
    found_schema: Option<i64>,
    current_schema: i64,
    minimum_schema: i64,
    status: ComponentStatus,
}

#[derive(Debug, Default, Serialize)]
struct ValidationSummary {
    event_runs: usize,
    checkpoints: usize,
    receipts: usize,
    mcp_servers: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct MigrationReport {
    schema: u32,
    state_directory: PathBuf,
    applied: bool,
    pending_components: usize,
    changed_components: usize,
    components: Vec<MigrationComponent>,
    validation: ValidationSummary,
}

impl MigrationReport {
    #[must_use]
    pub(crate) const fn pending_components(&self) -> usize {
        self.pending_components
    }
}

pub(crate) fn execute(state: &Path, apply: bool, json: bool) -> Result<(), CliError> {
    if !apply {
        let report = audit(state)?;
        return output_report(&report, json);
    }
    let settings = SettingsStore::discover().map_err(settings_error)?;
    let before = inspect_components(state, &settings)?;
    reject_incompatible(&before)?;
    let changed_components = before
        .iter()
        .filter(|component| component.status == ComponentStatus::MigrationRequired)
        .count();
    let locks = if apply && changed_components > 0 {
        acquire_inactive_run_locks(state)?
    } else {
        Vec::new()
    };
    let validation_before = validate_state(state, &before, &settings)?;
    if apply && changed_components > 0 {
        apply_known_migrations(state, &settings, &before)?;
    }
    let components = inspect_components(state, &settings)?;
    reject_incompatible(&components)?;
    if apply
        && components
            .iter()
            .any(|component| component.status == ComponentStatus::MigrationRequired)
    {
        return Err(CliError::Argument(
            "a supported state migration did not reach its current schema".to_owned(),
        ));
    }
    let validation = if apply && changed_components > 0 {
        validate_state(state, &components, &settings)?
    } else {
        validation_before
    };
    drop(locks);
    let report = MigrationReport {
        schema: MIGRATION_REPORT_SCHEMA,
        state_directory: state.to_path_buf(),
        applied: apply,
        pending_components: changed_components,
        changed_components: if apply { changed_components } else { 0 },
        components,
        validation,
    };
    output_report(&report, json)
}

/// Performs the complete non-creating migration and integrity preflight.
pub(crate) fn audit(state: &Path) -> Result<MigrationReport, CliError> {
    let settings = SettingsStore::discover().map_err(settings_error)?;
    let components = inspect_components(state, &settings)?;
    reject_incompatible(&components)?;
    let pending_components = components
        .iter()
        .filter(|component| component.status == ComponentStatus::MigrationRequired)
        .count();
    let validation = validate_state(state, &components, &settings)?;
    Ok(MigrationReport {
        schema: MIGRATION_REPORT_SCHEMA,
        state_directory: state.to_path_buf(),
        applied: false,
        pending_components,
        changed_components: 0,
        components,
        validation,
    })
}

fn output_report(report: &MigrationReport, json: bool) -> Result<(), CliError> {
    if json {
        write_json(report)
    } else {
        render_human(report)
    }
}

fn inspect_components(
    state: &Path,
    settings: &SettingsStore,
) -> Result<Vec<MigrationComponent>, CliError> {
    let settings_schema = settings
        .schema_version()
        .map_err(settings_error)?
        .map(i64::from);
    let event_path = state.join("events.sqlite3");
    let memory_path = state.join("memory.sqlite3");
    Ok(vec![
        component(
            "event_database",
            event_path.clone(),
            EventStore::database_schema_version(&event_path)?,
            EVENT_DATABASE_SCHEMA_VERSION,
            pactrail_store::MIN_EVENT_DATABASE_SCHEMA_VERSION,
        ),
        component(
            "interactive_settings",
            settings.settings_path(),
            settings_schema,
            i64::from(SETTINGS_SCHEMA),
            i64::from(MIN_SETTINGS_SCHEMA),
        ),
        component(
            "memory_database",
            memory_path.clone(),
            MemoryStore::database_schema_version(&memory_path)?,
            MEMORY_DATABASE_SCHEMA_VERSION,
            pactrail_memory::MIN_MEMORY_DATABASE_SCHEMA_VERSION,
        ),
    ])
}

fn component(
    id: &'static str,
    path: PathBuf,
    found_schema: Option<i64>,
    current_schema: i64,
    minimum_schema: i64,
) -> MigrationComponent {
    let status = match found_schema {
        None => ComponentStatus::Missing,
        Some(found) if found == current_schema => ComponentStatus::Current,
        Some(0) => ComponentStatus::MigrationRequired,
        Some(found) if (minimum_schema..current_schema).contains(&found) => {
            ComponentStatus::MigrationRequired
        }
        Some(_) => ComponentStatus::Incompatible,
    };
    MigrationComponent {
        id,
        path,
        found_schema,
        current_schema,
        minimum_schema,
        status,
    }
}

fn reject_incompatible(components: &[MigrationComponent]) -> Result<(), CliError> {
    let incompatible = components
        .iter()
        .filter(|component| component.status == ComponentStatus::Incompatible)
        .map(|component| {
            format!(
                "{} schema {} (supported {}..={})",
                component.id,
                component.found_schema.unwrap_or(-1),
                component.minimum_schema,
                component.current_schema
            )
        })
        .collect::<Vec<_>>();
    if incompatible.is_empty() {
        Ok(())
    } else {
        Err(CliError::Argument(format!(
            "state migration preflight failed; nothing changed: {}",
            incompatible.join(", ")
        )))
    }
}

fn apply_known_migrations(
    state: &Path,
    settings: &SettingsStore,
    components: &[MigrationComponent],
) -> Result<(), CliError> {
    for component in components {
        if component.status != ComponentStatus::MigrationRequired {
            continue;
        }
        match component.id {
            "event_database" => drop(EventStore::open(state.join("events.sqlite3"))?),
            "interactive_settings" => {
                drop(settings.load().map_err(settings_error)?);
            }
            "memory_database" => drop(MemoryStore::open(state.join("memory.sqlite3"))?),
            _ => {
                return Err(CliError::Argument(format!(
                    "no migration implementation is registered for {}",
                    component.id
                )));
            }
        }
    }
    Ok(())
}

fn validate_state(
    state: &Path,
    components: &[MigrationComponent],
    settings: &SettingsStore,
) -> Result<ValidationSummary, CliError> {
    let mut summary = ValidationSummary::default();
    let receipts = completed_runs(state)?;
    for receipt in &receipts {
        if !receipt.verify_integrity()? {
            return Err(CliError::Argument(format!(
                "receipt {} failed its integrity hash",
                receipt.run_id
            )));
        }
    }
    let event_component = find_component(components, "event_database")?;
    if event_component.found_schema == Some(0) {
        EventStore::validate_uninitialized(&event_component.path)?;
        if !receipts.is_empty() {
            return Err(CliError::Argument(
                "receipts exist beside an uninitialized event database".to_owned(),
            ));
        }
    } else if event_component.found_schema.is_some() {
        let store = EventStore::open_read_only(&event_component.path)?;
        let run_ids = store.list_run_ids()?;
        let mut checkpoint_references = 0_usize;
        for run_id in &run_ids {
            let loaded = store.load(*run_id)?;
            checkpoint_references = checkpoint_references.saturating_add(
                loaded
                    .iter()
                    .filter(|event| {
                        matches!(
                            &event.event,
                            pactrail_core::RunEvent::CheckpointCreated { checkpoint }
                                if checkpoint.starts_with("session:")
                        )
                    })
                    .count(),
            );
        }
        validate_receipt_bindings(&receipts, &store)?;
        if checkpoint_references > 0 {
            let checkpoint_root = state.join("artifacts").join("checkpoints");
            if !checkpoint_root.is_dir() {
                return Err(CliError::Argument(format!(
                    "event journal references checkpoints but {} is missing",
                    checkpoint_root.display()
                )));
            }
            let checkpoints = pactrail_engine::CheckpointStore::open(checkpoint_root)
                .map_err(pactrail_engine::EngineError::Checkpoint)?;
            for run_id in &run_ids {
                summary.checkpoints = summary.checkpoints.saturating_add(
                    checkpoints
                        .validate_all(&store, *run_id)
                        .map_err(pactrail_engine::EngineError::Checkpoint)?,
                );
            }
        }
        summary.event_runs = validate_run_artifacts(state, &store)?;
    } else if !receipts.is_empty() {
        return Err(CliError::Argument(
            "receipts exist without a current event database; their journal binding cannot be verified"
                .to_owned(),
        ));
    }
    let memory_component = find_component(components, "memory_database")?;
    match memory_component.found_schema {
        Some(0) => MemoryStore::validate_uninitialized(&memory_component.path)?,
        Some(_) => {
            let memory = MemoryStore::open_read_only(&memory_component.path)?;
            let _records = memory.validate_all()?;
        }
        None => {}
    }
    if find_component(components, "interactive_settings")?
        .found_schema
        .is_some()
    {
        settings
            .validate_without_migration()
            .map_err(settings_error)?;
    }
    summary.receipts = receipts.len();
    summary.mcp_servers = crate::mcp::validate_local_state(state)?;
    Ok(summary)
}

fn validate_receipt_bindings(
    receipts: &[pactrail_core::ChangeReceipt],
    events: &EventStore,
) -> Result<(), CliError> {
    for receipt in receipts {
        let snapshot = events.snapshot(receipt.run_id)?;
        if snapshot.last_hash.0 != receipt.final_event_hash {
            return Err(CliError::Argument(format!(
                "receipt {} is not bound to the current event head",
                receipt.run_id
            )));
        }
        if snapshot.contract.as_ref() != Some(&receipt.contract) {
            return Err(CliError::Argument(format!(
                "receipt {} contract does not match the event journal",
                receipt.run_id
            )));
        }
    }
    Ok(())
}

fn find_component<'a>(
    components: &'a [MigrationComponent],
    id: &str,
) -> Result<&'a MigrationComponent, CliError> {
    components
        .iter()
        .find(|component| component.id == id)
        .ok_or_else(|| CliError::Argument(format!("migration component {id} is missing")))
}

fn acquire_inactive_run_locks(state: &Path) -> Result<Vec<fs::File>, CliError> {
    let runs = state.join("runs");
    let mut locks = Vec::new();
    let entries = match fs::read_dir(&runs) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(locks),
        Err(source) => return Err(CliError::Io { path: runs, source }),
    };
    for entry in entries {
        let entry = entry.map_err(|source| CliError::Io {
            path: runs.clone(),
            source,
        })?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.parse::<RunId>().is_err() {
            continue;
        }
        let lock_path = entry.path().join("execution.lock");
        if !lock_path.is_file() {
            continue;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|source| CliError::Io {
                path: lock_path.clone(),
                source,
            })?;
        file.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => CliError::Argument(format!(
                "state migration refused because run {name} is active"
            )),
            fs::TryLockError::Error(source) => CliError::Io {
                path: lock_path,
                source,
            },
        })?;
        locks.push(file);
    }
    Ok(locks)
}

fn render_human(report: &MigrationReport) -> Result<(), CliError> {
    let mut lines = vec![
        "Pactrail state migration".to_owned(),
        format!("  state      {}", report.state_directory.display()),
        format!(
            "  operation  {}",
            if report.applied {
                "apply"
            } else {
                "audit only"
            }
        ),
    ];
    for component in &report.components {
        lines.push(format!(
            "  {:<20} {:<20} found {} · current {}",
            component.id,
            status_label(component.status),
            component
                .found_schema
                .map_or_else(|| "none".to_owned(), |value| format!("v{value}")),
            component.current_schema,
        ));
    }
    lines.push(format!(
        "  validated  {} runs · {} checkpoints · {} receipts · {} MCP servers",
        report.validation.event_runs,
        report.validation.checkpoints,
        report.validation.receipts,
        report.validation.mcp_servers
    ));
    if report.applied {
        lines.push(format!(
            "Migration complete · {} component(s) changed",
            report.changed_components
        ));
    } else if report
        .components
        .iter()
        .any(|component| component.status == ComponentStatus::MigrationRequired)
    {
        lines.push(
            "Migration available · rerun with --apply after reviewing this report".to_owned(),
        );
    } else {
        lines.push("State is current · no changes made".to_owned());
    }
    write_human_stdout(&format!("{}\n", lines.join("\n"))).map_err(CliError::Output)
}

const fn status_label(status: ComponentStatus) -> &'static str {
    match status {
        ComponentStatus::Missing => "not present",
        ComponentStatus::Current => "current",
        ComponentStatus::MigrationRequired => "migration required",
        ComponentStatus::Incompatible => "incompatible",
    }
}

fn settings_error(error: impl std::fmt::Display) -> CliError {
    CliError::Argument(format!("settings migration failed: {error}"))
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;

    const SETTINGS_V1: &str = "schema = 1\nprovider = \"ollama\"\napi_key_env = \"KEY\"\ncontext_tokens = 4096\nmax_output_tokens = 512\nmax_turns = 4\nallow_process = false\n";

    #[test]
    fn missing_state_audit_is_read_only() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        let state = root.path().join("state");
        let settings = SettingsStore::at(root.path().join("config"));
        let components = inspect_components(&state, &settings)
            .unwrap_or_else(|error| unreachable!("audit: {error}"));
        assert!(
            components
                .iter()
                .all(|component| component.status == ComponentStatus::Missing)
        );
        assert!(!state.exists());
        assert!(!settings.settings_path().exists());
    }

    #[test]
    fn preflight_refuses_every_migration_when_one_schema_is_future() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        let state = root.path().join("state");
        fs::create_dir_all(&state).unwrap_or_else(|error| unreachable!("state: {error}"));
        let settings = SettingsStore::at(root.path().join("config"));
        fs::create_dir_all(
            settings
                .settings_path()
                .parent()
                .unwrap_or_else(|| unreachable!()),
        )
        .unwrap_or_else(|error| unreachable!("config: {error}"));
        fs::write(settings.settings_path(), SETTINGS_V1)
            .unwrap_or_else(|error| unreachable!("settings: {error}"));
        let event_path = state.join("events.sqlite3");
        let event_connection =
            Connection::open(&event_path).unwrap_or_else(|error| unreachable!("events: {error}"));
        event_connection
            .pragma_update(None, "user_version", 99)
            .unwrap_or_else(|error| unreachable!("future schema: {error}"));
        drop(event_connection);

        let components = inspect_components(&state, &settings)
            .unwrap_or_else(|error| unreachable!("inspect: {error}"));
        assert!(reject_incompatible(&components).is_err());
        assert_eq!(settings.schema_version().ok(), Some(Some(1)));
        assert_eq!(
            EventStore::database_schema_version(event_path).ok(),
            Some(Some(99))
        );
    }

    #[test]
    fn supported_components_migrate_to_current_versions() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        let state = root.path().join("state");
        fs::create_dir_all(&state).unwrap_or_else(|error| unreachable!("state: {error}"));
        let settings = SettingsStore::at(root.path().join("config"));
        fs::create_dir_all(
            settings
                .settings_path()
                .parent()
                .unwrap_or_else(|| unreachable!()),
        )
        .unwrap_or_else(|error| unreachable!("config: {error}"));
        fs::write(settings.settings_path(), SETTINGS_V1)
            .unwrap_or_else(|error| unreachable!("settings: {error}"));

        let event_path = state.join("events.sqlite3");
        drop(
            EventStore::open(&event_path)
                .unwrap_or_else(|error| unreachable!("event fixture: {error}")),
        );
        let event_connection =
            Connection::open(&event_path).unwrap_or_else(|error| unreachable!("events: {error}"));
        event_connection
            .pragma_update(
                None,
                "user_version",
                pactrail_store::MIN_EVENT_DATABASE_SCHEMA_VERSION,
            )
            .unwrap_or_else(|error| unreachable!("event schema: {error}"));
        drop(event_connection);
        let memory_path = state.join("memory.sqlite3");
        drop(
            Connection::open(&memory_path).unwrap_or_else(|error| unreachable!("memory: {error}")),
        );

        let before = inspect_components(&state, &settings)
            .unwrap_or_else(|error| unreachable!("inspect: {error}"));
        assert!(
            before
                .iter()
                .all(|component| component.status == ComponentStatus::MigrationRequired)
        );
        let validation = validate_state(&state, &before, &settings)
            .unwrap_or_else(|error| unreachable!("preflight validation: {error}"));
        assert_eq!(validation.event_runs, 0);
        assert_eq!(settings.schema_version().ok(), Some(Some(1)));
        apply_known_migrations(&state, &settings, &before)
            .unwrap_or_else(|error| unreachable!("migrate: {error}"));
        let after = inspect_components(&state, &settings)
            .unwrap_or_else(|error| unreachable!("inspect: {error}"));
        assert!(
            after
                .iter()
                .all(|component| component.status == ComponentStatus::Current)
        );
    }

    #[test]
    fn active_run_lock_blocks_migration() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        let run_id = RunId::new();
        let run_root = root.path().join("runs").join(run_id.to_string());
        fs::create_dir_all(&run_root).unwrap_or_else(|error| unreachable!("run root: {error}"));
        let lock_path = run_root.join("execution.lock");
        let active = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .unwrap_or_else(|error| unreachable!("lock: {error}"));
        active
            .try_lock()
            .unwrap_or_else(|error| unreachable!("active lock: {error}"));

        assert!(matches!(
            acquire_inactive_run_locks(root.path()),
            Err(CliError::Argument(message)) if message.contains("is active")
        ));
    }
}
