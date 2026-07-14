use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use pactrail_core::{ChangeReceipt, ReceiptOutcome, RunId};
use reedline::{
    DefaultCompleter, DefaultValidator, FileBackedHistory, Prompt, PromptEditMode,
    PromptHistorySearch, PromptHistorySearchStatus, PromptViMode, Reedline, Signal,
};
use reqwest::{StatusCode, Url};
use serde_json::Value;

use crate::cli::{OutputFormat, ProviderKind, RunArgs};
use crate::commands::{self, CliError, CompletedRun};
use crate::diff::render_receipt_diff;
use crate::output::{write_human_stdout, write_stdout};
use crate::settings::{InteractiveSettings, SettingsStore};
use crate::theme::Theme;

const HISTORY_CAPACITY: usize = 2_000;
const MAX_MODEL_LIST_BYTES: usize = 1024 * 1024;
const COMMANDS: &[&str] = &[
    "/help",
    "/status",
    "/models",
    "/model",
    "/connect",
    "/provider",
    "/endpoint",
    "/key-env",
    "/context",
    "/output-tokens",
    "/turns",
    "/process",
    "/runs",
    "/inspect",
    "/review",
    "/diff",
    "/apply",
    "/discard",
    "/clear",
    "/quit",
];

pub(crate) async fn launch(
    workspace: &Path,
    state_override: Option<&Path>,
) -> Result<(), CliError> {
    let workspace = std::fs::canonicalize(workspace).map_err(|source| CliError::Io {
        path: workspace.to_path_buf(),
        source,
    })?;
    let state = commands::state_dir(&workspace, state_override)?;
    let preferences = SettingsStore::discover()
        .map_err(|error| CliError::Argument(format!("settings failed: {error}")))?;
    preferences
        .ensure_directory()
        .map_err(|error| CliError::Argument(format!("settings failed: {error}")))?;
    let settings = preferences
        .load()
        .map_err(|error| CliError::Argument(format!("settings failed: {error}")))?;
    let last_run = commands::completed_runs(&state)?
        .last()
        .map(|receipt| receipt.run_id);
    let history = FileBackedHistory::with_file(HISTORY_CAPACITY, preferences.history_path())
        .map_err(|error| CliError::Argument(format!("history failed: {error}")))?;
    let mut completer = DefaultCompleter::with_inclusions(&['/', '-']);
    completer.insert(COMMANDS.iter().map(ToString::to_string).collect());
    let editor = Reedline::create()
        .with_history(Box::new(history))
        .with_completer(Box::new(completer))
        .with_validator(Box::new(DefaultValidator));

    let mut session = Session {
        workspace,
        state,
        preferences,
        settings,
        editor,
        theme: Theme::detect(),
        last_run,
        known_models: Vec::new(),
    };
    session.bootstrap().await?;
    session.run().await
}

struct Session {
    workspace: PathBuf,
    state: PathBuf,
    preferences: SettingsStore,
    settings: InteractiveSettings,
    editor: Reedline,
    theme: Theme,
    last_run: Option<RunId>,
    known_models: Vec<String>,
}

impl Session {
    async fn bootstrap(&mut self) -> Result<(), CliError> {
        if self.settings.effective_model().is_none()
            && self.settings.provider == ProviderKind::Ollama
            && let Ok(models) = available_models(&self.settings).await
        {
            self.known_models = models;
            if let Some(model) = self.known_models.first() {
                let mut settings = self.settings.clone();
                settings.model = Some(model.clone());
                self.persist(settings)?;
            }
        }
        self.render_banner()
    }

    async fn run(&mut self) -> Result<(), CliError> {
        loop {
            let prompt = SessionPrompt::new(self.settings.effective_model());
            match self.editor.read_line(&prompt) {
                Ok(Signal::Success(line)) => {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if let Some(goal) = line.strip_prefix("//") {
                        self.execute_goal(format!("/{goal}")).await?;
                    } else if line.starts_with('/') {
                        match self.handle_command(line).await {
                            Ok(SessionControl::Continue) => {}
                            Ok(SessionControl::Exit) => break,
                            Err(error) => self.render_error(&error.to_string())?,
                        }
                    } else {
                        self.execute_goal(line.to_owned()).await?;
                    }
                }
                Ok(Signal::CtrlC) => {
                    self.emit(&format!(
                        "{}\n",
                        self.theme
                            .muted("Input cancelled. Use /quit to leave Pactrail.")
                    ))?;
                }
                Ok(Signal::CtrlD) => break,
                Ok(_) => {}
                Err(error) => {
                    return Err(CliError::Argument(format!(
                        "interactive input failed: {error}"
                    )));
                }
            }
        }
        self.emit(&format!(
            "{}\n",
            self.theme
                .muted("Session closed. Your receipts remain durable.")
        ))
    }

    async fn handle_command(&mut self, line: &str) -> Result<SessionControl, CliError> {
        let (command, arguments) = split_command(line);
        match command {
            "/help" | "/?" => self.render_help()?,
            "/status" | "/settings" => self.render_status()?,
            "/models" => self.refresh_models().await?,
            "/model" => self.set_model(arguments)?,
            "/connect" => self.connect(arguments)?,
            "/provider" => self.set_provider(arguments)?,
            "/endpoint" => self.set_endpoint(arguments)?,
            "/key-env" => self.set_api_key_env(arguments)?,
            "/context" => self.set_context(arguments)?,
            "/output-tokens" => self.set_output_tokens(arguments)?,
            "/turns" => self.set_turns(arguments)?,
            "/process" => self.set_process_access(arguments)?,
            "/runs" => self.render_runs()?,
            "/inspect" => self.inspect_run(arguments, false)?,
            "/review" => self.inspect_run(arguments, true)?,
            "/diff" => self.render_diff(self.resolve_run(arguments)?)?,
            "/apply" => self.apply_run(self.resolve_run(arguments)?)?,
            "/discard" => self.discard_run(self.resolve_run(arguments)?)?,
            "/clear" => write_stdout("\u{1b}[2J\u{1b}[H").map_err(CliError::Output)?,
            "/quit" | "/exit" => return Ok(SessionControl::Exit),
            _ => {
                return Err(CliError::Argument(format!(
                    "unknown command {command:?}; use /help"
                )));
            }
        }
        Ok(SessionControl::Continue)
    }

    async fn execute_goal(&mut self, goal: String) -> Result<(), CliError> {
        let Some(model) = self.settings.effective_model() else {
            self.render_error(
                "No model is configured. Use /models and /model, or /connect <base-url> <model>.",
            )?;
            return Ok(());
        };
        let args = RunArgs {
            goal: Some(goal),
            task: None,
            provider: self.settings.provider,
            model: Some(model),
            base_url: self.settings.effective_base_url(),
            api_key_env: self.settings.api_key_env.clone(),
            write_paths: vec![".".to_owned()],
            allow_process: self.settings.allow_process,
            apply: false,
            max_turns: self.settings.max_turns,
            context_tokens: self.settings.context_tokens,
            max_output_tokens: self.settings.max_output_tokens,
            output: OutputFormat::Human,
        };

        let spinner = ProgressBar::new_spinner();
        let style = ProgressStyle::with_template("{spinner:.cyan}  {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner())
            .tick_strings(&["\u{25d0}", "\u{25d3}", "\u{25d1}", "\u{25d2}"]);
        spinner.set_style(style);
        spinner.set_message("building context and negotiating tools");
        spinner.enable_steady_tick(Duration::from_millis(90));
        let result = commands::execute_run(&self.workspace, Some(&self.state), args).await;
        spinner.finish_and_clear();

        match result {
            Ok(completed) => {
                self.last_run = Some(completed.receipt.run_id);
                self.render_completed(&completed)?;
            }
            Err(error) => {
                self.render_error(&error.to_string())?;
                self.emit(&format!(
                    "{}\n",
                    self.theme
                        .muted("The durable event trail is available with /runs or pactrail list.")
                ))?;
            }
        }
        Ok(())
    }

    fn render_banner(&self) -> Result<(), CliError> {
        let model = self
            .settings
            .effective_model()
            .unwrap_or_else(|| "not configured".to_owned());
        let banner = format!(
            "\n{}\n{}\n\n  {}  {}\n  {}  {}\n  {}  {}\n\n{}\n\n",
            self.theme.brand("  P A C T R A I L"),
            self.theme.muted("  verification-native coding agent"),
            self.theme.muted("workspace"),
            self.theme.text(&self.workspace.display().to_string()),
            self.theme.muted("model    "),
            self.theme.accent(&model),
            self.theme.muted("policy   "),
            if self.settings.allow_process {
                self.theme
                    .warning("edits isolated · native processes trusted")
            } else {
                self.theme
                    .success("edits isolated · native processes blocked")
            },
            self.theme
                .muted("  Type a task, or /help for commands. Use // to start a task with '/'."),
        );
        self.emit(&banner)
    }

    fn render_help(&self) -> Result<(), CliError> {
        self.emit(&format!(
            "\n{}\n\
             {}\n\
             {}\n\
             {}\n\
             {}\n\
             {}\n\
             {}\n\
             {}\n\
             {}\n\
             {}\n\
             {}\n\
             {}\n\n\
             {}\n",
            self.theme.heading("Commands"),
            command_line(
                &self.theme,
                "/status",
                "workspace, model, endpoint, and safety policy"
            ),
            command_line(
                &self.theme,
                "/models",
                "discover models from the current endpoint"
            ),
            command_line(&self.theme, "/model <name|#>", "select and persist a model"),
            command_line(
                &self.theme,
                "/connect <url> <model>",
                "configure a compatible local/API endpoint"
            ),
            command_line(
                &self.theme,
                "/process on|off",
                "control unsandboxed verification commands"
            ),
            command_line(&self.theme, "/runs", "show durable run history"),
            command_line(
                &self.theme,
                "/review [run]",
                "inspect evidence and show the diff"
            ),
            command_line(
                &self.theme,
                "/apply [run]",
                "land a receipt-bound transaction"
            ),
            command_line(
                &self.theme,
                "/discard [run]",
                "discard an isolated transaction"
            ),
            command_line(&self.theme, "/clear", "clear the terminal"),
            command_line(&self.theme, "/quit", "leave the session"),
            self.theme.muted(
                "Arrow keys browse history. Ctrl-R searches it. Ctrl-C cancels input; Ctrl-D exits."
            ),
        ))
    }

    fn render_status(&self) -> Result<(), CliError> {
        let model = self
            .settings
            .effective_model()
            .unwrap_or_else(|| "not configured".to_owned());
        let endpoint =
            provider_base_url(&self.settings).unwrap_or_else(|| "not configured".to_owned());
        let process = if self.settings.allow_process {
            self.theme.warning("enabled (unsandboxed host authority)")
        } else {
            self.theme.success("disabled")
        };
        self.emit(&format!(
            "\n{}\n  {:<14} {}\n  {:<14} {}\n  {:<14} {}\n  {:<14} {}\n  {:<14} {}\n  {:<14} {}\n  {:<14} {}\n  {:<14} {}\n\n",
            self.theme.heading("Session"),
            "workspace",
            self.theme.text(&self.workspace.display().to_string()),
            "provider",
            provider_label(self.settings.provider),
            "model",
            self.theme.accent(&model),
            "endpoint",
            self.theme.text(&endpoint),
            "context",
            format_count(self.settings.context_tokens),
            "max output",
            format_count(self.settings.max_output_tokens),
            "max turns",
            self.settings.max_turns,
            "processes",
            process,
        ))
    }

    async fn refresh_models(&mut self) -> Result<(), CliError> {
        let spinner = ProgressBar::new_spinner();
        spinner.enable_steady_tick(Duration::from_millis(100));
        spinner.set_message("discovering models");
        let models = available_models(&self.settings).await;
        spinner.finish_and_clear();
        self.known_models = models.map_err(|error| CliError::Argument(error.to_string()))?;
        if self.known_models.is_empty() {
            return self.emit(&format!(
                "{}\n",
                self.theme.warning("No models reported by the endpoint.")
            ));
        }
        let mut lines = vec![self.theme.heading("Models")];
        let selected = self.settings.effective_model();
        for (index, model) in self.known_models.iter().enumerate() {
            let marker = if selected.as_deref() == Some(model.as_str()) {
                self.theme.success("\u{25cf}")
            } else {
                self.theme.muted("\u{25cb}")
            };
            lines.push(format!(
                "  {marker} {:>2}  {}",
                index + 1,
                self.theme.text(model)
            ));
        }
        lines.push(
            self.theme
                .muted("Use /model <number> or /model <name> to select one."),
        );
        self.emit(&format!("\n{}\n\n", lines.join("\n")))
    }

    fn set_model(&mut self, argument: &str) -> Result<(), CliError> {
        if argument.is_empty() {
            return Err(CliError::Argument(
                "usage: /model <name|number>; use /models to discover choices".to_owned(),
            ));
        }
        let selected = argument
            .parse::<usize>()
            .ok()
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| self.known_models.get(index).cloned())
            .unwrap_or_else(|| argument.to_owned());
        let mut settings = self.settings.clone();
        settings.model = Some(selected.clone());
        self.persist(settings)?;
        self.emit(&format!(
            "{} {}\n",
            self.theme.success("Model selected:"),
            self.theme.accent(&selected)
        ))
    }

    fn connect(&mut self, arguments: &str) -> Result<(), CliError> {
        let mut values = arguments.split_whitespace();
        let base_url = values
            .next()
            .ok_or_else(|| CliError::Argument("usage: /connect <base-url> <model>".to_owned()))?;
        let model = values
            .next()
            .ok_or_else(|| CliError::Argument("usage: /connect <base-url> <model>".to_owned()))?;
        if values.next().is_some() {
            return Err(CliError::Argument(
                "model identifiers and endpoint URLs cannot contain spaces".to_owned(),
            ));
        }
        validate_base_url(base_url)?;
        let mut settings = self.settings.clone();
        settings.provider = ProviderKind::OpenAiCompatible;
        settings.base_url = Some(base_url.to_owned());
        settings.model = Some(model.to_owned());
        self.persist(settings)?;
        self.known_models.clear();
        self.emit(&format!(
            "{} {} via {}\n",
            self.theme.success("Connected configuration saved:"),
            self.theme.accent(model),
            self.theme.text(base_url)
        ))
    }

    fn set_provider(&mut self, arguments: &str) -> Result<(), CliError> {
        let mut values = arguments.split_whitespace();
        let provider = values.next().and_then(parse_provider).ok_or_else(|| {
            CliError::Argument(
                "usage: /provider <ollama|open-ai|open-ai-compatible> [base-url]".to_owned(),
            )
        })?;
        let base_url = values.next().map(str::to_owned);
        if values.next().is_some() {
            return Err(CliError::Argument("too many provider arguments".to_owned()));
        }
        if let Some(url) = &base_url {
            validate_base_url(url)?;
        }
        let mut settings = self.settings.clone();
        settings.provider = provider;
        settings.base_url = base_url;
        self.persist(settings)?;
        self.known_models.clear();
        self.emit(&format!(
            "{} {}\n",
            self.theme.success("Provider selected:"),
            provider_label(provider)
        ))
    }

    fn set_endpoint(&mut self, argument: &str) -> Result<(), CliError> {
        if argument.is_empty() {
            return Err(CliError::Argument("usage: /endpoint <base-url>".to_owned()));
        }
        validate_base_url(argument)?;
        let mut settings = self.settings.clone();
        settings.base_url = Some(argument.to_owned());
        self.persist(settings)?;
        self.known_models.clear();
        self.emit(&format!(
            "{} {}\n",
            self.theme.success("Endpoint saved:"),
            self.theme.text(argument)
        ))
    }

    fn set_api_key_env(&mut self, argument: &str) -> Result<(), CliError> {
        if argument.is_empty() {
            return Err(CliError::Argument(
                "usage: /key-env <VARIABLE_NAME>".to_owned(),
            ));
        }
        let mut settings = self.settings.clone();
        argument.clone_into(&mut settings.api_key_env);
        self.persist(settings)?;
        self.emit(&format!(
            "{} {}\n",
            self.theme.success("API key environment variable saved:"),
            self.theme.code(argument)
        ))
    }

    fn set_context(&mut self, argument: &str) -> Result<(), CliError> {
        let value = parse_count(argument, "context")?;
        let mut settings = self.settings.clone();
        settings.context_tokens = value;
        self.persist(settings)?;
        self.emit(&format!(
            "{} {} tokens\n",
            self.theme.success("Context set to"),
            format_count(value)
        ))
    }

    fn set_output_tokens(&mut self, argument: &str) -> Result<(), CliError> {
        let value = parse_count(argument, "output token limit")?;
        let mut settings = self.settings.clone();
        settings.max_output_tokens = value;
        self.persist(settings)?;
        self.emit(&format!(
            "{} {} tokens\n",
            self.theme.success("Output limit set to"),
            format_count(value)
        ))
    }

    fn set_turns(&mut self, argument: &str) -> Result<(), CliError> {
        let value = argument.parse::<u16>().map_err(|_| {
            CliError::Argument("usage: /turns <integer between 1 and 256>".to_owned())
        })?;
        let mut settings = self.settings.clone();
        settings.max_turns = value;
        self.persist(settings)?;
        self.emit(&format!(
            "{} {value}\n",
            self.theme.success("Maximum turns set to")
        ))
    }

    fn set_process_access(&mut self, argument: &str) -> Result<(), CliError> {
        let enabled = match argument {
            "on" => true,
            "off" => false,
            _ => return Err(CliError::Argument("usage: /process on|off".to_owned())),
        };
        let mut settings = self.settings.clone();
        settings.allow_process = enabled;
        self.persist(settings)?;
        if enabled {
            self.emit(&format!(
                "{}\n{}\n",
                self.theme.warning("Native process execution enabled."),
                self.theme.muted("Commands can access the host filesystem, network, secrets, and external services.")
            ))
        } else {
            self.emit(&format!(
                "{}\n",
                self.theme.success("Native process execution disabled.")
            ))
        }
    }

    fn render_runs(&self) -> Result<(), CliError> {
        let receipts = commands::completed_runs(&self.state)?;
        if receipts.is_empty() {
            return self.emit(&format!(
                "{}\n",
                self.theme.muted("No completed runs in this workspace.")
            ));
        }
        let mut lines = vec![self.theme.heading("Recent runs")];
        for receipt in receipts.iter().rev().take(12) {
            lines.push(format!(
                "  {}  {:<15}  {:>2} files  {}",
                self.theme.code(&short_run_id(receipt.run_id)),
                outcome_text(&self.theme, receipt.outcome),
                receipt.changes.len(),
                self.theme.text(&truncate(&receipt.contract.goal, 56)),
            ));
        }
        lines.push(
            self.theme
                .muted("Commands accept a full run ID or the unique prefix shown above."),
        );
        self.emit(&format!("\n{}\n\n", lines.join("\n")))
    }

    fn inspect_run(&self, argument: &str, include_diff: bool) -> Result<(), CliError> {
        let run_id = self.resolve_run(argument)?;
        let run_root = commands::run_root(&self.state, run_id);
        let receipt = commands::read_receipt(&run_root)?;
        self.render_receipt(&receipt)?;
        if include_diff {
            self.render_diff(run_id)?;
        }
        Ok(())
    }

    fn render_diff(&self, run_id: RunId) -> Result<(), CliError> {
        let run_root = commands::run_root(&self.state, run_id);
        let receipt = commands::read_receipt(&run_root)?;
        if receipt.outcome == ReceiptOutcome::Discarded {
            return self.emit(&format!(
                "{}\n",
                self.theme
                    .warning("The candidate workspace was removed when this run was discarded.")
            ));
        }
        let diff = render_receipt_diff(&run_root, &receipt)
            .map_err(|error| CliError::Argument(format!("diff failed: {error}")))?;
        let mut output = format!("\n{}\n", self.theme.heading("Diff"));
        for line in diff.lines() {
            let rendered = if line.starts_with("+++") || line.starts_with("---") {
                self.theme.heading(line)
            } else if line.starts_with('+') {
                self.theme.addition(line)
            } else if line.starts_with('-') {
                self.theme.deletion(line)
            } else if line.starts_with("@@") {
                self.theme.accent(line)
            } else {
                self.theme.text(line)
            };
            output.push_str(&rendered);
            output.push('\n');
        }
        output.push('\n');
        self.emit(&output)
    }

    fn apply_run(&mut self, run_id: RunId) -> Result<(), CliError> {
        let receipt = commands::apply_run(&self.state, run_id)?;
        self.last_run = Some(run_id);
        self.emit(&format!(
            "{} {} files landed in {}\n",
            self.theme.success("Applied."),
            receipt.changes.len(),
            self.theme.text(&receipt.contract.workspace_root)
        ))
    }

    fn discard_run(&mut self, run_id: RunId) -> Result<(), CliError> {
        commands::discard_run(&self.state, run_id)?;
        self.last_run = Some(run_id);
        self.emit(&format!(
            "{} {}\n",
            self.theme.warning("Discarded isolated run"),
            self.theme.code(&run_id.to_string())
        ))
    }

    fn render_completed(&self, completed: &CompletedRun) -> Result<(), CliError> {
        self.render_receipt(&completed.receipt)?;
        let summary = if completed.model_summary.trim().is_empty() {
            "(model returned no summary)"
        } else {
            completed.model_summary.trim()
        };
        self.emit(&format!(
            "{}\n{}\n\n{} {}\n",
            self.theme.heading("Model summary"),
            self.theme.text(summary),
            self.theme.muted("Tokens"),
            format_count(completed.tokens),
        ))?;
        if completed.receipt.outcome == ReceiptOutcome::ReadyToApply {
            self.emit(&format!(
                "{}\n\n",
                self.theme.muted("Review with /diff, then /apply or /discard. The source workspace is still untouched.")
            ))?;
        }
        Ok(())
    }

    fn render_receipt(&self, receipt: &ChangeReceipt) -> Result<(), CliError> {
        let integrity = receipt.verify_integrity()?;
        let mut lines = vec![format!(
            "{}  {}",
            outcome_text(&self.theme, receipt.outcome),
            self.theme.code(&receipt.run_id.to_string())
        )];
        lines.push(format!(
            "  {} {}",
            self.theme.muted("goal     "),
            self.theme.text(&receipt.contract.goal)
        ));
        lines.push(format!(
            "  {} {} passed · {} failed · {} inconclusive · {} skipped",
            self.theme.muted("evidence "),
            receipt.verification.passed,
            receipt.verification.failed,
            receipt.verification.inconclusive,
            receipt.verification.skipped,
        ));
        lines.push(format!(
            "  {} {}",
            self.theme.muted("integrity"),
            if integrity {
                self.theme.success("verified")
            } else {
                self.theme.danger("INVALID")
            }
        ));
        if receipt.changes.is_empty() {
            lines.push(format!("  {} (none)", self.theme.muted("changes  ")));
        } else {
            lines.push(format!(
                "  {} {}",
                self.theme.muted("changes  "),
                receipt
                    .changes
                    .iter()
                    .map(|change| self.theme.code(&change.path))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        for risk in &receipt.unresolved_risks {
            lines.push(format!(
                "  {} {}",
                self.theme.warning("risk     "),
                self.theme.text(risk)
            ));
        }
        self.emit(&format!("\n{}\n\n", lines.join("\n")))
    }

    fn resolve_run(&self, argument: &str) -> Result<RunId, CliError> {
        if argument.is_empty() {
            return self.last_run.ok_or_else(|| {
                CliError::Argument("no run selected; use /runs or provide a run ID".to_owned())
            });
        }
        if let Ok(run_id) = commands::parse_run_id(argument) {
            return Ok(run_id);
        }
        let matches = commands::completed_runs(&self.state)?
            .into_iter()
            .map(|receipt| receipt.run_id)
            .filter(|run_id| run_id.to_string().starts_with(argument))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [run_id] => Ok(*run_id),
            [] => Err(CliError::Argument(format!(
                "no run matches prefix {argument:?}"
            ))),
            _ => Err(CliError::Argument(format!(
                "run prefix {argument:?} is ambiguous"
            ))),
        }
    }

    fn persist(&mut self, settings: InteractiveSettings) -> Result<(), CliError> {
        self.preferences
            .save(&settings)
            .map_err(|error| CliError::Argument(format!("settings failed: {error}")))?;
        self.settings = settings;
        Ok(())
    }

    fn render_error(&self, message: &str) -> Result<(), CliError> {
        self.emit(&format!(
            "{} {}\n",
            self.theme.danger("Error:"),
            self.theme.text(message)
        ))
    }

    fn emit(&self, value: &str) -> Result<(), CliError> {
        if self.theme.has_color() {
            write_stdout(value).map_err(CliError::Output)
        } else {
            write_human_stdout(value).map_err(CliError::Output)
        }
    }
}

enum SessionControl {
    Continue,
    Exit,
}

struct SessionPrompt {
    model: String,
}

impl SessionPrompt {
    fn new(model: Option<String>) -> Self {
        Self {
            model: model.unwrap_or_else(|| "no model".to_owned()),
        }
    }
}

impl Prompt for SessionPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Borrowed("pactrail")
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.model)
    }

    fn render_prompt_indicator(&self, edit_mode: PromptEditMode) -> Cow<'_, str> {
        match edit_mode {
            PromptEditMode::Vi(PromptViMode::Normal | PromptViMode::Visual) => {
                Cow::Borrowed(" \u{25c7} ")
            }
            _ => Cow::Borrowed(" \u{276f} "),
        }
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed(" \u{00b7} ")
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        let prefix = match history_search.status {
            PromptHistorySearchStatus::Failing => "failing ",
            PromptHistorySearchStatus::Passing => "",
        };
        Cow::Owned(format!("({prefix}search: {}) ", history_search.term))
    }
}

async fn available_models(settings: &InteractiveSettings) -> Result<Vec<String>, ModelListError> {
    let base_url = provider_base_url(settings).ok_or(ModelListError::MissingEndpoint)?;
    let endpoint = models_endpoint(&base_url)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("pactrail/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let mut request = client.get(endpoint);
    if settings.provider != ProviderKind::Ollama
        && let Ok(api_key) = std::env::var(&settings.api_key_env)
        && !api_key.is_empty()
    {
        request = request.bearer_auth(api_key);
    }
    let response = request.send().await?;
    let status = response.status();
    let bytes = read_bounded(response).await?;
    if status != StatusCode::OK {
        let message = String::from_utf8_lossy(&bytes).chars().take(500).collect();
        return Err(ModelListError::Provider(status.as_u16(), message));
    }
    let value: Value = serde_json::from_slice(&bytes)?;
    let mut models = value
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str))
        .filter(|model| !model.trim().is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    models.sort();
    models.dedup();
    Ok(models)
}

async fn read_bounded(response: reqwest::Response) -> Result<Vec<u8>, ModelListError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MODEL_LIST_BYTES as u64)
    {
        return Err(ModelListError::ResponseTooLarge);
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if bytes.len().saturating_add(chunk.len()) > MAX_MODEL_LIST_BYTES {
            return Err(ModelListError::ResponseTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn models_endpoint(base_url: &str) -> Result<Url, ModelListError> {
    validate_base_url(base_url)
        .map_err(|error| ModelListError::InvalidEndpoint(error.to_string()))?;
    Url::parse(&format!("{}/models", base_url.trim_end_matches('/')))
        .map_err(|error| ModelListError::InvalidEndpoint(error.to_string()))
}

fn validate_base_url(base_url: &str) -> Result<(), CliError> {
    let endpoint = Url::parse(base_url)
        .map_err(|error| CliError::Argument(format!("invalid endpoint: {error}")))?;
    let host = endpoint.host_str().unwrap_or_default();
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if endpoint.scheme() != "https" && !(endpoint.scheme() == "http" && loopback) {
        return Err(CliError::Argument(
            "remote endpoints must use HTTPS; plain HTTP is allowed only on loopback".to_owned(),
        ));
    }
    if !endpoint.username().is_empty() || endpoint.password().is_some() {
        return Err(CliError::Argument(
            "endpoint credentials must be supplied through an environment variable".to_owned(),
        ));
    }
    if endpoint.query().is_some() || endpoint.fragment().is_some() {
        return Err(CliError::Argument(
            "endpoint must not contain a query or fragment".to_owned(),
        ));
    }
    Ok(())
}

fn provider_base_url(settings: &InteractiveSettings) -> Option<String> {
    settings
        .effective_base_url()
        .or_else(|| match settings.provider {
            ProviderKind::Ollama => Some("http://127.0.0.1:11434/v1".to_owned()),
            ProviderKind::OpenAi => Some("https://api.openai.com/v1".to_owned()),
            ProviderKind::OpenAiCompatible => None,
        })
}

fn parse_provider(value: &str) -> Option<ProviderKind> {
    match value {
        "ollama" => Some(ProviderKind::Ollama),
        "openai" | "open-ai" => Some(ProviderKind::OpenAi),
        "compatible" | "openai-compatible" | "open-ai-compatible" => {
            Some(ProviderKind::OpenAiCompatible)
        }
        _ => None,
    }
}

fn provider_label(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Ollama => "ollama",
        ProviderKind::OpenAi => "open-ai",
        ProviderKind::OpenAiCompatible => "open-ai-compatible",
    }
}

fn outcome_text(theme: &Theme, outcome: ReceiptOutcome) -> String {
    match outcome {
        ReceiptOutcome::ReadyToApply => theme.success("READY TO APPLY"),
        ReceiptOutcome::Applied => theme.success("APPLIED"),
        ReceiptOutcome::Discarded => theme.warning("DISCARDED"),
        ReceiptOutcome::Failed => theme.danger("FAILED"),
        ReceiptOutcome::Cancelled => theme.warning("CANCELLED"),
    }
}

fn split_command(line: &str) -> (&str, &str) {
    line.split_once(char::is_whitespace)
        .map_or((line, ""), |(command, arguments)| {
            (command, arguments.trim())
        })
}

fn command_line(theme: &Theme, command: &str, description: &str) -> String {
    format!("  {:<31} {}", theme.code(command), theme.muted(description))
}

fn parse_count(value: &str, name: &str) -> Result<u64, CliError> {
    let normalized = value.replace(['_', ','], "");
    normalized
        .parse::<u64>()
        .map_err(|_| CliError::Argument(format!("{name} must be a positive integer")))
}

fn format_count(value: u64) -> String {
    let text = value.to_string();
    let mut output = String::with_capacity(text.len() + text.len() / 3);
    for (index, character) in text.chars().enumerate() {
        if index > 0 && (text.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}

fn short_run_id(run_id: RunId) -> String {
    run_id.to_string().chars().take(8).collect()
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut characters = value.chars();
    let prefix = characters.by_ref().take(max_chars).collect::<String>();
    if characters.next().is_some() {
        format!("{prefix}\u{2026}")
    } else {
        prefix
    }
}

#[derive(Debug, thiserror::Error)]
enum ModelListError {
    #[error("no endpoint is configured; use /connect <base-url> <model>")]
    MissingEndpoint,
    #[error("invalid model-list endpoint: {0}")]
    InvalidEndpoint(String),
    #[error("model discovery transport failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("model endpoint returned HTTP {0}: {1}")]
    Provider(u16, String),
    #[error("model endpoint returned more than {MAX_MODEL_LIST_BYTES} bytes")]
    ResponseTooLarge,
    #[error("model endpoint returned invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_reject_remote_http_and_url_credentials() {
        assert!(validate_base_url("http://example.com/v1").is_err());
        assert!(validate_base_url("https://user:pass@example.com/v1").is_err());
        assert!(validate_base_url("http://127.0.0.1:8080/v1").is_ok());
    }

    #[test]
    fn counts_and_text_truncation_are_stable() {
        assert_eq!(format_count(1_234_567), "1,234,567");
        assert_eq!(truncate("abcdef", 3), "abc\u{2026}");
        assert_eq!(truncate("abc", 3), "abc");
    }
}
