use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

/// Verification-native coding agent harness.
#[derive(Debug, Parser)]
#[command(
    name = "pactrail",
    version,
    about,
    propagate_version = true,
    after_help = "Run `pactrail` without a command to start the interactive coding session."
)]
pub struct Cli {
    /// Workspace used to resolve the default state directory.
    #[arg(long, global = true, default_value = ".")]
    pub workspace: PathBuf,

    /// Override the default WORKSPACE/.pactrail state directory.
    #[arg(long, global = true)]
    pub state_dir: Option<PathBuf>,

    /// Optional task to execute immediately when the interactive session opens.
    pub prompt: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Execute a task in an isolated transaction.
    Run(RunArgs),
    /// Continue an interrupted run from its latest safe checkpoint.
    Resume(ResumeArgs),
    /// Inspect a durable run and its evidence receipt.
    Inspect(RunIdArgs),
    /// Render the integrity-checked execution trace for a run.
    Trace(RunIdArgs),
    /// Apply a ready transaction after baseline-drift checks.
    Apply(RunIdArgs),
    /// Discard a ready transaction while preserving its receipt.
    Discard(RunIdArgs),
    /// List durable runs in the selected state directory.
    List {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print the built-in tool contracts.
    Tools {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print the current task-contract JSON Schema.
    Schema,
    /// Generate a validated task-contract TOML template.
    TaskTemplate {
        /// Natural-language software task.
        goal: String,
    },
    /// Generate a shell completion script on standard output.
    Completion {
        /// Target shell.
        #[arg(value_enum)]
        shell: CompletionShell,
    },
    /// Manage provenance-aware memory for the selected workspace.
    Memory {
        #[command(subcommand)]
        command: MemoryCommand,
    },
    /// Report local execution dependencies and sandbox limitations.
    Doctor {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum MemoryCommand {
    /// List the most recent active memories.
    List {
        /// Maximum number of memories.
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=100))]
        limit: u16,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Search memory by topic, file, decision, or convention.
    Search {
        /// Lexical retrieval query.
        query: String,
        /// Maximum number of matches.
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u16).range(1..=100))]
        limit: u16,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Add an explicit human-authored workspace memory.
    Add {
        /// Memory content.
        content: String,
        /// Short title; defaults to a bounded prefix of the content.
        #[arg(long)]
        title: Option<String>,
        /// Semantic memory class.
        #[arg(long, value_enum, default_value = "convention")]
        kind: MemoryKindArg,
        /// Searchable tag. Repeatable.
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Forget one active memory by ID or unique ID prefix.
    Forget {
        /// Full memory ID or unique prefix.
        id: String,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum MemoryKindArg {
    Convention,
    Decision,
    Warning,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Elvish,
    Fish,
    #[value(name = "powershell", alias = "power-shell", alias = "pwsh")]
    PowerShell,
    Zsh,
}

/// Process trust boundary selected for a run.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessBackendArg {
    #[default]
    Disabled,
    #[value(alias = "trusted", alias = "on")]
    Native,
    #[value(alias = "sandbox", alias = "container")]
    Oci,
}

/// Local OCI runtime used for restricted processes.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum OciRuntimeArg {
    #[default]
    Docker,
    Podman,
}

/// Resolution mode for process requests that reach the approval boundary.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessApprovalArg {
    /// Deny unresolved process approvals (the non-interactive default).
    #[default]
    Deny,
    /// Approve exact process requests for the duration of this run.
    AllowRun,
    /// Ask through the active interactive frontend.
    #[value(skip)]
    Prompt,
}

#[derive(Clone, Debug, Deserialize, Serialize, Args)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct RunArgs {
    /// Natural-language software task.
    #[arg(required_unless_present = "task", conflicts_with = "task")]
    pub goal: Option<String>,

    /// Load a complete task contract from TOML.
    #[arg(long, conflicts_with = "goal")]
    pub task: Option<PathBuf>,

    /// Configured provider kind.
    #[arg(long, value_enum, default_value = "ollama")]
    pub provider: ProviderKind,

    /// Model identifier, or `PACTRAIL_MODEL`.
    #[arg(long, env = "PACTRAIL_MODEL")]
    pub model: Option<String>,

    /// Provider API base URL, or `PACTRAIL_BASE_URL`.
    #[arg(long, env = "PACTRAIL_BASE_URL")]
    pub base_url: Option<String>,

    /// Name of the environment variable containing the API key.
    #[arg(long, default_value = "OPENAI_API_KEY")]
    pub api_key_env: String,

    /// Workspace-relative path prefix the model may modify. Repeatable.
    #[arg(long = "write-path", default_value = ".")]
    pub write_paths: Vec<String>,

    /// Process execution boundary. The default is disabled.
    #[arg(long, value_enum)]
    pub process_backend: Option<ProcessBackendArg>,

    /// Trust unsandboxed processes with host, network, secret, and external access.
    ///
    /// Deprecated alias for `--process-backend native`.
    #[arg(long)]
    pub allow_process: bool,

    /// How scoped process approval requests are resolved.
    #[arg(long, value_enum)]
    pub process_approval: Option<ProcessApprovalArg>,

    /// OCI runtime for `--process-backend oci`.
    #[arg(long, value_enum, default_value = "docker")]
    pub sandbox_runtime: OciRuntimeArg,

    /// Override the Docker or Podman executable used by the sandbox backend.
    #[arg(long)]
    pub sandbox_runtime_executable: Option<PathBuf>,

    /// Locally available OCI image used by the sandbox backend.
    #[arg(long)]
    pub sandbox_image: Option<String>,

    /// Sandbox memory ceiling in MiB.
    #[arg(long, default_value_t = 2_048, value_parser = clap::value_parser!(u64).range(64..=1_048_576))]
    pub sandbox_memory_mib: u64,

    /// Sandbox CPU ceiling in thousandths of one CPU.
    #[arg(long, default_value_t = 2_000, value_parser = clap::value_parser!(u32).range(100..=256_000))]
    pub sandbox_cpu_millis: u32,

    /// Sandbox process-count ceiling.
    #[arg(long, default_value_t = 128, value_parser = clap::value_parser!(u32).range(16..=32_768))]
    pub sandbox_pids: u32,

    /// Sandbox writable temporary-space ceiling in MiB.
    #[arg(long, default_value_t = 512, value_parser = clap::value_parser!(u64).range(1..=65_536))]
    pub sandbox_tmpfs_mib: u64,

    /// Apply immediately only if the run reaches ready-to-apply state.
    #[arg(long)]
    pub apply: bool,

    /// Maximum model turns.
    #[arg(long, default_value_t = 24)]
    pub max_turns: u16,

    /// Declared model context capacity.
    #[arg(long, default_value_t = 32_768)]
    pub context_tokens: u64,

    /// Maximum output tokens per model turn.
    #[arg(long, default_value_t = 4_096)]
    pub max_output_tokens: u64,

    /// HTTP deadline for each model request, in seconds.
    #[arg(
        long,
        env = "PACTRAIL_REQUEST_TIMEOUT_SECONDS",
        default_value_t = 300,
        value_parser = clap::value_parser!(u64).range(1..=3_600)
    )]
    pub request_timeout_seconds: u64,

    /// Disable provider response streaming and wait for one buffered response.
    #[arg(long)]
    #[serde(default = "legacy_buffered_transport")]
    pub no_stream: bool,

    /// Send the provider extension `thinking.type=disabled`.
    ///
    /// Use this for compatible providers such as `DeepSeek` V4 when Pactrail's
    /// multi-turn tool protocol should run without hidden reasoning state.
    #[arg(long)]
    pub disable_thinking: bool,

    /// Result rendering format.
    #[arg(long, value_enum, default_value = "human")]
    pub output: OutputFormat,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    Ollama,
    OpenAi,
    OpenAiCompatible,
}

const fn legacy_buffered_transport() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum OutputFormat {
    Human,
    Json,
}

#[derive(Debug, Args)]
pub struct ResumeArgs {
    /// `UUIDv7` run identifier.
    pub run_id: String,

    /// Override how unresolved process approvals are handled after restart.
    #[arg(long, value_enum)]
    pub process_approval: Option<ProcessApprovalArg>,

    /// Apply immediately only if resumed work reaches ready-to-apply state.
    #[arg(long)]
    pub apply: bool,

    /// Result rendering format.
    #[arg(long, value_enum, default_value = "human")]
    pub output: OutputFormat,
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command, OciRuntimeArg, OutputFormat, ProcessBackendArg};
    use clap::Parser;

    #[test]
    fn run_request_timeout_defaults_to_five_minutes() {
        let cli = Cli::try_parse_from(["pactrail", "run", "--model", "model", "task"])
            .unwrap_or_else(|error| unreachable!("valid CLI: {error}"));
        let Some(Command::Run(args)) = cli.command else {
            unreachable!("run command")
        };
        assert_eq!(args.request_timeout_seconds, 300);
    }

    #[test]
    fn resume_parses_explicit_post_restart_authority() {
        let cli = Cli::try_parse_from([
            "pactrail",
            "resume",
            "018f53d2-a0d8-7c6a-8e22-6b6a4b0b0f54",
            "--process-approval",
            "deny",
            "--apply",
            "--output",
            "json",
        ])
        .unwrap_or_else(|error| unreachable!("valid CLI: {error}"));
        let Some(Command::Resume(args)) = cli.command else {
            unreachable!("resume command")
        };
        assert!(args.apply);
        assert_eq!(args.output, OutputFormat::Json);
        assert!(args.process_approval.is_some());
    }

    #[test]
    fn run_request_timeout_accepts_slow_local_models() {
        let cli = Cli::try_parse_from([
            "pactrail",
            "run",
            "--model",
            "model",
            "--request-timeout-seconds",
            "900",
            "task",
        ])
        .unwrap_or_else(|error| unreachable!("valid CLI: {error}"));
        let Some(Command::Run(args)) = cli.command else {
            unreachable!("run command")
        };
        assert_eq!(args.request_timeout_seconds, 900);
    }

    #[test]
    fn run_request_timeout_rejects_unbounded_values() {
        assert!(
            Cli::try_parse_from([
                "pactrail",
                "run",
                "--model",
                "model",
                "--request-timeout-seconds",
                "3601",
                "task",
            ])
            .is_err()
        );
    }

    #[test]
    fn run_can_disable_provider_thinking() {
        let cli = Cli::try_parse_from([
            "pactrail",
            "run",
            "--model",
            "model",
            "--disable-thinking",
            "task",
        ])
        .unwrap_or_else(|error| unreachable!("valid CLI: {error}"));
        let Some(Command::Run(args)) = cli.command else {
            unreachable!("run command")
        };
        assert!(args.disable_thinking);
    }

    #[test]
    fn run_parses_restricted_process_profile() {
        let cli = Cli::try_parse_from([
            "pactrail",
            "run",
            "--model",
            "model",
            "--process-backend",
            "oci",
            "--sandbox-runtime",
            "podman",
            "--sandbox-image",
            "localhost/pactrail-ci@sha256:0123",
            "--sandbox-memory-mib",
            "1024",
            "--sandbox-cpu-millis",
            "1500",
            "--sandbox-pids",
            "64",
            "--sandbox-tmpfs-mib",
            "128",
            "task",
        ])
        .unwrap_or_else(|error| unreachable!("valid CLI: {error}"));
        let Some(Command::Run(args)) = cli.command else {
            unreachable!("run command")
        };
        assert_eq!(args.process_backend, Some(ProcessBackendArg::Oci));
        assert_eq!(args.sandbox_runtime, OciRuntimeArg::Podman);
        assert_eq!(
            args.sandbox_image.as_deref(),
            Some("localhost/pactrail-ci@sha256:0123")
        );
        assert_eq!(args.sandbox_memory_mib, 1024);
        assert_eq!(args.sandbox_cpu_millis, 1500);
        assert_eq!(args.sandbox_pids, 64);
        assert_eq!(args.sandbox_tmpfs_mib, 128);
    }

    #[test]
    fn run_rejects_unsafe_sandbox_resource_values() {
        for arguments in [
            ["--sandbox-memory-mib", "63"],
            ["--sandbox-cpu-millis", "99"],
            ["--sandbox-pids", "15"],
            ["--sandbox-tmpfs-mib", "0"],
        ] {
            assert!(
                Cli::try_parse_from([
                    "pactrail",
                    "run",
                    "--model",
                    "model",
                    arguments[0],
                    arguments[1],
                    "task",
                ])
                .is_err(),
                "{} accepted {}",
                arguments[0],
                arguments[1]
            );
        }
    }
}

#[derive(Debug, Args)]
pub struct RunIdArgs {
    /// `UUIDv7` run identifier.
    pub run_id: String,

    /// Emit machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}
