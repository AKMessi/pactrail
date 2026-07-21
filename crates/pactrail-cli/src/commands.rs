use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use clap::CommandFactory;
use clap_complete::{generate, shells};
use pactrail_core::{
    ApprovalDecision, ApprovalRequest, Capability, ChangeReceipt, EventEnvelope, Evidence,
    EvidenceGrade, EvidenceId, EvidenceKind, EvidenceStatus, ReceiptInput, ReceiptOutcome,
    RunEvent, RunId, RunState, TaskContract,
};
use pactrail_engine::{CheckpointStore, EngineError, RunEngine, RunObserver, RunOutcome};
use pactrail_memory::{
    MemoryDraft, MemoryError, MemoryId, MemoryKind, MemoryMatch, MemoryRecord, MemoryStore,
};
use pactrail_models::{
    AnthropicConfig, AnthropicDriver, CapabilityProbeReport, CapabilitySource, GeminiConfig,
    GeminiDriver, ImageArtifact, MAX_INPUT_IMAGE_BYTES, ModelCapabilities, ModelDriver, ModelError,
    OpenAiCompatibleConfig, OpenAiCompatibleDriver, probe_capabilities as run_capability_probe,
    validate_image_set,
};
use pactrail_store::{EventStore, RunLease, StoreError};
use pactrail_tools::{
    ApprovalResolver, DisabledProcessBackend, NativeProcessBackend, OciProcessBackend,
    OciProcessConfig, OciRuntimeKind, OciSandboxProfile, PolicyEngine, ProcessBackend,
    RunProcessTool, ToolError, ToolRisk, builtin_registry_with_process,
};
use pactrail_workspace::{TransactionError, WorkspaceTransaction};
use schemars::schema_for;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tempfile::NamedTempFile;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::cli::{
    Cli, Command, CompletionShell, McpApprovalArg, McpCommand, MemoryCommand, MemoryKindArg,
    OciRuntimeArg, OutputFormat, ProbeArgs, ProcessApprovalArg, ProcessBackendArg, ProviderKind,
    ResumeArgs, RunArgs, RunIdArgs,
};
use crate::mcp::{McpCliError, McpRuntime};
use crate::output::{
    escape_json_terminal_controls, write_human_stdout, write_stderr, write_stdout,
};

pub async fn dispatch(cli: Cli) -> Result<(), CliError> {
    match cli.command.ok_or_else(|| {
        CliError::Argument("a command is required outside interactive mode".to_owned())
    })? {
        Command::Run(args) => run(&cli.workspace, cli.state_dir.as_deref(), args).await,
        Command::Resume(args) => resume(&cli.workspace, cli.state_dir.as_deref(), args).await,
        Command::Probe(args) => probe(args).await,
        Command::Inspect(args) => {
            let state = state_dir(&cli.workspace, cli.state_dir.as_deref())?;
            inspect(&state, &args)
        }
        Command::Trace(args) => {
            let state = state_dir(&cli.workspace, cli.state_dir.as_deref())?;
            trace(&state, &args)
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
        Command::Tools { json } => {
            let state = state_dir(&cli.workspace, cli.state_dir.as_deref())?;
            tools(&state, json)
        }
        Command::Schema => schema(),
        Command::TaskTemplate { goal } => task_template(&cli.workspace, goal),
        Command::Completion { shell } => completion(shell),
        Command::Memory { command } => {
            let state = state_dir(&cli.workspace, cli.state_dir.as_deref())?;
            memory(&state, command)
        }
        Command::Mcp { command } => {
            let state = state_dir(&cli.workspace, cli.state_dir.as_deref())?;
            execute_mcp_command(&state, &cli.workspace, command).await
        }
        Command::Doctor { json } => doctor(json),
        Command::Compatibility { json } => compatibility(json),
        Command::Migrate { apply, json } => {
            let state = state_dir(&cli.workspace, cli.state_dir.as_deref())?;
            crate::migration::execute(&state, apply, json)
        }
        Command::Upgrade { json } => {
            let state = state_dir(&cli.workspace, cli.state_dir.as_deref())?;
            crate::upgrade::execute(&state, json)
        }
    }
}

fn compatibility(json_output: bool) -> Result<(), CliError> {
    let manifest = crate::compatibility::manifest();
    if json_output {
        return write_json(&manifest);
    }
    let mut lines = vec![format!(
        "Pactrail {} compatibility contract (manifest schema {})",
        manifest.pactrail_version, manifest.manifest_schema
    )];
    for format in manifest.formats {
        let durability = if format.durable { "durable" } else { "derived" };
        lines.push(format!(
            "  {:<27} v{:<3} reads >= v{:<3} {:<18} {}",
            format.id,
            format.current_schema,
            format.minimum_readable_schema,
            format.strategy.label(),
            durability,
        ));
    }
    lines.push(
        "Unknown future schemas fail closed; no command silently downgrades state.".to_owned(),
    );
    write_human_stdout(&format!("{}\n", lines.join("\n"))).map_err(CliError::Output)
}

/// Structured result shared by scriptable and interactive frontends.
pub(crate) struct CompletedRun {
    pub run_root: PathBuf,
    pub model_summary: String,
    pub receipt: ChangeReceipt,
    pub tokens: u64,
}

async fn execute_mcp_command(
    state: &Path,
    workspace: &Path,
    command: McpCommand,
) -> Result<(), CliError> {
    let cancellation = CancellationToken::new();
    let mut execution = Box::pin(crate::mcp::execute(
        state,
        workspace,
        command,
        cancellation.clone(),
    ));
    tokio::select! {
        result = &mut execution => result.map_err(CliError::from),
        signal = tokio::signal::ctrl_c() => {
            signal.map_err(|source| CliError::Io {
                path: PathBuf::from("<ctrl-c>"),
                source,
            })?;
            cancellation.cancel();
            (&mut execution).await.map_err(CliError::from)
        }
    }
}

pub(crate) async fn probe_model_capabilities(
    args: &RunArgs,
) -> Result<CapabilityProbeReport, CliError> {
    validate_model_limits(args)?;
    let contract = TaskContract::new("Probe configured model capabilities", ".");
    let driver = build_driver(&contract, args)?;
    run_capability_probe(driver.as_ref())
        .await
        .map_err(CliError::Model)
}

async fn probe(args: ProbeArgs) -> Result<(), CliError> {
    let output = args.output;
    let report = probe_model_capabilities(&probe_run_args(args)).await?;
    if output == OutputFormat::Json {
        return write_json(&report);
    }
    write_human_stdout(&format!(
        "Capability probe\n  adapter         {}\n  model           {}\n  native tools    {}\n  parallel tools  {}\n  streaming       {}\n  prompt cache    {}\n  valid calls     {}\n  tokens          {} input · {} output\n\nNot observed is inconclusive; no returned tool was executed.\n",
        report.adapter,
        report.model,
        probe_observation_label(report.native_tools.is_observed()),
        probe_observation_label(report.parallel_tools.is_observed()),
        probe_observation_label(report.streaming.is_observed()),
        probe_observation_label(report.prompt_cache.is_observed()),
        report.valid_probe_calls,
        report.usage.input_tokens,
        report.usage.output_tokens,
    ))
    .map_err(CliError::Output)
}

fn probe_observation_label(observed: bool) -> &'static str {
    if observed { "observed" } else { "not observed" }
}

fn probe_run_args(args: ProbeArgs) -> RunArgs {
    RunArgs {
        goal: None,
        task: None,
        images: Vec::new(),
        provider: args.provider,
        model: Some(args.model),
        base_url: args.base_url,
        api_key_env: args.api_key_env,
        write_paths: vec![".".to_owned()],
        process_backend: Some(ProcessBackendArg::Disabled),
        allow_process: false,
        process_approval: Some(ProcessApprovalArg::Deny),
        mcp_approval: Some(McpApprovalArg::Deny),
        sandbox_runtime: OciRuntimeArg::Docker,
        sandbox_runtime_executable: None,
        sandbox_image: None,
        sandbox_memory_mib: 2_048,
        sandbox_cpu_millis: 2_000,
        sandbox_pids: 128,
        sandbox_tmpfs_mib: 512,
        apply: false,
        max_turns: 1,
        context_tokens: args.context_tokens,
        max_output_tokens: args.max_output_tokens,
        request_timeout_seconds: args.request_timeout_seconds,
        no_stream: args.no_stream,
        disable_thinking: args.disable_thinking,
        native_tools: args.native_tools,
        parallel_tools: args.parallel_tools,
        structured_output: args.structured_output,
        vision: args.vision,
        prompt_caching: args.prompt_caching,
        reasoning_controls: args.reasoning_controls,
        output: args.output,
    }
}

pub(crate) const RUN_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub(crate) const MIN_RUN_MANIFEST_SCHEMA_VERSION: u32 = 1;
const MAX_RUN_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_RECEIPT_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RunManifest {
    schema_version: u32,
    run_id: RunId,
    workspace_root: PathBuf,
    contract: TaskContract,
    args: RunArgs,
}

impl RunManifest {
    fn validate(
        &self,
        expected_run_id: RunId,
        transaction: &WorkspaceTransaction,
    ) -> Result<(), CliError> {
        if self.schema_version != RUN_MANIFEST_SCHEMA_VERSION {
            return Err(CliError::Argument(format!(
                "run manifest schema {} is unsupported; expected {RUN_MANIFEST_SCHEMA_VERSION}",
                self.schema_version
            )));
        }
        if self.run_id != expected_run_id {
            return Err(CliError::Argument(format!(
                "run manifest belongs to {}, not {expected_run_id}",
                self.run_id
            )));
        }
        self.contract.validate().map_err(CliError::Contract)?;
        if self.workspace_root != transaction.source_root()
            || self.contract.workspace_root != transaction.source_root().display().to_string()
        {
            return Err(CliError::Argument(
                "run manifest workspace identity does not match the isolated transaction"
                    .to_owned(),
            ));
        }
        if self.args.task.is_some()
            || !self.args.images.is_empty()
            || self.args.allow_process
            || self.args.goal.as_deref() != Some(self.contract.goal.as_str())
            || self.args.write_paths != self.contract.allowed_write_paths
            || self.args.process_backend.is_none()
            || self.args.process_approval.is_none()
        {
            return Err(CliError::Argument(
                "run manifest contains a non-normalized execution configuration".to_owned(),
            ));
        }
        validate_model_limits(&self.args)?;
        let _backend = effective_process_backend(&self.args)?;
        let _approval = effective_process_approval(&self.args)?;
        let _mcp_approval = effective_mcp_approval(&self.args);
        Ok(())
    }
}

async fn run(
    cli_workspace: &Path,
    state_override: Option<&Path>,
    args: RunArgs,
) -> Result<(), CliError> {
    if args.allow_process {
        write_stderr(
            "warning: --allow-process is deprecated and will be removed in 2.0; use --process-backend native --process-approval allow-run\n",
        )
        .map_err(CliError::Output)?;
    }
    let output = args.output;
    let cancellation = CancellationToken::new();
    let mut execution = Box::pin(execute_run_inner(
        cli_workspace,
        state_override,
        args,
        None,
        cancellation.clone(),
    ));
    let completed = tokio::select! {
        result = &mut execution => result?,
        signal = tokio::signal::ctrl_c() => {
            signal.map_err(|source| CliError::Io {
                path: PathBuf::from("<ctrl-c>"),
                source,
            })?;
            cancellation.cancel();
            (&mut execution).await?
        }
    };
    render_run(
        &completed.run_root,
        &completed.model_summary,
        &completed.receipt,
        completed.tokens,
        output,
    )
}

async fn resume(
    cli_workspace: &Path,
    state_override: Option<&Path>,
    args: ResumeArgs,
) -> Result<(), CliError> {
    let output = args.output;
    let cancellation = CancellationToken::new();
    let mut execution = Box::pin(execute_resume_inner(
        cli_workspace,
        state_override,
        args,
        None,
        cancellation.clone(),
    ));
    let completed = tokio::select! {
        result = &mut execution => result?,
        signal = tokio::signal::ctrl_c() => {
            signal.map_err(|source| CliError::Io {
                path: PathBuf::from("<ctrl-c>"),
                source,
            })?;
            cancellation.cancel();
            (&mut execution).await?
        }
    };
    render_run(
        &completed.run_root,
        &completed.model_summary,
        &completed.receipt,
        completed.tokens,
        output,
    )
}

async fn execute_resume_inner(
    cli_workspace: &Path,
    state_override: Option<&Path>,
    resume_args: ResumeArgs,
    observer: Option<&dyn RunObserver>,
    cancellation: CancellationToken,
) -> Result<CompletedRun, CliError> {
    let state = state_dir(cli_workspace, state_override)?;
    let run_id = parse_run_id(&resume_args.run_id)?;
    let run_root = run_root(&state, run_id);
    let (manifest, manifest_identity) = read_run_manifest(&run_root)?;
    let transaction = WorkspaceTransaction::open(&run_root)?;
    manifest.validate(run_id, &transaction)?;
    let RunManifest {
        contract, mut args, ..
    } = manifest;
    if let Some(process_approval) = resume_args.process_approval {
        args.process_approval = Some(process_approval);
    }
    if let Some(mcp_approval) = resume_args.mcp_approval {
        args.mcp_approval = Some(mcp_approval);
    }
    args.apply |= resume_args.apply;
    args.output = resume_args.output;
    let process_backend_kind = effective_process_backend(&args)?;
    let process_approval = effective_process_approval(&args)?;
    let mcp_approval = effective_mcp_approval(&args);
    let mcp_runtime = McpRuntime::load(&state)?;
    let process_backend =
        build_process_backend(process_backend_kind, &args, transaction.source_root()).await?;
    let runtime_identity = runtime_identity(
        &manifest_identity,
        process_backend.as_ref(),
        mcp_runtime.snapshot_digests(),
    )?;

    let mut store = EventStore::open(state.join("events.sqlite3"))?;
    let checkpoints = CheckpointStore::open(state.join("artifacts").join("checkpoints"))
        .map_err(EngineError::from)?;
    let checkpoint = checkpoints
        .load_head(&store, run_id)
        .map_err(EngineError::from)?;
    let memory = MemoryStore::open(state.join("memory.sqlite3"))?;
    let process_tool = RunProcessTool::new(process_backend, cancellation.clone());
    let mut registry = builtin_registry_with_process(process_tool)?;
    mcp_runtime.register(&mut registry, &cancellation)?;
    let policy = PolicyEngine::new(contract.permissions.clone());
    let driver = build_driver(&contract, &args)?;
    let engine = RunEngine::new(driver.as_ref(), &registry, &policy)
        .with_memory(&memory)
        .with_context_fragments(mcp_runtime.context_fragments())
        .with_repository_cache(state.join("artifacts").join("repository-index"))
        .with_checkpoint_store(&checkpoints)
        .with_runtime_identity(runtime_identity)
        .with_max_turns(args.max_turns)
        .with_cancellation(cancellation);
    let approval_resolver = ConfiguredApprovalResolver {
        process: process_approval,
        mcp: mcp_approval,
        observer,
    };
    let engine = engine.with_approval_resolver(&approval_resolver);
    let cancelled_contract = contract.clone();
    let lease = acquire_execution_lease(&mut store, &run_root, run_id, &contract)?;
    let engine_result = match observer {
        Some(observer) => {
            engine
                .resume_with_observer(
                    run_id,
                    contract,
                    &transaction,
                    &mut store,
                    checkpoint,
                    observer,
                )
                .await
        }
        None => {
            engine
                .resume(run_id, contract, &transaction, &mut store, checkpoint)
                .await
        }
    };
    release_execution_lease(&mut store, lease)?;
    let outcome = match engine_result {
        Ok(outcome) => outcome,
        Err(EngineError::Cancelled) => {
            return cancelled_run(&run_root, &transaction, &store, run_id, cancelled_contract);
        }
        Err(source) => return Err(failed_run_error(&run_root, &state, &store, run_id, source)),
    };
    finish_run(
        &run_root,
        &transaction,
        &mut store,
        &memory,
        outcome,
        args.apply,
    )
}

pub(crate) async fn execute_run_with_observer_and_cancellation(
    cli_workspace: &Path,
    state_override: Option<&Path>,
    args: RunArgs,
    observer: &dyn RunObserver,
    cancellation: CancellationToken,
) -> Result<CompletedRun, CliError> {
    execute_run_inner(
        cli_workspace,
        state_override,
        args,
        Some(observer),
        cancellation,
    )
    .await
}

pub(crate) async fn execute_resume_with_observer_and_cancellation(
    cli_workspace: &Path,
    state_override: Option<&Path>,
    run_id: RunId,
    observer: &dyn RunObserver,
    cancellation: CancellationToken,
) -> Result<CompletedRun, CliError> {
    execute_resume_inner(
        cli_workspace,
        state_override,
        ResumeArgs {
            run_id: run_id.to_string(),
            process_approval: Some(ProcessApprovalArg::Prompt),
            mcp_approval: Some(McpApprovalArg::Prompt),
            apply: false,
            output: OutputFormat::Human,
        },
        Some(observer),
        cancellation,
    )
    .await
}

async fn execute_run_inner(
    cli_workspace: &Path,
    state_override: Option<&Path>,
    args: RunArgs,
    observer: Option<&dyn RunObserver>,
    cancellation: CancellationToken,
) -> Result<CompletedRun, CliError> {
    let input_images = prepare_input_images(cli_workspace, &args)?;
    let process_backend = effective_process_backend(&args)?;
    let process_approval = effective_process_approval(&args)?;
    let mcp_approval = effective_mcp_approval(&args);
    let (contract, workspace, state, mcp_runtime) = prepare_run_contract(
        cli_workspace,
        state_override,
        &args,
        process_backend,
        mcp_approval,
    )?;
    let durable_args = normalized_run_args(
        &args,
        &contract,
        process_backend,
        process_approval,
        mcp_approval,
    );
    // Resolve and attest the execution boundary before creating durable run state. Invalid
    // sandbox configuration must fail without leaving an empty run behind for users to diagnose.
    let process_backend = build_process_backend(process_backend, &args, &workspace).await?;

    fs::create_dir_all(state.join("runs")).map_err(|source| CliError::Io {
        path: state.clone(),
        source,
    })?;
    let run_id = RunId::new();
    let run_root = state.join("runs").join(run_id.to_string());
    let transaction =
        WorkspaceTransaction::create(&workspace, &run_root, &contract.allowed_write_paths)?;
    let manifest_identity = write_run_manifest(
        &run_root,
        &RunManifest {
            schema_version: RUN_MANIFEST_SCHEMA_VERSION,
            run_id,
            workspace_root: workspace,
            contract: contract.clone(),
            args: durable_args,
        },
    )?;
    let runtime_identity = runtime_identity(
        &manifest_identity,
        process_backend.as_ref(),
        mcp_runtime.snapshot_digests(),
    )?;
    let mut store = EventStore::open(state.join("events.sqlite3"))?;
    let checkpoints = CheckpointStore::open(state.join("artifacts").join("checkpoints"))
        .map_err(EngineError::from)?;
    let memory = MemoryStore::open(state.join("memory.sqlite3"))?;
    let process_tool = RunProcessTool::new(process_backend, cancellation.clone());
    let mut registry = builtin_registry_with_process(process_tool)?;
    mcp_runtime.register(&mut registry, &cancellation)?;
    let policy = PolicyEngine::new(contract.permissions.clone());
    let driver = build_driver(&contract, &args)?;
    let mut context_fragments = memory_context_fragments(&contract, &memory)?;
    context_fragments.extend(mcp_runtime.context_fragments());
    let engine = RunEngine::new(driver.as_ref(), &registry, &policy)
        .with_memory(&memory)
        .with_context_fragments(context_fragments)
        .with_repository_cache(state.join("artifacts").join("repository-index"))
        .with_checkpoint_store(&checkpoints)
        .with_runtime_identity(runtime_identity)
        .with_input_images(input_images)
        .with_max_turns(args.max_turns)
        .with_cancellation(cancellation);
    let approval_resolver = ConfiguredApprovalResolver {
        process: process_approval,
        mcp: mcp_approval,
        observer,
    };
    let engine = engine.with_approval_resolver(&approval_resolver);
    let cancelled_contract = contract.clone();
    let lease = acquire_execution_lease(&mut store, &run_root, run_id, &contract)?;
    let engine_result = match observer {
        Some(observer) => {
            engine
                .execute_with_id_and_observer(run_id, contract, &transaction, &mut store, observer)
                .await
        }
        None => {
            engine
                .execute_with_id(run_id, contract, &transaction, &mut store)
                .await
        }
    };
    release_execution_lease(&mut store, lease)?;
    let outcome = match engine_result {
        Ok(outcome) => outcome,
        Err(EngineError::Cancelled) => {
            return cancelled_run(&run_root, &transaction, &store, run_id, cancelled_contract);
        }
        Err(source) => return Err(failed_run_error(&run_root, &state, &store, run_id, source)),
    };
    finish_run(
        &run_root,
        &transaction,
        &mut store,
        &memory,
        outcome,
        args.apply,
    )
}

fn prepare_run_contract(
    cli_workspace: &Path,
    state_override: Option<&Path>,
    args: &RunArgs,
    process_backend: ProcessBackendArg,
    mcp_approval: McpApprovalArg,
) -> Result<(TaskContract, PathBuf, PathBuf, McpRuntime), CliError> {
    let (mut contract, workspace) = load_contract(cli_workspace, args)?;
    let state = if let Some(override_path) = state_override {
        absolute_or_join(cli_workspace, override_path)?
    } else {
        workspace.join(".pactrail")
    };
    let mcp_runtime = McpRuntime::load(&state)?;
    contract.workspace_root = workspace.display().to_string();
    if args.task.is_none() {
        contract.allowed_write_paths.clone_from(&args.write_paths);
        contract.permissions.allow.insert(Capability::FileRead);
        contract.permissions.allow.insert(Capability::FileWrite);
        contract.permissions.allow.insert(Capability::MemoryRead);
        contract.permissions.deny.insert(Capability::McpInvoke);
        contract.permissions.deny.insert(Capability::Network);
        contract.permissions.deny.insert(Capability::SecretUse);
        contract.permissions.deny.insert(Capability::ExternalWrite);
    }
    configure_process_permissions(
        &mut contract,
        process_backend,
        args.task.is_some(),
        mcp_runtime
            .required_capabilities()
            .contains(&Capability::ProcessSpawn),
    )?;
    configure_mcp_permissions(
        &mut contract,
        mcp_runtime.required_capabilities(),
        mcp_approval,
        args.task.is_some(),
    )?;
    contract.validate().map_err(CliError::Contract)?;
    for required in [Capability::FileRead, Capability::FileWrite] {
        if !contract.permissions.allow.contains(&required) {
            return Err(CliError::Argument(format!(
                "task contract must explicitly allow {required}"
            )));
        }
    }
    Ok((contract, workspace, state, mcp_runtime))
}

fn normalized_run_args(
    args: &RunArgs,
    contract: &TaskContract,
    process_backend: ProcessBackendArg,
    process_approval: ProcessApprovalArg,
    mcp_approval: McpApprovalArg,
) -> RunArgs {
    let mut durable = args.clone();
    durable.goal = Some(contract.goal.clone());
    durable.task = None;
    durable
        .write_paths
        .clone_from(&contract.allowed_write_paths);
    durable.process_backend = Some(process_backend);
    durable.allow_process = false;
    durable.process_approval = Some(process_approval);
    durable.mcp_approval = Some(mcp_approval);
    durable.images.clear();
    durable
}

pub(crate) fn load_input_images(paths: &[PathBuf]) -> Result<Vec<ImageArtifact>, CliError> {
    let mut images = Vec::with_capacity(paths.len());
    for path in paths {
        let metadata = fs::symlink_metadata(path).map_err(|source| CliError::Io {
            path: path.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(CliError::Argument(format!(
                "image input {} must be a regular file, not a symlink or special file",
                path.display()
            )));
        }
        if metadata.len() == 0 || metadata.len() > MAX_INPUT_IMAGE_BYTES as u64 {
            return Err(CliError::Argument(format!(
                "image input {} has {} bytes; expected 1..={MAX_INPUT_IMAGE_BYTES}",
                path.display(),
                metadata.len()
            )));
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                CliError::Argument(format!(
                    "image input {} has no portable UTF-8 filename",
                    path.display()
                ))
            })?;
        let mut file = fs::File::open(path).map_err(|source| CliError::Io {
            path: path.clone(),
            source,
        })?;
        let opened = file.metadata().map_err(|source| CliError::Io {
            path: path.clone(),
            source,
        })?;
        if !opened.is_file() || opened.len() != metadata.len() {
            return Err(CliError::Argument(format!(
                "image input {} changed while it was being sealed",
                path.display()
            )));
        }
        let expected_len = usize::try_from(opened.len()).map_err(|_| {
            CliError::Argument(format!("image input {} is too large", path.display()))
        })?;
        let mut bytes = Vec::with_capacity(expected_len);
        Read::by_ref(&mut file)
            .take(MAX_INPUT_IMAGE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|source| CliError::Io {
                path: path.clone(),
                source,
            })?;
        if bytes.len() != expected_len {
            return Err(CliError::Argument(format!(
                "image input {} changed while it was being sealed",
                path.display()
            )));
        }
        images.push(
            ImageArtifact::from_bytes(name, &bytes)
                .map_err(|error| CliError::Argument(format!("{}: {error}", path.display())))?,
        );
    }
    validate_image_set(&images).map_err(|error| CliError::Argument(error.to_string()))?;
    Ok(images)
}

fn prepare_input_images(root: &Path, args: &RunArgs) -> Result<Vec<ImageArtifact>, CliError> {
    validate_model_limits(args)?;
    let paths = args
        .images
        .iter()
        .map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                root.join(path)
            }
        })
        .collect::<Vec<_>>();
    let images = load_input_images(&paths)?;
    if !images.is_empty() && !configured_capabilities(args).vision {
        return Err(CliError::Argument(
            "--image requires vision support; pass --vision on only when the configured model accepts image input"
                .to_owned(),
        ));
    }
    Ok(images)
}

struct ExecutionLease {
    durable: RunLease,
    _lock_file: fs::File,
}

fn acquire_execution_lease(
    store: &mut EventStore,
    run_root: &Path,
    run_id: RunId,
    contract: &TaskContract,
) -> Result<ExecutionLease, CliError> {
    const LEASE_GRACE_SECONDS: u64 = 5 * 60;
    const MAX_LEASE_SECONDS: u64 = 30 * 24 * 60 * 60;
    let ttl_seconds = contract
        .budget
        .wall_time_seconds
        .checked_add(LEASE_GRACE_SECONDS)
        .ok_or_else(|| CliError::Argument("task wall-time budget is too large".to_owned()))?;
    if ttl_seconds > MAX_LEASE_SECONDS {
        return Err(CliError::Argument(format!(
            "task wall-time budget cannot exceed {} seconds when durable resume is enabled",
            MAX_LEASE_SECONDS - LEASE_GRACE_SECONDS
        )));
    }
    let lock_path = run_root.join("execution.lock");
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|source| CliError::Io {
            path: lock_path,
            source,
        })?;
    lock_file.try_lock().map_err(|error| match error {
        fs::TryLockError::WouldBlock => CliError::Argument(format!(
            "run {run_id} is already active in another Pactrail process"
        )),
        fs::TryLockError::Error(source) => CliError::Io {
            path: run_root.join("execution.lock"),
            source,
        },
    })?;
    // The OS lock is the live-owner authority and is released by the kernel on
    // process death. A stable logical owner lets the surviving process renew
    // stale SQLite metadata immediately after acquiring that authority.
    let owner = format!("local:{run_id}");
    let durable = store
        .acquire_run_lease(run_id, &owner, Duration::from_secs(ttl_seconds))
        .map_err(CliError::Store)?;
    Ok(ExecutionLease {
        durable,
        _lock_file: lock_file,
    })
}

fn release_execution_lease(store: &mut EventStore, lease: ExecutionLease) -> Result<(), CliError> {
    let result = store
        .release_run_lease(&lease.durable)
        .map_err(CliError::Store);
    drop(lease);
    result
}

fn finish_run(
    run_root: &Path,
    transaction: &WorkspaceTransaction,
    store: &mut EventStore,
    memory: &MemoryStore,
    outcome: RunOutcome,
    apply: bool,
) -> Result<CompletedRun, CliError> {
    let mut receipt = outcome.receipt;
    write_receipt(run_root, &receipt)?;
    crate::diff::write_receipt_diff(run_root, &receipt)
        .map_err(|error| CliError::Argument(format!("review artifact failed: {error}")))?;
    if apply && receipt.outcome == ReceiptOutcome::ReadyToApply {
        receipt = apply_ready_receipt(run_root, receipt, transaction, store)?;
        memory.remember_applied_run(&receipt)?;
    }
    write_trace_artifact(run_root, store, receipt.run_id)?;
    Ok(CompletedRun {
        run_root: run_root.to_path_buf(),
        model_summary: outcome.final_text,
        receipt,
        tokens: outcome.usage.total(),
    })
}

fn memory_context_fragments(
    contract: &TaskContract,
    memory: &MemoryStore,
) -> Result<Vec<pactrail_context::ContextFragment>, CliError> {
    if !contract.permissions.allow.contains(&Capability::MemoryRead) {
        return Ok(Vec::new());
    }
    Ok(memory
        .search(&contract.goal, 8)?
        .into_iter()
        .map(|item| pactrail_context::ContextFragment {
            source: format!(
                "memory:{} [{}; {}] {}",
                item.memory.id, item.memory.kind, item.memory.source, item.memory.title
            ),
            content: item.memory.content,
        })
        .collect())
}

fn cancelled_run(
    run_root: &Path,
    transaction: &WorkspaceTransaction,
    store: &EventStore,
    run_id: RunId,
    contract: TaskContract,
) -> Result<CompletedRun, CliError> {
    let snapshot = store.snapshot(run_id)?;
    let mut evidence = snapshot.evidence;
    for obligation in &contract.obligations {
        if !evidence
            .iter()
            .any(|record| record.obligation_id == obligation.id)
        {
            evidence.push(Evidence {
                id: EvidenceId::new(),
                obligation_id: obligation.id,
                grade: EvidenceGrade::Unverified,
                kind: EvidenceKind::Other,
                status: EvidenceStatus::Inconclusive,
                summary: "Run cancelled before this obligation could be verified".to_owned(),
                artifact_digest: None,
                reproduction: None,
            });
        }
    }
    let receipt = ChangeReceipt::build(ReceiptInput {
        run_id,
        contract,
        outcome: ReceiptOutcome::Cancelled,
        baseline_digest: transaction.baseline_digest().to_owned(),
        final_event_hash: snapshot.last_hash.0,
        changes: transaction.changes()?,
        evidence,
        approvals: snapshot.approvals,
        unresolved_risks: vec![
            "Run cancelled before completion; candidate completeness and verification are inconclusive"
                .to_owned(),
        ],
    })?;
    write_receipt(run_root, &receipt)?;
    crate::diff::write_receipt_diff(run_root, &receipt)
        .map_err(|error| CliError::Argument(format!("review artifact failed: {error}")))?;
    write_trace_artifact(run_root, store, run_id)?;
    Ok(CompletedRun {
        run_root: run_root.to_path_buf(),
        model_summary:
            "Run cancelled cleanly. Any isolated candidate changes were preserved for review."
                .to_owned(),
        receipt,
        tokens: 0,
    })
}

fn failed_run_error(
    run_root: &Path,
    state: &Path,
    store: &EventStore,
    run_id: RunId,
    source: EngineError,
) -> CliError {
    let trace_path = run_root.join("trace.jsonl");
    let trace_status = match write_trace_artifact(run_root, store, run_id) {
        Ok(()) => format!("Portable trace: {}", trace_path.display()),
        Err(error) => format!(
            "Portable trace export failed ({error}); authoritative events remain in {}",
            state.join("events.sqlite3").display()
        ),
    };
    CliError::RunFailed {
        run_id,
        source: Box::new(source),
        trace_status,
    }
}

fn validate_model_limits(args: &RunArgs) -> Result<(), CliError> {
    if !(1_024..=4_194_304).contains(&args.context_tokens) {
        return Err(CliError::Argument(
            "context tokens must be between 1,024 and 4,194,304".to_owned(),
        ));
    }
    if args.max_output_tokens == 0 || args.max_output_tokens >= args.context_tokens {
        return Err(CliError::Argument(
            "maximum output tokens must be greater than zero and smaller than context tokens"
                .to_owned(),
        ));
    }
    if args.max_turns == 0 || args.max_turns > 256 {
        return Err(CliError::Argument(
            "maximum turns must be between 1 and 256".to_owned(),
        ));
    }
    if !(1..=3_600).contains(&args.request_timeout_seconds) {
        return Err(CliError::Argument(
            "request timeout must be between 1 and 3,600 seconds".to_owned(),
        ));
    }
    Ok(())
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
    let mut contract = TaskContract::new(goal, workspace.display().to_string());
    contract.budget.max_model_attempts = args.max_turns;
    contract.budget.model_tokens =
        generated_model_token_budget(args.context_tokens, args.max_output_tokens, args.max_turns);
    Ok((contract, workspace))
}

fn generated_model_token_budget(context_tokens: u64, output_tokens: u64, max_turns: u16) -> u64 {
    context_tokens
        .saturating_add(output_tokens)
        .saturating_mul(u64::from(max_turns))
}

fn build_driver(contract: &TaskContract, args: &RunArgs) -> Result<Box<dyn ModelDriver>, CliError> {
    let model = configured_model(contract, args)?;
    validate_model_options(args)?;
    let capabilities = configured_capabilities(args);
    build_driver_for_provider(model, capabilities, args)
}

fn validate_model_options(args: &RunArgs) -> Result<(), CliError> {
    if args.native_tools == crate::cli::CapabilitySetting::Off
        && args.parallel_tools == crate::cli::CapabilitySetting::On
    {
        return Err(CliError::Argument(
            "--parallel-tools on conflicts with --native-tools off".to_owned(),
        ));
    }
    if args.disable_thinking && args.reasoning_controls == crate::cli::CapabilitySetting::Off {
        return Err(CliError::Argument(
            "--disable-thinking conflicts with --reasoning-controls off".to_owned(),
        ));
    }
    if args.disable_thinking
        && matches!(
            args.provider,
            ProviderKind::Anthropic | ProviderKind::Gemini
        )
    {
        return Err(CliError::Argument(
            "--disable-thinking is an OpenAI-compatible extension and is not valid for native Anthropic or Gemini adapters"
                .to_owned(),
        ));
    }
    Ok(())
}

fn build_driver_for_provider(
    model: String,
    capabilities: ModelCapabilities,
    args: &RunArgs,
) -> Result<Box<dyn ModelDriver>, CliError> {
    let driver: Box<dyn ModelDriver> = match args.provider {
        ProviderKind::Ollama => Box::new(
            OpenAiCompatibleDriver::new(OpenAiCompatibleConfig {
                name: "ollama".to_owned(),
                base_url: args
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "http://127.0.0.1:11434/v1".to_owned()),
                model,
                api_key: None,
                timeout: Duration::from_secs(args.request_timeout_seconds),
                capabilities,
                stream: !args.no_stream,
                disable_thinking: args.disable_thinking,
            })
            .map_err(CliError::Model)?,
        ),
        ProviderKind::OpenAi => Box::new(
            OpenAiCompatibleDriver::new(OpenAiCompatibleConfig {
                name: "openai".to_owned(),
                base_url: args
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "https://api.openai.com/v1".to_owned()),
                model,
                api_key: Some(api_key_from_env(&args.api_key_env)?),
                timeout: Duration::from_secs(args.request_timeout_seconds),
                capabilities,
                stream: !args.no_stream,
                disable_thinking: args.disable_thinking,
            })
            .map_err(CliError::Model)?,
        ),
        ProviderKind::OpenAiCompatible => Box::new(
            OpenAiCompatibleDriver::new(OpenAiCompatibleConfig {
                name: "openai-compatible".to_owned(),
                base_url: args.base_url.clone().ok_or_else(|| {
                    CliError::Argument("--base-url is required for open-ai-compatible".to_owned())
                })?,
                model,
                api_key: std::env::var(&args.api_key_env)
                    .ok()
                    .filter(|api_key| !api_key.is_empty())
                    .map(SecretString::from),
                timeout: Duration::from_secs(args.request_timeout_seconds),
                capabilities,
                stream: !args.no_stream,
                disable_thinking: args.disable_thinking,
            })
            .map_err(CliError::Model)?,
        ),
        ProviderKind::Anthropic => Box::new(
            AnthropicDriver::new(AnthropicConfig {
                name: "anthropic".to_owned(),
                base_url: args
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "https://api.anthropic.com".to_owned()),
                model,
                api_key: api_key_from_env(provider_key_env(args.provider, &args.api_key_env))?,
                timeout: Duration::from_secs(args.request_timeout_seconds),
                capabilities,
                stream: !args.no_stream,
            })
            .map_err(CliError::Model)?,
        ),
        ProviderKind::Gemini => Box::new(
            GeminiDriver::new(GeminiConfig {
                name: "gemini".to_owned(),
                base_url: args
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "https://generativelanguage.googleapis.com".to_owned()),
                model,
                api_key: api_key_from_env(provider_key_env(args.provider, &args.api_key_env))?,
                timeout: Duration::from_secs(args.request_timeout_seconds),
                capabilities,
                stream: !args.no_stream,
            })
            .map_err(CliError::Model)?,
        ),
    };
    Ok(driver)
}

fn configured_model(contract: &TaskContract, args: &RunArgs) -> Result<String, CliError> {
    args.model
        .clone()
        .or_else(|| contract.model.clone())
        .ok_or_else(|| {
            CliError::Argument("a model is required; pass --model or set PACTRAIL_MODEL".to_owned())
        })
}

fn configured_capabilities(args: &RunArgs) -> ModelCapabilities {
    let native_tools = args.native_tools.resolve(true);
    let provider_parallel = matches!(
        args.provider,
        ProviderKind::Anthropic | ProviderKind::Gemini
    );
    ModelCapabilities {
        native_tools,
        parallel_tools: native_tools && args.parallel_tools.resolve(provider_parallel),
        structured_output: args.structured_output.resolve(false),
        vision: args.vision.resolve(false),
        prompt_caching: args.prompt_caching.resolve(false),
        streaming: !args.no_stream,
        reasoning_controls: args.reasoning_controls.resolve(args.disable_thinking),
        context_tokens: args.context_tokens,
        max_output_tokens: args.max_output_tokens,
        source: CapabilitySource::UserDeclared,
    }
}

fn provider_key_env(provider: ProviderKind, configured: &str) -> &str {
    match (provider, configured) {
        (ProviderKind::Anthropic, "OPENAI_API_KEY") => "ANTHROPIC_API_KEY",
        (ProviderKind::Gemini, "OPENAI_API_KEY") => "GEMINI_API_KEY",
        _ => configured,
    }
}

fn api_key_from_env(name: &str) -> Result<SecretString, CliError> {
    std::env::var(name)
        .ok()
        .filter(|api_key| !api_key.is_empty())
        .map(SecretString::from)
        .ok_or_else(|| {
            CliError::Argument(format!(
                "required API key environment variable {name:?} is not set or is empty"
            ))
        })
}

fn effective_process_backend(args: &RunArgs) -> Result<ProcessBackendArg, CliError> {
    match (args.process_backend, args.allow_process) {
        (Some(ProcessBackendArg::Native) | None, true) => Ok(ProcessBackendArg::Native),
        (Some(mode), true) => Err(CliError::Argument(format!(
            "--allow-process is a deprecated alias for --process-backend native and conflicts with --process-backend {mode:?}"
        ))),
        (Some(mode), false) => Ok(mode),
        (None, false) => Ok(ProcessBackendArg::Disabled),
    }
}

fn effective_process_approval(args: &RunArgs) -> Result<ProcessApprovalArg, CliError> {
    match (args.process_approval, args.allow_process) {
        (Some(ProcessApprovalArg::AllowRun) | None, true) => Ok(ProcessApprovalArg::AllowRun),
        (Some(mode), true) => Err(CliError::Argument(format!(
            "--allow-process includes run-scoped approval and conflicts with --process-approval {mode:?}"
        ))),
        (Some(mode), false) => Ok(mode),
        (None, false) => Ok(ProcessApprovalArg::Deny),
    }
}

fn effective_mcp_approval(args: &RunArgs) -> McpApprovalArg {
    args.mcp_approval.unwrap_or(McpApprovalArg::Deny)
}

struct ConfiguredApprovalResolver<'a> {
    process: ProcessApprovalArg,
    mcp: McpApprovalArg,
    observer: Option<&'a dyn RunObserver>,
}

impl ApprovalResolver for ConfiguredApprovalResolver<'_> {
    fn resolve(&self, request: &ApprovalRequest) -> ApprovalDecision {
        let decision = if request.binding.backend_kind.starts_with("mcp_") {
            match self.mcp {
                McpApprovalArg::Deny => Some(ApprovalDecision::Deny),
                McpApprovalArg::AllowRun => Some(ApprovalDecision::AllowRun),
                McpApprovalArg::Prompt => None,
            }
        } else {
            match self.process {
                ProcessApprovalArg::Deny => Some(ApprovalDecision::Deny),
                ProcessApprovalArg::AllowRun => Some(ApprovalDecision::AllowRun),
                ProcessApprovalArg::Prompt => None,
            }
        };
        decision.unwrap_or_else(|| {
            self.observer.map_or(ApprovalDecision::Deny, |observer| {
                observer.on_approval_request(request)
            })
        })
    }
}

fn configure_process_permissions(
    contract: &mut TaskContract,
    backend: ProcessBackendArg,
    authoritative_permissions: bool,
    mcp_requires_process: bool,
) -> Result<(), CliError> {
    let effective = [
        Capability::ProcessSpawn,
        Capability::Network,
        Capability::SecretUse,
        Capability::ExternalWrite,
    ];
    match backend {
        ProcessBackendArg::Disabled => {
            if !mcp_requires_process
                && (contract
                    .permissions
                    .allow
                    .contains(&Capability::ProcessSpawn)
                    || contract.permissions.ask.contains(&Capability::ProcessSpawn))
            {
                return Err(CliError::Argument(
                    "the task contract permits process_spawn but the selected process backend is disabled; select --process-backend native or oci explicitly"
                        .to_owned(),
                ));
            }
            if !mcp_requires_process {
                contract.permissions.deny.insert(Capability::ProcessSpawn);
            }
        }
        ProcessBackendArg::Oci => {
            if contract
                .permissions
                .deny
                .contains(&Capability::ProcessSpawn)
            {
                return Err(CliError::Argument(
                    "the task contract explicitly denies process_spawn; an OCI backend cannot override that denial"
                        .to_owned(),
                ));
            }
            contract.permissions.deny.remove(&Capability::ProcessSpawn);
            if contract
                .permissions
                .allow
                .contains(&Capability::ProcessSpawn)
            {
                contract.permissions.ask.remove(&Capability::ProcessSpawn);
            } else {
                contract.permissions.ask.insert(Capability::ProcessSpawn);
            }
            for capability in [
                Capability::Network,
                Capability::SecretUse,
                Capability::ExternalWrite,
            ] {
                contract.permissions.allow.remove(&capability);
                contract.permissions.ask.remove(&capability);
                contract.permissions.deny.insert(capability);
            }
        }
        ProcessBackendArg::Native => {
            if authoritative_permissions
                && let Some(capability) = effective
                    .iter()
                    .find(|capability| contract.permissions.deny.contains(*capability))
            {
                return Err(CliError::Argument(format!(
                    "native execution retains {capability} authority and cannot override the task contract's explicit denial"
                )));
            }
            for capability in &effective {
                contract.permissions.deny.remove(capability);
                if capability == &Capability::ProcessSpawn {
                    if contract.permissions.allow.contains(capability) {
                        contract.permissions.ask.remove(capability);
                    } else {
                        contract.permissions.ask.insert(capability.clone());
                    }
                } else {
                    contract.permissions.ask.remove(capability);
                    contract.permissions.allow.insert(capability.clone());
                }
            }
        }
    }
    validate_native_authority(contract, backend, &effective)?;
    Ok(())
}

fn configure_mcp_permissions(
    contract: &mut TaskContract,
    required: &std::collections::BTreeSet<Capability>,
    approval: McpApprovalArg,
    authoritative_permissions: bool,
) -> Result<(), CliError> {
    if required.is_empty() {
        if !authoritative_permissions {
            contract.permissions.deny.insert(Capability::McpInvoke);
        }
        return Ok(());
    }
    if authoritative_permissions {
        let missing = required
            .iter()
            .filter(|capability| {
                !contract.permissions.allow.contains(*capability)
                    && !contract.permissions.ask.contains(*capability)
            })
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(CliError::Argument(format!(
                "enabled MCP snapshots require task-contract authority for: {}",
                missing.join(", ")
            )));
        }
        return Ok(());
    }

    match approval {
        McpApprovalArg::Deny => {
            contract.permissions.allow.remove(&Capability::McpInvoke);
            contract.permissions.ask.remove(&Capability::McpInvoke);
            contract.permissions.deny.insert(Capability::McpInvoke);
        }
        McpApprovalArg::AllowRun | McpApprovalArg::Prompt => {
            for capability in required {
                contract.permissions.deny.remove(capability);
                if !contract.permissions.allow.contains(capability) {
                    contract.permissions.ask.insert(capability.clone());
                }
            }
            // MCP invocation always remains request-scoped, even for allow-run. The
            // resolver records each exact snapshot/tool/argument-bound grant.
            contract.permissions.allow.remove(&Capability::McpInvoke);
            contract.permissions.ask.insert(Capability::McpInvoke);
        }
    }
    Ok(())
}

fn validate_native_authority(
    contract: &TaskContract,
    backend: ProcessBackendArg,
    effective: &[Capability],
) -> Result<(), CliError> {
    let process_permitted = contract.permissions.ask.contains(&Capability::ProcessSpawn)
        || contract
            .permissions
            .allow
            .contains(&Capability::ProcessSpawn);
    if backend != ProcessBackendArg::Native || !process_permitted {
        return Ok(());
    }
    let missing = effective
        .iter()
        .filter(|capability| {
            !(contract.permissions.allow.contains(*capability)
                || *capability == &Capability::ProcessSpawn
                    && contract.permissions.ask.contains(*capability))
        })
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(CliError::Argument(format!(
            "native process execution is unsandboxed; its contract must also allow: {}",
            missing.join(", ")
        )))
    }
}

async fn build_process_backend(
    backend: ProcessBackendArg,
    args: &RunArgs,
    source_workspace: &Path,
) -> Result<Arc<dyn ProcessBackend>, CliError> {
    let backend: Arc<dyn ProcessBackend> = match backend {
        ProcessBackendArg::Disabled => Arc::new(DisabledProcessBackend),
        ProcessBackendArg::Native => Arc::new(NativeProcessBackend),
        ProcessBackendArg::Oci => {
            let image = args.sandbox_image.clone().ok_or_else(|| {
                CliError::Argument(
                    "--process-backend oci requires --sandbox-image <local-image>".to_owned(),
                )
            })?;
            let runtime = match args.sandbox_runtime {
                OciRuntimeArg::Docker => OciRuntimeKind::Docker,
                OciRuntimeArg::Podman => OciRuntimeKind::Podman,
            };
            let mut config = OciProcessConfig::for_runtime(runtime, image);
            if let Some(executable) = &args.sandbox_runtime_executable {
                config.runtime_executable = executable.as_os_str().to_owned();
            }
            let default_profile = OciSandboxProfile::default();
            config.profile = OciSandboxProfile {
                memory_bytes: mebibytes(args.sandbox_memory_mib, "sandbox memory")?,
                milli_cpus: args.sandbox_cpu_millis,
                pids_limit: args.sandbox_pids,
                tmpfs_bytes: mebibytes(args.sandbox_tmpfs_mib, "sandbox temporary space")?,
                user: default_profile.user,
            };
            Arc::new(
                OciProcessBackend::initialize(config, &[source_workspace.to_path_buf()])
                    .await
                    .map_err(|error| CliError::Tool(ToolError::ProcessBackend(error)))?,
            )
        }
    };
    Ok(backend)
}

fn mebibytes(value: u64, label: &str) -> Result<u64, CliError> {
    value
        .checked_mul(1024 * 1024)
        .ok_or_else(|| CliError::Argument(format!("{label} is too large to represent in bytes")))
}

pub(crate) fn inspect(state: &Path, args: &RunIdArgs) -> Result<(), CliError> {
    let run_id = parse_run_id(&args.run_id)?;
    let run_root = run_root(state, run_id);
    let receipt_path = run_root.join("receipt.json");
    if optional_regular_file(&receipt_path, "receipt")? {
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
        return write_human_stdout(&text).map_err(CliError::Output);
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
        write_human_stdout(&format!(
            "Run: {run_id}\nState: {:?}\nReceipt: not available\n",
            snapshot.state
        ))
        .map_err(CliError::Output)
    }
}

pub(crate) fn trace(state: &Path, args: &RunIdArgs) -> Result<(), CliError> {
    let run_id = parse_run_id(&args.run_id)?;
    let events = load_trace(state, run_id)?;
    if args.json {
        return write_json(&events);
    }
    write_human_stdout(&render_trace_text(run_id, &events)).map_err(CliError::Output)
}

pub(crate) fn load_trace(state: &Path, run_id: RunId) -> Result<Vec<EventEnvelope>, CliError> {
    EventStore::open(state.join("events.sqlite3"))?
        .load(run_id)
        .map_err(CliError::Store)
}

fn render_trace_text(run_id: RunId, events: &[EventEnvelope]) -> String {
    if events.is_empty() {
        return format!("Trace {run_id}\nNo durable events found.\n");
    }
    let started = events[0].timestamp;
    let mut lines = vec![format!(
        "Trace {run_id}  ({} events, hash chain verified)",
        events.len()
    )];
    for envelope in events {
        let elapsed = envelope.timestamp - started;
        let elapsed_ms = elapsed.whole_milliseconds().max(0);
        let prefix = format!(
            "{:>8}  #{:<3}",
            format_trace_elapsed(elapsed_ms),
            envelope.sequence
        );
        match &envelope.event {
            RunEvent::ContractRegistered(contract) => {
                lines.push(format!("{prefix}  CONTRACT  {}", contract.goal));
            }
            RunEvent::StateChanged { from, to } => {
                lines.push(format!("{prefix}  STATE     {from:?} -> {to:?}"));
            }
            RunEvent::ActionCompleted(action) => {
                lines.push(format!(
                    "{prefix}  {:<9} {:>7}  {}  {}",
                    trace_actor(&action.actor),
                    format_trace_elapsed(i128::from(action.duration_ms)),
                    if action.succeeded { "OK" } else { "FAIL" },
                    action.summary
                ));
                if !action.attributes.is_empty() {
                    lines.push(format!(
                        "                    {}",
                        action
                            .attributes
                            .iter()
                            .map(|(key, value)| format!("{key}={value}"))
                            .collect::<Vec<_>>()
                            .join("  ")
                    ));
                }
                if !action.observed_effects.is_empty() {
                    lines.push(format!(
                        "                    effects: {}",
                        action.observed_effects.join(", ")
                    ));
                }
            }
            RunEvent::EvidenceRecorded(evidence) => lines.push(format!(
                "{prefix}  EVIDENCE  {:?}/{:?}  {}",
                evidence.grade, evidence.status, evidence.summary
            )),
            RunEvent::PolicyEvaluated(decision) => {
                lines.push(format!("{prefix}  POLICY    {decision:?}"));
            }
            RunEvent::ApprovalDecided(approval) => {
                lines.push(format!(
                    "{prefix}  APPROVAL  {:?}  {} via {}",
                    approval.decision, approval.binding.capability, approval.binding.backend_kind
                ));
                lines.push(format!(
                    "                    actor={}  profile={}  resource={}",
                    short_digest(&approval.binding.actor_fingerprint),
                    short_digest(&approval.binding.profile_digest),
                    approval.binding.resource
                ));
            }
            RunEvent::EffectPrepared(effect) => {
                lines.push(format!(
                    "{prefix}  PREPARE   {}  {}  risk={}",
                    effect.tool, effect.call_id, effect.risk
                ));
                lines.push(format!(
                    "                    args={}  candidate={}  runtime={}",
                    short_digest(&effect.arguments_digest),
                    short_digest(&effect.candidate_digest_before),
                    short_digest(&effect.runtime_profile_digest)
                ));
            }
            RunEvent::EffectCompleted(effect) => {
                lines.push(format!(
                    "{prefix}  EFFECT    {}  {}  result={}  candidate={}",
                    if effect.succeeded { "OK" } else { "FAIL" },
                    effect.call_id,
                    short_digest(&effect.result_digest),
                    short_digest(&effect.candidate_digest_after)
                ));
            }
            RunEvent::CheckpointCreated { checkpoint } => {
                lines.push(format!("{prefix}  CHECKPT   {checkpoint}"));
            }
            RunEvent::NoteRecorded { message } => {
                lines.push(format!("{prefix}  NOTE      {message}"));
            }
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

fn trace_actor(actor: &str) -> &'static str {
    if actor.starts_with("model:") {
        "MODEL"
    } else if actor.starts_with("tool:") {
        "TOOL"
    } else if actor == "verifier" {
        "VERIFY"
    } else {
        "ACTION"
    }
}

fn format_trace_elapsed(milliseconds: i128) -> String {
    if milliseconds < 1_000 {
        format!("{milliseconds}ms")
    } else {
        format!(
            "{}.{:02}s",
            milliseconds / 1_000,
            (milliseconds % 1_000) / 10
        )
    }
}

fn short_digest(value: &str) -> &str {
    value.get(..12).unwrap_or(value)
}

pub(crate) fn apply(state: &Path, args: &RunIdArgs) -> Result<(), CliError> {
    let run_id = parse_run_id(&args.run_id)?;
    let receipt = apply_run(state, run_id)?;
    if args.json {
        write_json(&receipt)
    } else {
        write_human_stdout(&format!(
            "Applied run {} to {} ({} files).\n",
            run_id,
            receipt.contract.workspace_root,
            receipt.changes.len()
        ))
        .map_err(CliError::Output)
    }
}

pub(crate) fn apply_run(state: &Path, run_id: RunId) -> Result<ChangeReceipt, CliError> {
    let run_root = run_root(state, run_id);
    let receipt = read_receipt(&run_root)?;
    let transaction = WorkspaceTransaction::open(&run_root)?;
    let mut store = EventStore::open(state.join("events.sqlite3"))?;
    let applied = apply_ready_receipt(&run_root, receipt, &transaction, &mut store)?;
    write_trace_artifact(&run_root, &store, run_id)?;
    MemoryStore::open(state.join("memory.sqlite3"))?.remember_applied_run(&applied)?;
    Ok(applied)
}

fn apply_ready_receipt(
    run_root: &Path,
    receipt: ChangeReceipt,
    transaction: &WorkspaceTransaction,
    store: &mut EventStore,
) -> Result<ChangeReceipt, CliError> {
    require_receipt_integrity(&receipt)?;
    if receipt.baseline_digest != transaction.baseline_digest()
        || receipt.contract.workspace_root != transaction.source_root().display().to_string()
    {
        return Err(CliError::Argument(
            "receipt is not bound to the isolated transaction baseline and source workspace"
                .to_owned(),
        ));
    }
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

pub(crate) fn discard(state: &Path, args: &RunIdArgs) -> Result<(), CliError> {
    let run_id = parse_run_id(&args.run_id)?;
    let discarded = discard_run(state, run_id)?;
    if args.json {
        write_json(&discarded)
    } else {
        write_human_stdout(&format!("Discarded run {run_id}; receipt preserved.\n"))
            .map_err(CliError::Output)
    }
}

pub(crate) fn discard_run(state: &Path, run_id: RunId) -> Result<ChangeReceipt, CliError> {
    let run_root = run_root(state, run_id);
    let receipt = read_receipt(&run_root)?;
    let mut store = EventStore::open(state.join("events.sqlite3"))?;
    if let Some(discarded) = recover_completed_discard(&run_root, receipt.clone(), &store)? {
        write_trace_artifact(&run_root, &store, run_id)?;
        return Ok(discarded);
    }
    let transaction = WorkspaceTransaction::open(&run_root)?;
    let discarded = discard_ready_receipt(&run_root, receipt, &transaction, &mut store)?;
    write_trace_artifact(&run_root, &store, run_id)?;
    Ok(discarded)
}

fn recover_completed_discard(
    run_root: &Path,
    receipt: ChangeReceipt,
    store: &EventStore,
) -> Result<Option<ChangeReceipt>, CliError> {
    require_receipt_integrity(&receipt)?;
    let snapshot = store.snapshot(receipt.run_id)?;
    if snapshot.state != RunState::Discarded {
        return Ok(None);
    }
    let workspace = run_root.join("workspace");
    if workspace.exists() {
        return Err(CliError::Argument(format!(
            "discarded run still has a transaction workspace at {}",
            workspace.display()
        )));
    }
    remove_staged_workspace(&run_root.join("discarded-workspace"))?;
    match receipt.outcome {
        ReceiptOutcome::Discarded => {
            if receipt.final_event_hash != snapshot.last_hash.0 {
                return Err(CliError::Argument(
                    "discarded receipt does not match the durable event head".to_owned(),
                ));
            }
            Ok(Some(receipt))
        }
        ReceiptOutcome::ReadyToApply => {
            let discarded =
                rebuild_receipt(receipt, ReceiptOutcome::Discarded, snapshot.last_hash.0)?;
            write_receipt(run_root, &discarded)?;
            Ok(Some(discarded))
        }
        outcome => Err(state_receipt_mismatch(
            RunState::Discarded,
            outcome,
            "discard",
        )),
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
        approvals: receipt.approvals,
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

pub(crate) fn list(state: &Path, json_output: bool) -> Result<(), CliError> {
    let runs = run_history(state)?;
    let values = runs
        .iter()
        .map(|run| {
            json!({
                "run_id": run.run_id,
                "state": run.state,
                "outcome": run.outcome,
                "goal": run.goal,
                "changes": run.changes,
            })
        })
        .collect::<Vec<_>>();
    if json_output {
        write_json(&values)
    } else if values.is_empty() {
        write_human_stdout("No durable Pactrail runs found.\n").map_err(CliError::Output)
    } else {
        let text = values
            .iter()
            .map(|value| {
                let status = value["outcome"]
                    .as_str()
                    .or_else(|| value["state"].as_str())
                    .unwrap_or_default();
                format!(
                    "{}  {:<16}  {}",
                    value["run_id"].as_str().unwrap_or_default(),
                    status,
                    value["goal"].as_str().unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        write_human_stdout(&format!("{text}\n")).map_err(CliError::Output)
    }
}

pub(crate) struct RunHistoryEntry {
    pub run_id: RunId,
    pub state: RunState,
    pub outcome: Option<ReceiptOutcome>,
    pub goal: String,
    pub changes: usize,
}

pub(crate) fn run_history(state_root: &Path) -> Result<Vec<RunHistoryEntry>, CliError> {
    let database = state_root.join("events.sqlite3");
    if !database.is_file() {
        return Ok(Vec::new());
    }
    let store = EventStore::open(database)?;
    let mut history = Vec::new();
    for run_id in store.list_run_ids()? {
        let events = store.load(run_id)?;
        let mut durable_state = RunState::Created;
        let mut goal = "(task contract unavailable)".to_owned();
        for envelope in events {
            match envelope.event {
                RunEvent::ContractRegistered(contract) => goal = contract.goal,
                RunEvent::StateChanged { to, .. } => durable_state = to,
                _ => {}
            }
        }
        let root = run_root(state_root, run_id);
        let receipt_path = root.join("receipt.json");
        let receipt = if optional_regular_file(&receipt_path, "receipt")? {
            Some(read_receipt(&root)?)
        } else {
            None
        };
        history.push(RunHistoryEntry {
            run_id,
            state: durable_state,
            outcome: receipt.as_ref().map(|receipt| receipt.outcome),
            goal,
            changes: receipt.as_ref().map_or(0, |receipt| receipt.changes.len()),
        });
    }
    Ok(history)
}

pub(crate) fn completed_runs(state: &Path) -> Result<Vec<ChangeReceipt>, CliError> {
    let runs = state.join("runs");
    let metadata = match fs::symlink_metadata(&runs) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(CliError::Io { path: runs, source }),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CliError::Argument(format!(
            "run directory {} is not a real local directory",
            runs.display()
        )));
    }
    let mut receipts = Vec::new();
    for entry in fs::read_dir(&runs).map_err(|source| CliError::Io {
        path: runs.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| CliError::Io {
            path: runs.clone(),
            source,
        })?;
        let file_type = entry.file_type().map_err(|source| CliError::Io {
            path: entry.path(),
            source,
        })?;
        if file_type.is_symlink() || !file_type.is_dir() {
            return Err(CliError::Argument(format!(
                "unexpected non-directory entry in {}",
                runs.display()
            )));
        }
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            CliError::Argument("run directory name is not valid UTF-8".to_owned())
        })?;
        let _run_id = parse_run_id(name)?;
        let receipt_path = entry.path().join("receipt.json");
        if optional_regular_file(&receipt_path, "receipt")? {
            receipts.push(read_receipt(entry.path().as_path())?);
        }
    }
    receipts.sort_by_key(|receipt| receipt.run_id.to_string());
    Ok(receipts)
}

/// Validates run manifests and transaction metadata against the event journal
/// without invoking backup recovery or mutating state.
pub(crate) fn validate_run_artifacts(state: &Path, store: &EventStore) -> Result<usize, CliError> {
    let run_ids = store.list_run_ids()?;
    let expected = run_ids
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let runs = state.join("runs");
    let metadata = match fs::symlink_metadata(&runs) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound && expected.is_empty() => {
            return Ok(0);
        }
        Err(source) => {
            return Err(CliError::Io {
                path: runs.clone(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CliError::Argument(format!(
            "run directory {} is not a real local directory",
            runs.display()
        )));
    }

    let mut discovered = std::collections::BTreeSet::new();
    for entry in fs::read_dir(&runs).map_err(|source| CliError::Io {
        path: runs.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| CliError::Io {
            path: runs.clone(),
            source,
        })?;
        let file_type = entry.file_type().map_err(|source| CliError::Io {
            path: entry.path(),
            source,
        })?;
        if file_type.is_symlink() || !file_type.is_dir() {
            return Err(CliError::Argument(format!(
                "unexpected non-directory entry in {}",
                runs.display()
            )));
        }
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            CliError::Argument("run directory name is not valid UTF-8".to_owned())
        })?;
        let run_id = parse_run_id(name)?;
        if !expected.contains(&run_id) {
            return Err(CliError::Argument(format!(
                "run directory {run_id} has no authoritative event journal"
            )));
        }
        discovered.insert(run_id);
    }
    if discovered != expected {
        let missing = expected
            .difference(&discovered)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        return Err(CliError::Argument(format!(
            "event journal run(s) are missing local transaction state: {}",
            missing.join(", ")
        )));
    }

    for run_id in &run_ids {
        let root = run_root(state, *run_id);
        let transaction = WorkspaceTransaction::open(&root)?;
        let (manifest, _digest) = read_run_manifest_exact(&root)?;
        manifest.validate(*run_id, &transaction)?;
        let snapshot = store.snapshot(*run_id)?;
        if snapshot.contract.as_ref() != Some(&manifest.contract) {
            return Err(CliError::Argument(format!(
                "run {run_id} manifest contract does not match its event journal"
            )));
        }
        let receipt_path = root.join("receipt.json");
        if optional_regular_file(&receipt_path, "receipt")? {
            let receipt = read_receipt_exact(&root)?;
            if receipt.run_id != *run_id
                || !receipt.verify_integrity()?
                || receipt.baseline_digest != transaction.baseline_digest()
                || receipt.contract.workspace_root
                    != transaction.source_root().display().to_string()
            {
                return Err(CliError::Argument(format!(
                    "run {run_id} receipt identity, integrity, or transaction binding is invalid"
                )));
            }
        }
    }
    Ok(run_ids.len())
}

fn tools(state: &Path, json_output: bool) -> Result<(), CliError> {
    let cancellation = CancellationToken::new();
    let mut registry = builtin_registry_with_process(RunProcessTool::disabled())?;
    McpRuntime::load(state)?.register(&mut registry, &cancellation)?;
    let descriptors = registry.descriptors();
    if json_output {
        write_json(&descriptors)
    } else {
        let mut lines = vec![format!(
            "Tool kernel · {} typed contracts",
            descriptors.len()
        )];
        lines.push(format!(
            "{:<20} {:<8} {:<14} {}",
            "NAME", "RISK", "CAPABILITY", "BEHAVIOR"
        ));
        for tool in &descriptors {
            let risk = match tool.annotations.risk {
                ToolRisk::ReadOnly => "read",
                ToolRisk::WorkspaceMutation => "edit",
                ToolRisk::RestrictedExecution => "sandbox",
                ToolRisk::HostExecution => "host",
            };
            let annotations = [
                tool.annotations.read_only.then_some("read-only"),
                tool.annotations.idempotent.then_some("idempotent"),
                tool.annotations.parallel_safe.then_some("parallel-safe"),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ");
            lines.push(format!(
                "{:<20} {risk:<8} {:<14} {}",
                tool.name,
                tool.required_capability.to_string(),
                if annotations.is_empty() {
                    "serial side effect"
                } else {
                    &annotations
                }
            ));
            lines.push(format!("  {}", tool.description));
        }
        write_human_stdout(&format!("{}\n", lines.join("\n"))).map_err(CliError::Output)
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
    contract.permissions.allow.insert(Capability::MemoryRead);
    contract.permissions.deny.insert(Capability::McpInvoke);
    contract.permissions.deny.insert(Capability::Network);
    contract.permissions.deny.insert(Capability::SecretUse);
    contract.permissions.deny.insert(Capability::ExternalWrite);
    let mut text = toml::to_string_pretty(&contract).map_err(CliError::TaskTomlSerialize)?;
    text.push('\n');
    write_stdout(&text).map_err(CliError::Output)
}

fn completion(shell: CompletionShell) -> Result<(), CliError> {
    let mut command = Cli::command();
    let mut output = Vec::new();
    match shell {
        CompletionShell::Bash => generate(shells::Bash, &mut command, "pactrail", &mut output),
        CompletionShell::Elvish => {
            generate(shells::Elvish, &mut command, "pactrail", &mut output);
        }
        CompletionShell::Fish => generate(shells::Fish, &mut command, "pactrail", &mut output),
        CompletionShell::PowerShell => {
            generate(shells::PowerShell, &mut command, "pactrail", &mut output);
        }
        CompletionShell::Zsh => generate(shells::Zsh, &mut command, "pactrail", &mut output),
    }
    let output = String::from_utf8(output)
        .map_err(|error| CliError::Argument(format!("completion output was not UTF-8: {error}")))?;
    write_stdout(&output).map_err(CliError::Output)
}

fn memory(state: &Path, command: MemoryCommand) -> Result<(), CliError> {
    match command {
        MemoryCommand::List { limit, json } => {
            let memories = list_memories(state, usize::from(limit))?;
            render_memories(&memories, json)
        }
        MemoryCommand::Search { query, limit, json } => {
            let matches = search_memories(state, &query, usize::from(limit))?;
            if json {
                write_json(&matches)
            } else {
                let memories = matches
                    .into_iter()
                    .map(|item| item.memory)
                    .collect::<Vec<_>>();
                render_memories(&memories, false)
            }
        }
        MemoryCommand::Add {
            content,
            title,
            kind,
            tags,
            json,
        } => {
            let title = title.unwrap_or_else(|| default_memory_title(&content));
            let memory = remember_memory(
                state,
                MemoryDraft {
                    kind: memory_kind(kind),
                    title,
                    content,
                    tags,
                },
            )?;
            if json {
                write_json(&memory)
            } else {
                write_human_stdout(&format!(
                    "Remembered {} [{}] {}\n",
                    memory.id, memory.kind, memory.title
                ))
                .map_err(CliError::Output)
            }
        }
        MemoryCommand::Forget { id, json } => {
            let id = resolve_memory_id(state, &id)?;
            forget_memory(state, id)?;
            if json {
                write_json(&json!({ "forgotten": id }))
            } else {
                write_human_stdout(&format!("Forgot memory {id}.\n")).map_err(CliError::Output)
            }
        }
    }
}

pub(crate) fn list_memories(state: &Path, limit: usize) -> Result<Vec<MemoryRecord>, CliError> {
    MemoryStore::open(state.join("memory.sqlite3"))?
        .list(limit)
        .map_err(CliError::Memory)
}

pub(crate) fn search_memories(
    state: &Path,
    query: &str,
    limit: usize,
) -> Result<Vec<MemoryMatch>, CliError> {
    MemoryStore::open(state.join("memory.sqlite3"))?
        .search(query, limit)
        .map_err(CliError::Memory)
}

pub(crate) fn remember_memory(state: &Path, draft: MemoryDraft) -> Result<MemoryRecord, CliError> {
    MemoryStore::open(state.join("memory.sqlite3"))?
        .remember(draft)
        .map_err(CliError::Memory)
}

pub(crate) fn forget_memory(state: &Path, id: MemoryId) -> Result<(), CliError> {
    MemoryStore::open(state.join("memory.sqlite3"))?
        .forget(id)
        .map_err(CliError::Memory)
}

pub(crate) fn resolve_memory_id(state: &Path, value: &str) -> Result<MemoryId, CliError> {
    if let Ok(id) = MemoryId::from_str(value) {
        return Ok(id);
    }
    let canonical_prefix = value.to_ascii_lowercase();
    let separators_are_canonical = canonical_prefix.bytes().enumerate().all(|(index, byte)| {
        byte.is_ascii_hexdigit() || (byte == b'-' && matches!(index, 8 | 13 | 18 | 23))
    });
    let compact_prefix = canonical_prefix.replace('-', "");
    if !(4..=32).contains(&compact_prefix.len())
        || !separators_are_canonical
        || !compact_prefix.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(CliError::Argument(format!(
            "memory id {value:?} is not a UUID or canonical hexadecimal UUID prefix"
        )));
    }
    let matches = list_memories(state, 100)?
        .into_iter()
        .filter(|memory| {
            memory
                .id
                .to_string()
                .replace('-', "")
                .starts_with(&compact_prefix)
        })
        .map(|memory| memory.id)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [id] => Ok(*id),
        [] => Err(CliError::Argument(format!(
            "no active memory matches prefix {value:?}"
        ))),
        _ => Err(CliError::Argument(format!(
            "memory prefix {value:?} is ambiguous"
        ))),
    }
}

fn render_memories(memories: &[MemoryRecord], json_output: bool) -> Result<(), CliError> {
    if json_output {
        return write_json(memories);
    }
    if memories.is_empty() {
        return write_human_stdout("No matching Pactrail memories.\n").map_err(CliError::Output);
    }
    let text = memories
        .iter()
        .map(|memory| {
            format!(
                "{}  {:<11}  {}\n    {}",
                memory.id,
                memory.kind,
                memory.title,
                memory.content.replace('\n', " ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    write_human_stdout(&format!("{text}\n")).map_err(CliError::Output)
}

fn default_memory_title(content: &str) -> String {
    let title = content
        .split_whitespace()
        .take(10)
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        "Workspace memory".to_owned()
    } else {
        title
    }
}

const fn memory_kind(kind: MemoryKindArg) -> MemoryKind {
    match kind {
        MemoryKindArg::Convention => MemoryKind::Convention,
        MemoryKindArg::Decision => MemoryKind::Decision,
        MemoryKindArg::Warning => MemoryKind::Warning,
    }
}

pub(crate) fn doctor(json_output: bool) -> Result<(), CliError> {
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
    let command_available = |name: &str| {
        checks
            .iter()
            .any(|check| check["program"] == name && check["available"].as_bool().unwrap_or(false))
    };
    let process_backends = json!([
        {
            "id": "disabled",
            "available": true,
            "trust": "no process execution",
            "network": "denied",
            "filesystem": "none"
        },
        {
            "id": "native",
            "available": true,
            "trust": "explicitly trusted host execution",
            "network": "host authority",
            "filesystem": "candidate working directory plus host process authority"
        },
        {
            "id": "oci-docker",
            "available": command_available("docker"),
            "trust": "restricted local OCI execution",
            "network": "denied",
            "filesystem": "candidate-only writable bind mount; read-only image"
        },
        {
            "id": "oci-podman",
            "available": command_available("podman"),
            "trust": "restricted local OCI execution",
            "network": "denied",
            "filesystem": "candidate-only writable bind mount; read-only image"
        }
    ]);
    let report = json!({
        "native_process_isolation": "workspace transaction only; not a host-filesystem or network sandbox",
        "recommended_hostile_repo_backend": "--process-backend oci --sandbox-image <local-image>",
        "process_backends": process_backends,
        "commands": checks,
    });
    if json_output {
        write_json(&report)
    } else {
        let mut lines = vec![
            "Process boundaries".to_owned(),
            "  disabled    ready       no process execution".to_owned(),
            "  native      ready       trusted host authority; candidate working directory"
                .to_owned(),
            format!(
                "  oci/docker  {:<10} network denied; candidate-only writable mount",
                if command_available("docker") {
                    "detected"
                } else {
                    "not found"
                }
            ),
            format!(
                "  oci/podman  {:<10} network denied; candidate-only writable mount",
                if command_available("podman") {
                    "detected"
                } else {
                    "not found"
                }
            ),
            String::new(),
            "Native mode is not a host-filesystem or network sandbox.".to_owned(),
            "Use --process-backend oci --sandbox-image <local-image> for untrusted commands."
                .to_owned(),
            String::new(),
            "Toolchain discovery".to_owned(),
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
        write_human_stdout(&format!("{}\n", lines.join("\n"))).map_err(CliError::Output)
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
            "approvals": receipt.approvals,
            "risks": receipt.unresolved_risks,
            "tokens": tokens,
            "receipt": run_root.join("receipt.json"),
            "trace": run_root.join("trace.jsonl"),
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
            write_human_stdout(&format!(
                "Run: {}\nOutcome: {:?}\n\n{}\n\nChanged files:\n{}\n\nEvidence: {} passed, {} failed, {} inconclusive\nTokens: {}\nReceipt: {}\nTrace: {}\nApply: {}\n",
                receipt.run_id,
                receipt.outcome,
                model_summary,
                changes,
                receipt.verification.passed,
                receipt.verification.failed,
                receipt.verification.inconclusive,
                tokens,
                run_root.join("receipt.json").display(),
                run_root.join("trace.jsonl").display(),
                apply_hint,
            ))
            .map_err(CliError::Output)
        }
    }
}

pub(crate) fn state_dir(
    workspace: &Path,
    override_path: Option<&Path>,
) -> Result<PathBuf, CliError> {
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

pub(crate) fn run_root(state: &Path, run_id: RunId) -> PathBuf {
    state.join("runs").join(run_id.to_string())
}

pub(crate) fn parse_run_id(value: &str) -> Result<RunId, CliError> {
    RunId::from_str(value)
        .map_err(|error| CliError::Argument(format!("invalid run id {value:?}: {error}")))
}

fn write_run_manifest(run_root: &Path, manifest: &RunManifest) -> Result<String, CliError> {
    let path = run_root.join("run.json");
    let backup = run_root.join("run.json.bak");
    let bytes = serde_json::to_vec_pretty(manifest).map_err(CliError::Json)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_RUN_MANIFEST_BYTES {
        return Err(CliError::Argument(format!(
            "run manifest exceeds the {MAX_RUN_MANIFEST_BYTES}-byte safety limit"
        )));
    }
    write_atomic_artifact(&path, &backup, &bytes)?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn runtime_identity(
    manifest_identity: &str,
    process_backend: &dyn ProcessBackend,
    mcp_snapshot_digests: &[String],
) -> Result<String, CliError> {
    #[derive(Serialize)]
    struct RuntimeIdentity<'a> {
        manifest: &'a str,
        process_backend: pactrail_tools::ProcessBackendDescriptor,
    }

    let process_backend = process_backend.descriptor();
    // Preserve the pre-MCP identity for runs without MCP so v0.6 checkpoints remain resumable.
    let bytes = if mcp_snapshot_digests.is_empty() {
        serde_json::to_vec(&RuntimeIdentity {
            manifest: manifest_identity,
            process_backend,
        })
        .map_err(CliError::Json)?
    } else {
        #[derive(Serialize)]
        struct RuntimeIdentityWithMcp<'a> {
            manifest: &'a str,
            process_backend: pactrail_tools::ProcessBackendDescriptor,
            mcp_snapshot_digests: &'a [String],
        }
        serde_json::to_vec(&RuntimeIdentityWithMcp {
            manifest: manifest_identity,
            process_backend,
            mcp_snapshot_digests,
        })
        .map_err(CliError::Json)?
    };
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn read_run_manifest(run_root: &Path) -> Result<(RunManifest, String), CliError> {
    let path = run_root.join("run.json");
    let backup = run_root.join("run.json.bak");
    if !path.exists() && backup.is_file() {
        fs::rename(&backup, &path).map_err(|source| CliError::Io {
            path: backup,
            source,
        })?;
    }
    read_run_manifest_exact(run_root)
}

fn read_run_manifest_exact(run_root: &Path) -> Result<(RunManifest, String), CliError> {
    let path = run_root.join("run.json");
    let bytes = read_bounded_regular_file(&path, MAX_RUN_MANIFEST_BYTES, "run manifest")?;
    let digest = blake3::hash(&bytes).to_hex().to_string();
    let manifest = serde_json::from_slice(&bytes).map_err(CliError::Json)?;
    Ok((manifest, digest))
}

pub(crate) fn read_receipt(run_root: &Path) -> Result<ChangeReceipt, CliError> {
    let path = run_root.join("receipt.json");
    let backup = run_root.join("receipt.json.bak");
    if !path.exists() && backup.is_file() {
        fs::rename(&backup, &path).map_err(|source| CliError::Io {
            path: backup,
            source,
        })?;
    }
    read_receipt_exact(run_root)
}

fn read_receipt_exact(run_root: &Path) -> Result<ChangeReceipt, CliError> {
    let path = run_root.join("receipt.json");
    let bytes = read_bounded_regular_file(&path, MAX_RECEIPT_BYTES, "receipt")?;
    serde_json::from_slice(&bytes).map_err(CliError::Json)
}

fn write_receipt(run_root: &Path, receipt: &ChangeReceipt) -> Result<(), CliError> {
    let path = run_root.join("receipt.json");
    let backup = run_root.join("receipt.json.bak");
    let bytes = serde_json::to_vec_pretty(receipt).map_err(CliError::Json)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_RECEIPT_BYTES {
        return Err(CliError::Argument(format!(
            "receipt exceeds the {MAX_RECEIPT_BYTES}-byte safety limit"
        )));
    }
    write_atomic_artifact(&path, &backup, &bytes)
}

fn read_bounded_regular_file(path: &Path, limit: u64, label: &str) -> Result<Vec<u8>, CliError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| CliError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CliError::Argument(format!(
            "{label} path {} is not a regular, non-symlink file",
            path.display()
        )));
    }
    if metadata.len() > limit {
        return Err(CliError::Argument(format!(
            "{label} {} exceeds the {limit}-byte safety limit",
            path.display()
        )));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    fs::File::open(path)
        .and_then(|file| file.take(limit.saturating_add(1)).read_to_end(&mut bytes))
        .map_err(|source| CliError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(CliError::Argument(format!(
            "{label} {} grew beyond its safety limit while being read",
            path.display()
        )));
    }
    Ok(bytes)
}

fn optional_regular_file(path: &Path, label: &str) -> Result<bool, CliError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(CliError::Argument(format!(
                "{label} path {} is not a regular, non-symlink file",
                path.display()
            )))
        }
        Ok(_) => Ok(true),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(CliError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn write_trace_artifact(
    run_root: &Path,
    store: &EventStore,
    run_id: RunId,
) -> Result<(), CliError> {
    let events = store.load(run_id)?;
    let mut bytes = Vec::new();
    for event in events {
        serde_json::to_writer(&mut bytes, &event).map_err(CliError::Json)?;
        bytes.push(b'\n');
    }
    write_atomic_artifact(
        &run_root.join("trace.jsonl"),
        &run_root.join("trace.jsonl.bak"),
        &bytes,
    )
}

fn write_atomic_artifact(path: &Path, backup: &Path, bytes: &[u8]) -> Result<(), CliError> {
    if backup.exists() {
        if path.exists() {
            fs::remove_file(backup).map_err(|source| CliError::Io {
                path: backup.to_path_buf(),
                source,
            })?;
        } else {
            fs::rename(backup, path).map_err(|source| CliError::Io {
                path: backup.to_path_buf(),
                source,
            })?;
        }
    }
    let parent = path.parent().ok_or_else(|| {
        CliError::Argument(format!("artifact path {} has no parent", path.display()))
    })?;
    let mut temporary = NamedTempFile::new_in(parent).map_err(|source| CliError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    temporary
        .write_all(bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|source| CliError::Io {
            path: temporary.path().to_path_buf(),
            source,
        })?;
    if path.exists() {
        fs::rename(path, backup).map_err(|source| CliError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    }
    if let Err(error) = temporary.persist(path) {
        if backup.exists() {
            let _restore = fs::rename(backup, path);
        }
        return Err(CliError::Io {
            path: path.to_path_buf(),
            source: error.error,
        });
    }
    if backup.exists() {
        fs::remove_file(backup).map_err(|source| CliError::Io {
            path: backup.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

pub(crate) fn write_json<T: Serialize + ?Sized>(value: &T) -> Result<(), CliError> {
    let mut text = serde_json::to_string_pretty(value).map_err(CliError::Json)?;
    text.push('\n');
    write_stdout(&escape_json_terminal_controls(&text)).map_err(CliError::Output)
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
    #[error("engine failed for run {run_id}: {source}\n{trace_status}")]
    RunFailed {
        run_id: RunId,
        #[source]
        source: Box<EngineError>,
        trace_status: String,
    },
    #[error("event store failed: {0}")]
    Store(#[from] StoreError),
    #[error("workspace transaction failed: {0}")]
    Transaction(#[from] TransactionError),
    #[error("tool registry failed: {0}")]
    Tool(#[from] ToolError),
    #[error("workspace memory failed: {0}")]
    Memory(#[from] MemoryError),
    #[error("{0}")]
    Mcp(#[from] McpCliError),
    #[error("receipt failed: {0}")]
    Receipt(#[from] pactrail_core::ReceiptError),
}

impl CliError {
    pub(crate) const fn run_id(&self) -> Option<RunId> {
        match self {
            Self::RunFailed { run_id, .. } => Some(*run_id),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use pactrail_core::{Evidence, EvidenceKind};

    use super::*;

    fn tiny_png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&13_u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 2, 0, 0, 0]);
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&0_u32.to_be_bytes());
        bytes.extend_from_slice(b"IEND");
        bytes.extend_from_slice(&[0; 4]);
        bytes
    }

    #[test]
    fn image_loader_erases_paths_and_rejects_duplicate_content() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        let first = root.path().join("screen.not-an-extension");
        let second = root.path().join("copy.png");
        fs::write(&first, tiny_png(100, 50)).unwrap_or_else(|error| unreachable!("first: {error}"));
        fs::write(&second, tiny_png(100, 50))
            .unwrap_or_else(|error| unreachable!("second: {error}"));

        let images = load_input_images(std::slice::from_ref(&first))
            .unwrap_or_else(|error| unreachable!("load: {error}"));
        assert_eq!(images[0].name(), "screen.not-an-extension");
        assert!(
            !serde_json::to_string(&images[0])
                .unwrap_or_else(|error| unreachable!("json: {error}"))
                .contains(&root.path().display().to_string())
        );
        assert!(matches!(
            load_input_images(&[first, second]),
            Err(CliError::Argument(message)) if message.contains("more than once")
        ));
    }

    #[test]
    fn optional_artifact_probe_distinguishes_missing_from_unsafe_paths() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        let missing = root.path().join("missing.json");
        assert_eq!(optional_regular_file(&missing, "fixture").ok(), Some(false));

        let directory = root.path().join("receipt.json");
        fs::create_dir(&directory)
            .unwrap_or_else(|error| unreachable!("fixture directory: {error}"));
        assert!(matches!(
            optional_regular_file(&directory, "receipt"),
            Err(CliError::Argument(message)) if message.contains("non-symlink file")
        ));
    }

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
        let contract = TaskContract::new(
            "Create a README",
            transaction.source_root().display().to_string(),
        );
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
            approvals: Vec::new(),
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
    fn apply_rejects_an_integrity_valid_receipt_for_a_different_baseline() {
        let mut fixture = ready_fixture();
        let receipt = ChangeReceipt::build(ReceiptInput {
            run_id: fixture.receipt.run_id,
            contract: fixture.receipt.contract.clone(),
            outcome: fixture.receipt.outcome,
            baseline_digest: "0".repeat(64),
            final_event_hash: fixture.receipt.final_event_hash.clone(),
            changes: fixture.receipt.changes.clone(),
            evidence: fixture.receipt.evidence.clone(),
            approvals: fixture.receipt.approvals.clone(),
            unresolved_risks: fixture.receipt.unresolved_risks.clone(),
        })
        .unwrap_or_else(|error| unreachable!("alternate receipt: {error}"));
        assert!(receipt.verify_integrity().is_ok_and(|valid| valid));

        let result = apply_ready_receipt(
            &fixture.run_root,
            receipt,
            &fixture.transaction,
            &mut fixture.store,
        );
        assert!(matches!(
            result,
            Err(CliError::Argument(message)) if message.contains("not bound")
        ));
        assert!(!fixture.source.path().join("README.md").exists());
    }

    #[test]
    fn cancellation_preserves_an_integrity_checked_candidate_receipt() {
        let mut fixture = ready_fixture();
        let run_id = fixture.receipt.run_id;
        let contract = fixture.receipt.contract.clone();
        let snapshot = fixture
            .store
            .snapshot(run_id)
            .unwrap_or_else(|error| unreachable!("snapshot: {error}"));
        fixture
            .store
            .append(
                run_id,
                snapshot.last_sequence.map_or(0, |value| value + 1),
                RunEvent::StateChanged {
                    from: RunState::AwaitingApply,
                    to: RunState::Cancelled,
                },
            )
            .unwrap_or_else(|error| unreachable!("cancel event: {error}"));

        let completed = cancelled_run(
            &fixture.run_root,
            &fixture.transaction,
            &fixture.store,
            run_id,
            contract,
        )
        .unwrap_or_else(|error| unreachable!("cancelled receipt: {error}"));
        assert_eq!(completed.receipt.outcome, ReceiptOutcome::Cancelled);
        assert_eq!(completed.receipt.changes.len(), 1);
        assert_eq!(completed.receipt.verify_integrity().ok(), Some(true));
        assert!(fixture.run_root.join("review.diff").is_file());
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
        configure_process_permissions(&mut contract, ProcessBackendArg::Native, false, false)
            .unwrap_or_else(|error| unreachable!("native opt-in: {error}"));
        assert!(contract.permissions.ask.contains(&Capability::ProcessSpawn));
        assert!(
            !contract
                .permissions
                .allow
                .contains(&Capability::ProcessSpawn)
        );
        for capability in [
            Capability::Network,
            Capability::SecretUse,
            Capability::ExternalWrite,
        ] {
            assert!(contract.permissions.allow.contains(&capability));
            assert!(!contract.permissions.deny.contains(&capability));
        }
    }

    #[test]
    fn mcp_authority_is_request_scoped_and_independent_from_process_authority() {
        let mut contract = TaskContract::new("query remote issue", ".");
        contract.permissions.deny.extend([
            Capability::McpInvoke,
            Capability::Network,
            Capability::ExternalWrite,
        ]);
        let required = std::collections::BTreeSet::from([
            Capability::McpInvoke,
            Capability::Network,
            Capability::ExternalWrite,
        ]);
        configure_mcp_permissions(&mut contract, &required, McpApprovalArg::AllowRun, false)
            .unwrap_or_else(|error| unreachable!("MCP permissions: {error}"));
        for capability in required {
            assert!(contract.permissions.ask.contains(&capability));
            assert!(!contract.permissions.allow.contains(&capability));
            assert!(!contract.permissions.deny.contains(&capability));
        }

        let resolver = ConfiguredApprovalResolver {
            process: ProcessApprovalArg::Deny,
            mcp: McpApprovalArg::AllowRun,
            observer: None,
        };
        let request = ApprovalRequest {
            binding: pactrail_core::ApprovalBinding {
                run_id: RunId::new(),
                capability: Capability::McpInvoke,
                resource: "demo::mcp__demo__query".to_owned(),
                actor_fingerprint: "actor".to_owned(),
                backend_kind: "mcp_streamable_http".to_owned(),
                backend_identity: Some("runtime".to_owned()),
                profile_digest: "snapshot".to_owned(),
            },
            reason: "test".to_owned(),
            presentation: std::collections::BTreeMap::new(),
        };
        assert_eq!(resolver.resolve(&request), ApprovalDecision::AllowRun);
        let mut process_request = request;
        process_request.binding.backend_kind = "native".to_owned();
        assert_eq!(resolver.resolve(&process_request), ApprovalDecision::Deny);
    }

    #[test]
    fn authoritative_contract_must_declare_every_enabled_mcp_effect() {
        let mut contract = TaskContract::new("query remote issue", ".");
        contract.permissions.allow.insert(Capability::McpInvoke);
        let required =
            std::collections::BTreeSet::from([Capability::McpInvoke, Capability::Network]);
        let result =
            configure_mcp_permissions(&mut contract, &required, McpApprovalArg::Deny, true);
        assert!(matches!(result, Err(CliError::Argument(_))));
    }

    #[test]
    fn task_file_cannot_understate_native_process_access() {
        let mut contract = TaskContract::new("run checks", ".");
        contract.permissions.allow.insert(Capability::ProcessSpawn);
        let result =
            configure_process_permissions(&mut contract, ProcessBackendArg::Disabled, true, false);
        assert!(matches!(result, Err(CliError::Argument(_))));
    }

    #[test]
    fn oci_process_mode_records_restricted_authority() {
        let mut contract = TaskContract::new("run checks", ".");
        contract.permissions.allow.extend([
            Capability::Network,
            Capability::SecretUse,
            Capability::ExternalWrite,
        ]);
        configure_process_permissions(&mut contract, ProcessBackendArg::Oci, false, false)
            .unwrap_or_else(|error| unreachable!("OCI permissions: {error}"));
        assert!(contract.permissions.ask.contains(&Capability::ProcessSpawn));
        for capability in [
            Capability::Network,
            Capability::SecretUse,
            Capability::ExternalWrite,
        ] {
            assert!(contract.permissions.deny.contains(&capability));
            assert!(!contract.permissions.allow.contains(&capability));
        }
    }

    #[test]
    fn explicit_contract_denial_cannot_be_overridden_by_backend_selection() {
        let mut contract = TaskContract::new("run checks", ".");
        contract.permissions.deny.insert(Capability::ProcessSpawn);
        let result =
            configure_process_permissions(&mut contract, ProcessBackendArg::Oci, true, false);
        assert!(matches!(result, Err(CliError::Argument(_))));
        assert!(
            contract
                .permissions
                .deny
                .contains(&Capability::ProcessSpawn)
        );
    }

    #[test]
    fn generated_budget_can_reach_every_configured_turn() {
        assert_eq!(generated_model_token_budget(16_384, 2_048, 16), 294_912);
        assert_eq!(generated_model_token_budget(u64::MAX, 1, 256), u64::MAX);
    }
}
