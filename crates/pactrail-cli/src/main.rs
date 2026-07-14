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
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
    let cli = Cli::parse();
    let result = if cli.command.is_none() {
        if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
            Err(commands::CliError::Argument(
                "interactive mode requires a terminal; use `pactrail run <goal>` for automation"
                    .to_owned(),
            ))
        } else {
            interactive::launch(&cli.workspace, cli.state_dir.as_deref()).await
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
