//! Pactrail command-line entry point.

mod cli;
mod commands;
mod diff;
mod interactive;
mod output;
mod settings;
mod theme;

use std::io::IsTerminal;
use std::process::ExitCode;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::cli::Cli;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let interactive = cli.command.is_none();
    let conflicting_prompt = cli.prompt.is_some() && !interactive;
    let filter = if interactive {
        std::env::var("PACTRAIL_LOG")
            .ok()
            .and_then(|value| EnvFilter::try_new(value).ok())
            .unwrap_or_else(|| EnvFilter::new("error"))
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"))
    };
    let ansi = std::io::stderr().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").map_or(true, |term| term != "dumb");
    let _subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(ansi)
        .try_init();
    let result = if conflicting_prompt {
        Err(commands::CliError::Argument(
            "an interactive PROMPT cannot be combined with a subcommand; use `pactrail run <goal>` for a one-shot run"
                .to_owned(),
        ))
    } else if interactive {
        if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
            Err(commands::CliError::Argument(
                "interactive mode requires a terminal; use `pactrail run <goal>` for automation"
                    .to_owned(),
            ))
        } else {
            interactive::launch(
                &cli.workspace,
                cli.state_dir.as_deref(),
                cli.prompt.as_deref(),
            )
            .await
        }
    } else {
        commands::dispatch(cli).await
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _write_result = output::write_stderr(&format!("error: {error}\n"));
            ExitCode::FAILURE
        }
    }
}
