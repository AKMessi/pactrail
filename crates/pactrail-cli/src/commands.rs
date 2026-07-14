use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::str::FromStr;
use std::time::Duration;

use pactrail_core::{
    Capability, ChangeReceipt, ReceiptInput, ReceiptOutcome, RunEvent, RunId, RunState,
    TaskContract,
};
use pactrail_engine::{EngineError, RunEngine};
use pactrail_models::{
    ModelCapabilities, ModelError, OpenAiCompatibleConfig, OpenAiCompatibleDriver,
};
use pactrail_store::{EventStore, StoreError};
use pactrail_tools::{PolicyEngine, ToolError, builtin_registry};
use pactrail_workspace::{TransactionError, WorkspaceTransaction};
use schemars::schema_for;
use secrecy::SecretString;
use serde_json::{Value, json};
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::cli::{Cli, Command, OutputFormat, ProviderKind, RunArgs, RunIdArgs};
use crate::output::write_stdout;

pub async fn dispatch(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::Run(args) => run(&cli.workspace, cli.state_dir.as_deref(), args).await,
        Command::Inspect(args) => {
            let state = state_dir(&cli.workspace, cli.state_dir.as_deref())?;
            inspect(&state, &args)
        }
        Command::Apply(args) => {
            let state = state_dir(&cli.workspace, cli.state_dir.as_deref())?;
            apply(&state, &args)
        }
        Command::Discard(args) => {
            let state = state_dir(&cli.workspace, cli.state_dir.as_deref())?;
            discard(&state, &args)
        }
        Command::List { json } => {
            let state = state_dir(&cli.workspace, cli.state_dir.as_deref())?;
            list(&state, json)
        }
        Command::Tools { json } => tools(json),
        Command::Schema => schema(),
        Command::TaskTemplate { goal } => task_template(&cli.workspace, goal),
        Command::Doctor { json } => doctor(json),
    }
}

async fn run(
    cli_workspace: &Path,
    state_override: Option<&Path>,
    args: RunArgs,
) -> Result<(), CliError> {
    let (mut contract, workspace) = load_contract(cli_workspace, &args)?;
    contract.workspace_root = workspace.display().to_string();
    if args.task.is_none() {
        contract.allowed_write_paths.clone_from(&args.write_paths);
        contract.permissions.allow.insert(Capability::FileRead);
        contract.permissions.allow.insert(Capability::FileWrite);
        contract.permissions.deny.insert(Capability::Network);
        contract.permissions.deny.insert(Capability::SecretUse);
        contract.permissions.deny.insert(Capability::ExternalWrite);
    }
    configure_native_process_permissions(&mut contract, args.allow_process)?;
    contract.validate().map_err(CliError::Contract)?;
    for required in [Capability::FileRead, Capability::FileWrite] {
        if !contract.permissions.allow.contains(&required) {
            return Err(CliError::Argument(format!(
                "task contract must explicitly allow {required}"
            )));
        }
    }

    let state = if let Some(override_path) = state_override {
        absolute_or_join(cli_workspace, override_path)?
    } else {
        workspace.join(".pactrail")
    };
    fs::create_dir_all(state.join("runs")).map_err(|source| CliError::Io {
        path: state.clone(),
        source,
    })?;
    let run_id = RunId::new();
    let run_root = state.join("runs").join(run_id.to_string());
    let transaction =
        WorkspaceTransaction::create(&workspace, &run_root, &contract.allowed_write_paths)?;
    let mut store = EventStore::open(state.join("events.sqlite3"))?;
    let registry = builtin_registry()?;
    let policy = PolicyEngine::new(contract.permissions.clone());
    let driver = build_driver(&contract, &args)?;
    let engine = RunEngine::new(&driver, &registry, &policy).with_max_turns(args.max_turns);
    let outcome = engine
        .execute_with_id(run_id, contract, &transaction, &mut store)
        .await?;
    let mut receipt = outcome.receipt;
    write_receipt(&run_root, &receipt)?;

    if args.apply && receipt.outcome == ReceiptOutcome::ReadyToApply {
        receipt = apply_ready_receipt(&run_root, receipt, &transaction, &mut store)?;
    }
    render_run(
        &run_root,
        &outcome.final_text,
        &receipt,
        outcome.usage.total(),
        args.output,
    )
}

fn load_contract(
    cli_workspace: &Path,
    args: &RunArgs,
) -> Result<(TaskContract, PathBuf), CliError> {
    if let Some(path) = &args.task {
        let task_path = absolute_or_join(cli_workspace, path)?;
        let text = fs::read_to_string(&task_path).map_err(|source| CliError::Io {
            path: task_path.clone(),
            source,
        })?;
        let contract: TaskContract = toml::from_str(&text).map_err(CliError::TaskToml)?;
        let base = task_path.parent().unwrap_or(cli_workspace);
        let workspace = absolute_or_join(base, Path::new(&contract.workspace_root))?;
        return Ok((contract, workspace));
    }
    let goal = args
        .goal
        .clone()
        .ok_or_else(|| CliError::Argument("goal or --task is required".to_owned()))?;
    let workspace = fs::canonicalize(cli_workspace).map_err(|source| CliError::Io {
        path: cli_workspace.to_path_buf(),
        source,
    })?;
    Ok((
        TaskContract::new(goal, workspace.display().to_string()),
        workspace,
    ))
}

fn build_driver(
    contract: &TaskContract,
    args: &RunArgs,
) -> Result<OpenAiCompatibleDriver, CliError> {
    let model = args
        .model
        .clone()
        .or_else(|| contract.model.clone())
        .ok_or_else(|| {
            CliError::Argument("a model is required; pass --model or set PACTRAIL_MODEL".to_owned())
        })?;
    let capabilities = ModelCapabilities {
        context_tokens: args.context_tokens,
        max_output_tokens: args.max_output_tokens,
        ..ModelCapabilities::default()
    };
    let config = match args.provider {
        ProviderKind::Ollama => OpenAiCompatibleConfig {
            name: "ollama".to_owned(),
            base_url: args
                .base_url
                .clone()
                .unwrap_or_else(|| "http://127.0.0.1:11434/v1".to_owned()),
            model,
            api_key: None,
            timeout: Duration::from_mins(5),
            capabilities,
        },
        ProviderKind::OpenAi => OpenAiCompatibleConfig {
            name: "openai".to_owned(),
            base_url: args
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1".to_owned()),
            model,
            api_key: Some(api_key_from_env(&args.api_key_env)?),
            timeout: Duration::from_mins(5),
            capabilities,
        },
        ProviderKind::OpenAiCompatible => OpenAiCompatibleConfig {
            name: "openai-compatible".to_owned(),
            base_url: args.base_url.clone().ok_or_else(|| {
                CliError::Argument("--base-url is required for open-ai-compatible".to_owned())
            })?,
            model,
            api_key: std::env::var(&args.api_key_env)
                .ok()
                .map(SecretString::from),
            timeout: Duration::from_mins(5),
            capabilities,
        },
    };
    OpenAiCompatibleDriver::new(config).map_err(CliError::Model)
}

fn api_key_from_env(name: &str) -> Result<SecretString, CliError> {
    std::env::var(name).map(SecretString::from).map_err(|_| {
        CliError::Argument(format!(
            "required API key environment variable {name:?} is not set"
        ))
    })
}

fn configure_native_process_permissions(
    contract: &mut TaskContract,
    cli_opt_in: bool,
) -> Result<(), CliError> {
    let effective = [
        Capability::ProcessSpawn,
        Capability::Network,
        Capability::SecretUse,
        Capability::ExternalWrite,
    ];
    if cli_opt_in {
        for capability in &effective {
            contract.permissions.deny.remove(capability);
            contract.permissions.allow.insert(capability.clone());
        }
    }
    if contract
        .permissions
        .allow
        .contains(&Capability::ProcessSpawn)
    {
        let missing = effective
            .iter()
            .filter(|capability| !contract.permissions.allow.contains(*capability))
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(CliError::Argument(format!(
                "native process execution is unsandboxed; its contract must also allow: {}",
                missing.join(", ")
            )));
        }
    }
    Ok(())
}

fn inspect(state: &Path, args: &RunIdArgs) -> Result<(), CliError> {
    let run_id = parse_run_id(&args.run_id)?;
    let run_root = run_root(state, run_id);
    let receipt_path = run_root.join("receipt.json");
    if receipt_path.is_file() {
        let receipt = read_receipt(&run_root)?;
        if args.json {
            return write_json(&receipt);
        }
        let integrity = receipt.verify_integrity()?;
        let text = format!(
            "Run: {}\nOutcome: {:?}\nGoal: {}\nChanges: {}\nEvidence: {} passed, {} failed, {} inconclusive\nIntegrity: {}\nReceipt: {}\n",
            receipt.run_id,
            receipt.outcome,
            receipt.contract.goal,
            receipt.changes.len(),
            receipt.verification.passed,
            receipt.verification.failed,
            receipt.verification.inconclusive,
            if integrity { "valid" } else { "INVALID" },
            receipt_path.display(),
        );
        return write_stdout(&text).map_err(CliError::Output);
    }
    let store = EventStore::open(state.join("events.sqlite3"))?;
    let snapshot = store.snapshot(run_id)?;
    let value = json!({
        "run_id": run_id,
        "state": format!("{:?}", snapshot.state),
        "events": snapshot.last_sequence.map_or(0, |sequence| sequence + 1),
        "receipt": null,
    });
    if args.json {
        write_json(&value)
    } else {
        write_stdout(&format!(
            "Run: {run_id}\nState: {:?}\nReceipt: not available\n",
            snapshot.state
        ))
        .map_err(CliError::Output)
    }
}

fn apply(state: &Path, args: &RunIdArgs) -> Result<(), CliError> {
    let run_id = parse_run_id(&args.run_id)?;
    let run_root = run_root(state, run_id);
    let receipt = read_receipt(&run_root)?;
    let transaction = WorkspaceTransaction::open(&run_root)?;
    let mut store = EventStore::open(state.join("events.sqlite3"))?;
    let receipt = apply_ready_receipt(&run_root, receipt, &transaction, &mut store)?;
    if args.json {
        write_json(&receipt)
    } else {
        write_stdout(&format!(
            "Applied run {} to {} ({} files).\n",
            run_id,
            transaction.source_root().display(),
            receipt.changes.len()
        ))
        .map_err(CliError::Output)
    }
}

fn apply_ready_receipt(
    run_root: &Path,
    receipt: ChangeReceipt,
    transaction: &WorkspaceTransaction,
    store: &mut EventStore,
) -> Result<ChangeReceipt, CliError> {
    require_receipt_integrity(&receipt)?;
    let snapshot = store.snapshot(receipt.run_id)?;
    if snapshot.state == RunState::Applied {
        transaction.apply_expected(&receipt.changes)?;
        if receipt.outcome == ReceiptOutcome::Applied {
            if receipt.final_event_hash != snapshot.last_hash.0 {
                return Err(CliError::Argument(
                    "applied receipt does not match the durable event head".to_owned(),
                ));
            }
            return Ok(receipt);
        }
        if receipt.outcome == ReceiptOutcome::ReadyToApply {
            let applied = rebuild_receipt(receipt, ReceiptOutcome::Applied, snapshot.last_hash.0)?;
            write_receipt(run_root, &applied)?;
            return Ok(applied);
        }
    }
    if snapshot.state != RunState::AwaitingApply || receipt.outcome != ReceiptOutcome::ReadyToApply
    {
        return Err(state_receipt_mismatch(
            snapshot.state,
            receipt.outcome,
            "apply",
        ));
    }
    transaction.apply_expected(&receipt.changes)?;
    let sequence = snapshot.last_sequence.map_or(0, |value| value + 1);
    let event = store.append(
        receipt.run_id,
        sequence,
        RunEvent::StateChanged {
            from: RunState::AwaitingApply,
            to: RunState::Applied,
        },
    )?;
    let applied = rebuild_receipt(receipt, ReceiptOutcome::Applied, event.hash.0)?;
    write_receipt(run_root, &applied)?;
    Ok(applied)
}

fn discard(state: &Path, args: &RunIdArgs) -> Result<(), CliError> {
    let run_id = parse_run_id(&args.run_id)?;
    let run_root = run_root(state, run_id);
    let receipt = read_receipt(&run_root)?;
    let transaction = WorkspaceTransaction::open(&run_root)?;
    let mut store = EventStore::open(state.join("events.sqlite3"))?;
    let discarded = discard_ready_receipt(&run_root, receipt, &transaction, &mut store)?;
    if args.json {
        write_json(&discarded)
    } else {
        write_stdout(&format!("Discarded run {run_id}; receipt preserved.\n"))
            .map_err(CliError::Output)
    }
}

fn discard_ready_receipt(
    run_root: &Path,
    receipt: ChangeReceipt,
    transaction: &WorkspaceTransaction,
    store: &mut EventStore,
) -> Result<ChangeReceipt, CliError> {
    require_receipt_integrity(&receipt)?;
    let snapshot = store.snapshot(receipt.run_id)?;
    let workspace = transaction.workspace_root().to_path_buf();
    let staged = run_root.join("discarded-workspace");

    if snapshot.state == RunState::Discarded {
        if workspace.exists() {
            return Err(CliError::Argument(format!(
                "discarded run still has a transaction workspace at {}",
                workspace.display()
            )));
        }
        remove_staged_workspace(&staged)?;
        if receipt.outcome == ReceiptOutcome::Discarded {
            if receipt.final_event_hash != snapshot.last_hash.0 {
                return Err(CliError::Argument(
                    "discarded receipt does not match the durable event head".to_owned(),
                ));
            }
            return Ok(receipt);
        }
        if receipt.outcome == ReceiptOutcome::ReadyToApply {
            let discarded =
                rebuild_receipt(receipt, ReceiptOutcome::Discarded, snapshot.last_hash.0)?;
            write_receipt(run_root, &discarded)?;
            return Ok(discarded);
        }
    }
    if snapshot.state != RunState::AwaitingApply || receipt.outcome != ReceiptOutcome::ReadyToApply
    {
        return Err(state_receipt_mismatch(
            snapshot.state,
            receipt.outcome,
            "discard",
        ));
    }

    if workspace.exists() && !staged.exists() {
        fs::rename(&workspace, &staged).map_err(|source| CliError::Io {
            path: workspace.clone(),
            source,
        })?;
    } else if workspace.exists() || !staged.is_dir() {
        return Err(CliError::Argument(format!(
            "discard staging is inconsistent (workspace={}, staged={})",
            workspace.exists(),
            staged.exists()
        )));
    }
    let sequence = snapshot.last_sequence.map_or(0, |value| value + 1);
    let appended = store.append(
        receipt.run_id,
        sequence,
        RunEvent::StateChanged {
            from: RunState::AwaitingApply,
            to: RunState::Discarded,
        },
    );
    let event = match appended {
        Ok(event) => event,
        Err(error) => {
            let _restore = fs::rename(&staged, &workspace);
            return Err(CliError::Store(error));
        }
    };
    remove_staged_workspace(&staged)?;
    let discarded = rebuild_receipt(receipt, ReceiptOutcome::Discarded, event.hash.0)?;
    write_receipt(run_root, &discarded)?;
    Ok(discarded)
}

fn rebuild_receipt(
    receipt: ChangeReceipt,
    outcome: ReceiptOutcome,
    final_event_hash: String,
) -> Result<ChangeReceipt, CliError> {
    ChangeReceipt::build(ReceiptInput {
        run_id: receipt.run_id,
        contract: receipt.contract,
        outcome,
        baseline_digest: receipt.baseline_digest,
        final_event_hash,
        changes: receipt.changes,
        evidence: receipt.evidence,
        unresolved_risks: receipt.unresolved_risks,
    })
    .map_err(CliError::Receipt)
}

fn require_receipt_integrity(receipt: &ChangeReceipt) -> Result<(), CliError> {
    if !receipt.verify_integrity()? {
        return Err(CliError::Argument(
            "receipt integrity check failed; refusing state change".to_owned(),
        ));
    }
    Ok(())
}

fn remove_staged_workspace(staged: &Path) -> Result<(), CliError> {
    if staged.exists() {
        fs::remove_dir_all(staged).map_err(|source| CliError::Io {
            path: staged.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

fn state_receipt_mismatch(state: RunState, outcome: ReceiptOutcome, operation: &str) -> CliError {
    CliError::Argument(format!(
        "cannot {operation}: durable run state is {state:?}, receipt outcome is {outcome:?}"
    ))
}

fn list(state: &Path, json_output: bool) -> Result<(), CliError> {
    let runs = state.join("runs");
    if !runs.is_dir() {
        return if json_output {
            write_json(&Vec::<Value>::new())
        } else {
            write_stdout("No Pactrail runs found.\n").map_err(CliError::Output)
        };
    }
    let mut values = Vec::new();
    for entry in fs::read_dir(&runs).map_err(|source| CliError::Io {
        path: runs.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| CliError::Io {
            path: runs.clone(),
            source,
        })?;
        let receipt_path = entry.path().join("receipt.json");
        if !receipt_path.is_file() {
            continue;
        }
        let receipt = read_receipt(entry.path().as_path())?;
        values.push(json!({
            "run_id": receipt.run_id,
            "outcome": receipt.outcome,
            "goal": receipt.contract.goal,
            "changes": receipt.changes.len(),
            "created_at": receipt.created_at,
        }));
    }
    values.sort_by(|left, right| left["run_id"].as_str().cmp(&right["run_id"].as_str()));
    if json_output {
        write_json(&values)
    } else if values.is_empty() {
        write_stdout("No completed Pactrail runs found.\n").map_err(CliError::Output)
    } else {
        let text = values
            .iter()
            .map(|value| {
                format!(
                    "{}  {:<16}  {}",
                    value["run_id"].as_str().unwrap_or_default(),
                    value["outcome"].as_str().unwrap_or_default(),
                    value["goal"].as_str().unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        write_stdout(&format!("{text}\n")).map_err(CliError::Output)
    }
}

fn tools(json_output: bool) -> Result<(), CliError> {
    let descriptors = builtin_registry()?.descriptors();
    if json_output {
        write_json(&descriptors)
    } else {
        let text = descriptors
            .iter()
            .map(|tool| {
                format!(
                    "{:<16} {:<14} {}",
                    tool.name,
                    tool.required_capability.to_string(),
                    tool.description
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        write_stdout(&format!("{text}\n")).map_err(CliError::Output)
    }
}

fn schema() -> Result<(), CliError> {
    write_json(&schema_for!(TaskContract))
}

fn task_template(workspace: &Path, goal: String) -> Result<(), CliError> {
    let workspace = fs::canonicalize(workspace).map_err(|source| CliError::Io {
        path: workspace.to_path_buf(),
        source,
    })?;
    let mut contract = TaskContract::new(goal, workspace.display().to_string());
    contract.permissions.allow.insert(Capability::FileRead);
    contract.permissions.allow.insert(Capability::FileWrite);
    contract.permissions.deny.insert(Capability::Network);
    contract.permissions.deny.insert(Capability::SecretUse);
    contract.permissions.deny.insert(Capability::ExternalWrite);
    let mut text = toml::to_string_pretty(&contract).map_err(CliError::TaskTomlSerialize)?;
    text.push('\n');
    write_stdout(&text).map_err(CliError::Output)
}

fn doctor(json_output: bool) -> Result<(), CliError> {
    let commands = ["git", "cargo", "rustc", "docker", "podman", "ollama"];
    let checks = commands
        .iter()
        .map(|program| {
            let available = ProcessCommand::new(program)
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success());
            json!({ "program": program, "available": available })
        })
        .collect::<Vec<_>>();
    let report = json!({
        "native_process_isolation": "workspace transaction only; not a host-filesystem or network sandbox",
        "recommended_hostile_repo_backend": "OCI via Docker or Podman",
        "commands": checks,
    });
    if json_output {
        write_json(&report)
    } else {
        let mut lines = vec![
            "Native execution protects the working tree but is not a host-filesystem/network sandbox."
                .to_owned(),
            "Use Docker or Podman for hostile repositories when OCI support is configured."
                .to_owned(),
        ];
        for check in checks {
            lines.push(format!(
                "{:<10} {}",
                check["program"].as_str().unwrap_or_default(),
                if check["available"].as_bool().unwrap_or(false) {
                    "available"
                } else {
                    "not found"
                }
            ));
        }
        write_stdout(&format!("{}\n", lines.join("\n"))).map_err(CliError::Output)
    }
}

fn render_run(
    run_root: &Path,
    model_summary: &str,
    receipt: &ChangeReceipt,
    tokens: u64,
    output: OutputFormat,
) -> Result<(), CliError> {
    match output {
        OutputFormat::Json => write_json(&json!({
            "run_id": receipt.run_id,
            "outcome": receipt.outcome,
            "summary": model_summary,
            "changes": receipt.changes,
            "verification": receipt.verification,
            "risks": receipt.unresolved_risks,
            "tokens": tokens,
            "receipt": run_root.join("receipt.json"),
        })),
        OutputFormat::Human => {
            let changes = if receipt.changes.is_empty() {
                "  (none)".to_owned()
            } else {
                receipt
                    .changes
                    .iter()
                    .map(|change| format!("  {}", change.path))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let apply_hint = if receipt.outcome == ReceiptOutcome::ReadyToApply {
                format!("pactrail apply {}", receipt.run_id)
            } else {
                "not applicable".to_owned()
            };
            write_stdout(&format!(
                "Run: {}\nOutcome: {:?}\n\n{}\n\nChanged files:\n{}\n\nEvidence: {} passed, {} failed, {} inconclusive\nTokens: {}\nReceipt: {}\nApply: {}\n",
                receipt.run_id,
                receipt.outcome,
                model_summary,
                changes,
                receipt.verification.passed,
                receipt.verification.failed,
                receipt.verification.inconclusive,
                tokens,
                run_root.join("receipt.json").display(),
                apply_hint,
            ))
            .map_err(CliError::Output)
        }
    }
}

fn state_dir(workspace: &Path, override_path: Option<&Path>) -> Result<PathBuf, CliError> {
    override_path.map_or_else(
        || absolute_or_join(workspace, Path::new(".pactrail")),
        |path| absolute_or_join(workspace, path),
    )
}

fn absolute_or_join(base: &Path, path: &Path) -> Result<PathBuf, CliError> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    if joined.exists() {
        fs::canonicalize(&joined).map_err(|source| CliError::Io {
            path: joined,
            source,
        })
    } else {
        Ok(joined)
    }
}

fn run_root(state: &Path, run_id: RunId) -> PathBuf {
    state.join("runs").join(run_id.to_string())
}

fn parse_run_id(value: &str) -> Result<RunId, CliError> {
    RunId::from_str(value)
        .map_err(|error| CliError::Argument(format!("invalid run id {value:?}: {error}")))
}

fn read_receipt(run_root: &Path) -> Result<ChangeReceipt, CliError> {
    let path = run_root.join("receipt.json");
    let backup = run_root.join("receipt.json.bak");
    if !path.exists() && backup.is_file() {
        fs::rename(&backup, &path).map_err(|source| CliError::Io {
            path: backup,
            source,
        })?;
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|source| CliError::Io {
            path: path.clone(),
            source,
        })?;
    serde_json::from_slice(&bytes).map_err(CliError::Json)
}

fn write_receipt(run_root: &Path, receipt: &ChangeReceipt) -> Result<(), CliError> {
    let path = run_root.join("receipt.json");
    let backup = run_root.join("receipt.json.bak");
    if backup.exists() {
        if path.exists() {
            fs::remove_file(&backup).map_err(|source| CliError::Io {
                path: backup.clone(),
                source,
            })?;
        } else {
            fs::rename(&backup, &path).map_err(|source| CliError::Io {
                path: backup.clone(),
                source,
            })?;
        }
    }
    let bytes = serde_json::to_vec_pretty(receipt).map_err(CliError::Json)?;
    let mut temporary = NamedTempFile::new_in(run_root).map_err(|source| CliError::Io {
        path: run_root.to_path_buf(),
        source,
    })?;
    temporary
        .write_all(&bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|source| CliError::Io {
            path: temporary.path().to_path_buf(),
            source,
        })?;
    if path.exists() {
        fs::rename(&path, &backup).map_err(|source| CliError::Io {
            path: path.clone(),
            source,
        })?;
    }
    if let Err(error) = temporary.persist(&path) {
        if backup.exists() {
            let _restore = fs::rename(&backup, &path);
        }
        return Err(CliError::Io {
            path,
            source: error.error,
        });
    }
    if backup.exists() {
        fs::remove_file(&backup).map_err(|source| CliError::Io {
            path: backup,
            source,
        })?;
    }
    Ok(())
}

fn write_json<T: serde::Serialize>(value: &T) -> Result<(), CliError> {
    let mut text = serde_json::to_string_pretty(value).map_err(CliError::Json)?;
    text.push('\n');
    write_stdout(&text).map_err(CliError::Output)
}

#[derive(Debug, Error)]
pub enum CliError {
    #[error("{0}")]
    Argument(String),
    #[error("I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("output failed: {0}")]
    Output(std::io::Error),
    #[error("task contract TOML is invalid: {0}")]
    TaskToml(toml::de::Error),
    #[error("task contract TOML serialization failed: {0}")]
    TaskTomlSerialize(toml::ser::Error),
    #[error("task contract is invalid: {0}")]
    Contract(pactrail_core::ContractError),
    #[error("JSON is invalid: {0}")]
    Json(serde_json::Error),
    #[error("model configuration failed: {0}")]
    Model(ModelError),
    #[error("engine failed: {0}")]
    Engine(#[from] EngineError),
    #[error("event store failed: {0}")]
    Store(#[from] StoreError),
    #[error("workspace transaction failed: {0}")]
    Transaction(#[from] TransactionError),
    #[error("tool registry failed: {0}")]
    Tool(#[from] ToolError),
    #[error("receipt failed: {0}")]
    Receipt(#[from] pactrail_core::ReceiptError),
}

#[cfg(test)]
mod tests {
    use pactrail_core::{Evidence, EvidenceKind};

    use super::*;

    struct ReadyFixture {
        source: tempfile::TempDir,
        _control: tempfile::TempDir,
        run_root: PathBuf,
        transaction: WorkspaceTransaction,
        store: EventStore,
        receipt: ChangeReceipt,
    }

    fn ready_fixture() -> ReadyFixture {
        let source = tempfile::tempdir().unwrap_or_else(|error| unreachable!("source: {error}"));
        let control = tempfile::tempdir().unwrap_or_else(|error| unreachable!("control: {error}"));
        let run_root = control.path().join("run");
        let transaction = WorkspaceTransaction::create(source.path(), &run_root, &[".".to_owned()])
            .unwrap_or_else(|error| unreachable!("transaction: {error}"));
        transaction
            .write_file("README.md", b"# Recovered\n")
            .unwrap_or_else(|error| unreachable!("candidate write: {error}"));

        let run_id = RunId::new();
        let contract = TaskContract::new("Create a README", source.path().display().to_string());
        let evidence = vec![Evidence::deterministic_pass(
            contract.obligations[0].id,
            EvidenceKind::Test,
            "fixture passed",
        )];
        let mut store = EventStore::open_in_memory()
            .unwrap_or_else(|error| unreachable!("event store: {error}"));
        store
            .append(run_id, 0, RunEvent::ContractRegistered(contract.clone()))
            .unwrap_or_else(|error| unreachable!("contract event: {error}"));
        let transitions = [
            (RunState::Created, RunState::Contracting),
            (RunState::Contracting, RunState::Investigating),
            (RunState::Investigating, RunState::Planning),
            (RunState::Planning, RunState::Executing),
            (RunState::Executing, RunState::Verifying),
            (RunState::Verifying, RunState::Reviewing),
            (RunState::Reviewing, RunState::AwaitingApply),
        ];
        for (offset, (from, to)) in transitions.into_iter().enumerate() {
            store
                .append(
                    run_id,
                    u64::try_from(offset).unwrap_or_default() + 1,
                    RunEvent::StateChanged { from, to },
                )
                .unwrap_or_else(|error| unreachable!("state event: {error}"));
        }
        let snapshot = store
            .snapshot(run_id)
            .unwrap_or_else(|error| unreachable!("snapshot: {error}"));
        let receipt = ChangeReceipt::build(ReceiptInput {
            run_id,
            contract,
            outcome: ReceiptOutcome::ReadyToApply,
            baseline_digest: transaction.baseline_digest().to_owned(),
            final_event_hash: snapshot.last_hash.0,
            changes: transaction
                .changes()
                .unwrap_or_else(|error| unreachable!("changes: {error}")),
            evidence,
            unresolved_risks: Vec::new(),
        })
        .unwrap_or_else(|error| unreachable!("receipt: {error}"));
        ReadyFixture {
            source,
            _control: control,
            run_root,
            transaction,
            store,
            receipt,
        }
    }

    #[test]
    fn apply_repairs_receipt_after_terminal_event() {
        let mut fixture = ready_fixture();
        fixture
            .transaction
            .apply()
            .unwrap_or_else(|error| unreachable!("filesystem apply: {error}"));
        let snapshot = fixture
            .store
            .snapshot(fixture.receipt.run_id)
            .unwrap_or_else(|error| unreachable!("snapshot: {error}"));
        let terminal = fixture
            .store
            .append(
                fixture.receipt.run_id,
                snapshot.last_sequence.map_or(0, |value| value + 1),
                RunEvent::StateChanged {
                    from: RunState::AwaitingApply,
                    to: RunState::Applied,
                },
            )
            .unwrap_or_else(|error| unreachable!("applied event: {error}"));

        let repaired = apply_ready_receipt(
            &fixture.run_root,
            fixture.receipt,
            &fixture.transaction,
            &mut fixture.store,
        )
        .unwrap_or_else(|error| unreachable!("receipt recovery: {error}"));
        assert_eq!(repaired.outcome, ReceiptOutcome::Applied);
        assert_eq!(repaired.final_event_hash, terminal.hash.0);
        assert_eq!(read_receipt(&fixture.run_root).ok(), Some(repaired));
    }

    #[test]
    fn discard_repairs_receipt_and_staging_after_terminal_event() {
        let mut fixture = ready_fixture();
        let staged = fixture.run_root.join("discarded-workspace");
        fs::rename(fixture.transaction.workspace_root(), &staged)
            .unwrap_or_else(|error| unreachable!("discard staging: {error}"));
        let snapshot = fixture
            .store
            .snapshot(fixture.receipt.run_id)
            .unwrap_or_else(|error| unreachable!("snapshot: {error}"));
        let terminal = fixture
            .store
            .append(
                fixture.receipt.run_id,
                snapshot.last_sequence.map_or(0, |value| value + 1),
                RunEvent::StateChanged {
                    from: RunState::AwaitingApply,
                    to: RunState::Discarded,
                },
            )
            .unwrap_or_else(|error| unreachable!("discarded event: {error}"));

        let repaired = discard_ready_receipt(
            &fixture.run_root,
            fixture.receipt,
            &fixture.transaction,
            &mut fixture.store,
        )
        .unwrap_or_else(|error| unreachable!("discard recovery: {error}"));
        assert_eq!(repaired.outcome, ReceiptOutcome::Discarded);
        assert_eq!(repaired.final_event_hash, terminal.hash.0);
        assert!(!staged.exists());
        assert_eq!(read_receipt(&fixture.run_root).ok(), Some(repaired));
    }

    #[test]
    fn apply_rejects_candidate_changes_after_receipt() {
        let mut fixture = ready_fixture();
        fixture
            .transaction
            .write_file("README.md", b"# Changed after receipt\n")
            .unwrap_or_else(|error| unreachable!("candidate mutation: {error}"));

        let result = apply_ready_receipt(
            &fixture.run_root,
            fixture.receipt,
            &fixture.transaction,
            &mut fixture.store,
        );
        assert!(matches!(
            result,
            Err(CliError::Transaction(TransactionError::CandidateSetDrift))
        ));
        assert!(!fixture.source.path().join("README.md").exists());
    }

    #[test]
    fn native_process_opt_in_records_all_effective_capabilities() {
        let mut contract = TaskContract::new("run checks", ".");
        contract.permissions.deny.extend([
            Capability::Network,
            Capability::SecretUse,
            Capability::ExternalWrite,
        ]);
        configure_native_process_permissions(&mut contract, true)
            .unwrap_or_else(|error| unreachable!("native opt-in: {error}"));
        for capability in [
            Capability::ProcessSpawn,
            Capability::Network,
            Capability::SecretUse,
            Capability::ExternalWrite,
        ] {
            assert!(contract.permissions.allow.contains(&capability));
            assert!(!contract.permissions.deny.contains(&capability));
        }
    }

    #[test]
    fn task_file_cannot_understate_native_process_access() {
        let mut contract = TaskContract::new("run checks", ".");
        contract.permissions.allow.insert(Capability::ProcessSpawn);
        let result = configure_native_process_permissions(&mut contract, false);
        assert!(matches!(result, Err(CliError::Argument(_))));
    }
}
