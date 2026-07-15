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
    /// Inspect a durable run and its evidence receipt.
    Inspect(RunIdArgs),
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
    /// Report local execution dependencies and sandbox limitations.
    Doctor {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
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

#[derive(Debug, Args)]
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

    /// Trust unsandboxed processes with host, network, secret, and external access.
    #[arg(long)]
    pub allow_process: bool,

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

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
}

#[derive(Debug, Args)]
pub struct RunIdArgs {
    /// `UUIDv7` run identifier.
    pub run_id: String,

    /// Emit machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}
