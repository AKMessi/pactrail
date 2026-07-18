use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU16, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use pactrail_core::{
    ActionRecord, ChangeReceipt, EventEnvelope, FileChange, ReceiptOutcome, RunEvent, RunId,
    RunState,
};
use pactrail_engine::{RunObserver, RunProgress};
use pactrail_memory::{MemoryDraft, MemoryKind};
use pactrail_tools::{ToolDescriptor, ToolRisk, builtin_registry};
use reedline::{
    DefaultCompleter, DefaultValidator, FileBackedHistory, Prompt, PromptEditMode,
    PromptHistorySearch, PromptHistorySearchStatus, PromptViMode, Reedline, Signal,
};
use reqwest::{StatusCode, Url};
use serde_json::Value;
use terminal_size::{Width, terminal_size};

use crate::cli::{OutputFormat, ProviderKind, RunArgs};
use crate::commands::{self, CliError, CompletedRun};
use crate::diff::render_receipt_diff;
use crate::output::{sanitize_terminal_text, write_human_stdout, write_stdout};
use crate::settings::{InteractiveSettings, SettingsStore};
use crate::theme::Theme;

const HISTORY_CAPACITY: usize = 2_000;
const MAX_MODEL_LIST_BYTES: usize = 1024 * 1024;
const MAX_DISCOVERED_MODELS: usize = 1_000;
const MAX_DISCOVERED_MODEL_BYTES: usize = 512;
const DEFAULT_TERMINAL_COLUMNS: usize = 100;
const MIN_TERMINAL_COLUMNS: usize = 40;
const MAX_TERMINAL_COLUMNS: usize = 240;
const HELP_GROUPS: &[&str] = &["Work", "Memory", "Model", "Kernel", "Safety", "Session"];
const COMMANDS: &[CommandHelp] = &[
    CommandHelp::new("Work", "/review [run]", "show receipt and immutable diff"),
    CommandHelp::new("Work", "/diff [run]", "review candidate changes"),
    CommandHelp::new("Work", "/apply [run]", "land a verified candidate"),
    CommandHelp::new(
        "Work",
        "/discard [run]",
        "reject a candidate; retain evidence",
    ),
    CommandHelp::new("Work", "/runs", "browse durable run history"),
    CommandHelp::new(
        "Work",
        "/trace [run]",
        "show the verified model/tool/event timeline",
    ),
    CommandHelp::new(
        "Work",
        "/inspect [run]",
        "inspect a receipt without its diff",
    ),
    CommandHelp::new(
        "Memory",
        "/memory [query]",
        "browse or search workspace memory",
    ),
    CommandHelp::new(
        "Memory",
        "/remember [kind] <text>",
        "save a convention, decision, or warning",
    ),
    CommandHelp::new(
        "Memory",
        "/forget <id>",
        "remove a memory by ID or unique prefix",
    ),
    CommandHelp::new("Model", "/models", "discover models from the endpoint"),
    CommandHelp::new("Model", "/model <name|#>", "select and persist a model"),
    CommandHelp::new(
        "Model",
        "/connect <url> <model>",
        "connect any compatible endpoint",
    ),
    CommandHelp::new("Model", "/provider <kind> [url]", "switch provider adapter"),
    CommandHelp::new("Model", "/endpoint <url>", "change the active API endpoint"),
    CommandHelp::new(
        "Model",
        "/key-env <name>",
        "select the API-key environment variable",
    ),
    CommandHelp::new(
        "Kernel",
        "/tools",
        "inspect typed tools, capabilities, and risk classes",
    ),
    CommandHelp::new(
        "Safety",
        "/process on|off",
        "control trusted native verification",
    ),
    CommandHelp::new("Safety", "/context <tokens>", "set model context capacity"),
    CommandHelp::new(
        "Safety",
        "/output-tokens <tokens>",
        "set output budget per turn",
    ),
    CommandHelp::new(
        "Safety",
        "/turns <count>",
        "set the model-turn safety bound",
    ),
    CommandHelp::new(
        "Session",
        "/status",
        "show model, limits, safety, and review state",
    ),
    CommandHelp::new("Session", "/doctor", "check local runtimes and isolation"),
    CommandHelp::new(
        "Session",
        "/help [command]",
        "show commands or focused help",
    ),
    CommandHelp::new("Session", "/clear", "clear the terminal"),
    CommandHelp::new("Session", "/quit", "close the session; retain all receipts"),
];

struct CommandHelp {
    group: &'static str,
    usage: &'static str,
    description: &'static str,
}

impl CommandHelp {
    const fn new(group: &'static str, usage: &'static str, description: &'static str) -> Self {
        Self {
            group,
            usage,
            description,
        }
    }

    fn name(&self) -> &'static str {
        self.usage.split_whitespace().next().unwrap_or(self.usage)
    }
}

pub(crate) async fn launch(
    workspace: &Path,
    state_override: Option<&Path>,
    initial_goal: Option<&str>,
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
    let receipts = commands::completed_runs(&state)?;
    let (last_run, pending_runs) = run_focus(&receipts);
    let memory_count = commands::list_memories(&state, 100)?.len();
    let history = FileBackedHistory::with_file(HISTORY_CAPACITY, preferences.history_path())
        .map_err(|error| CliError::Argument(format!("history failed: {error}")))?;
    let mut completer = DefaultCompleter::with_inclusions(&['/', '-']);
    completer.insert(
        COMMANDS
            .iter()
            .map(|command| command.name().to_owned())
            .collect(),
    );
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
        pending_runs,
        memory_count,
        known_models: Vec::new(),
    };
    session.bootstrap().await?;
    if let Some(goal) = initial_goal {
        session.execute_goal(goal.to_owned()).await?;
    }
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
    pending_runs: usize,
    memory_count: usize,
    known_models: Vec<String>,
}

#[derive(Clone, Copy)]
enum TimelineTone {
    Normal,
    Muted,
    Brand,
    Accent,
    Success,
    Warning,
    Danger,
}

impl TimelineTone {
    fn paint(self, theme: &Theme, value: &str) -> String {
        match self {
            Self::Normal => theme.text(value),
            Self::Muted => theme.muted(value),
            Self::Brand => theme.brand(value),
            Self::Accent => theme.accent(value),
            Self::Success => theme.success(value),
            Self::Warning => theme.warning(value),
            Self::Danger => theme.danger(value),
        }
    }
}

fn timeline_row(
    theme: &Theme,
    columns: usize,
    elapsed_ms: u64,
    marker: &str,
    label: &str,
    detail: &str,
    tone: TimelineTone,
) -> Vec<String> {
    let time = theme.muted(&format!("{:>7}", trace_duration(elapsed_ms)));
    let marker = tone.paint(theme, marker);
    let label = tone.paint(theme, &format!("{label:<9}"));
    wrap_text(detail, content_width(columns, 25, 96))
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                format!(
                    "  {} {time}  {marker} {label} {}",
                    theme.muted("│"),
                    theme.text(&line)
                )
            } else {
                format!("                         {}", theme.text(&line))
            }
        })
        .collect()
}

fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

const fn format_state(state: RunState) -> &'static str {
    match state {
        RunState::Created => "created",
        RunState::Contracting => "contracting",
        RunState::Investigating => "investigating",
        RunState::Planning => "planning",
        RunState::Executing => "executing",
        RunState::Verifying => "verifying",
        RunState::Reviewing => "reviewing",
        RunState::Completed => "answered",
        RunState::AwaitingApply => "ready to apply",
        RunState::Applied => "applied",
        RunState::Discarded => "discarded",
        RunState::Failed => "failed",
        RunState::Cancelled => "cancelled",
    }
}

struct RunActivity {
    progress: ProgressBar,
    theme: Theme,
    model: String,
    turn: AtomicU16,
    tool_calls: AtomicUsize,
    model_tokens: AtomicU64,
    model_time_ms: AtomicU64,
    truncated_outputs: AtomicUsize,
    started: Instant,
    columns: usize,
}

impl RunActivity {
    fn new(model: &str, theme: Theme) -> Self {
        let progress = ProgressBar::new_spinner();
        let style = ProgressStyle::with_template(
            "  {spinner:.cyan}  {msg}  {elapsed_precise:.bright_black}",
        )
        .unwrap_or_else(|_| ProgressStyle::default_spinner())
        .tick_strings(&["\u{25d0}", "\u{25d3}", "\u{25d1}", "\u{25d2}"]);
        progress.set_style(style);
        progress.set_message("starting isolated transaction");
        progress.enable_steady_tick(Duration::from_millis(90));
        Self {
            progress,
            theme,
            model: truncate(&sanitize_terminal_text(model), 32),
            turn: AtomicU16::new(0),
            tool_calls: AtomicUsize::new(0),
            model_tokens: AtomicU64::new(0),
            model_time_ms: AtomicU64::new(0),
            truncated_outputs: AtomicUsize::new(0),
            started: Instant::now(),
            columns: terminal_columns(),
        }
    }

    fn finish(&self) {
        self.progress.finish_and_clear();
    }

    fn summary(&self, succeeded: bool) -> String {
        let turns = self.turn.load(Ordering::Relaxed);
        let tools = self.tool_calls.load(Ordering::Relaxed);
        let turn_word = plural(turns.into(), "turn", "turns");
        let tool_word = plural(tools, "tool", "tools");
        let tokens = self.model_tokens.load(Ordering::Relaxed);
        let model_time = format_duration(Duration::from_millis(
            self.model_time_ms.load(Ordering::Relaxed),
        ));
        let elapsed = format_duration(self.started.elapsed());
        let duration = self.theme.muted(&elapsed);
        let truncated = self.truncated_outputs.load(Ordering::Relaxed);
        let truncation_text = if truncated == 0 {
            String::new()
        } else {
            format!(" · {truncated} bounded")
        };
        let outcome = if succeeded {
            self.theme.success("✓ complete")
        } else {
            self.theme.danger("× stopped")
        };
        let metrics = format!(
            "{turns} {turn_word} · {tools} {tool_word} · {} tokens · {model_time} model · {elapsed}{truncation_text}",
            format_count(tokens),
        );
        if self.columns < 80 {
            let mut lines = vec![format!("  {} {outcome}", self.theme.muted("╰─"))];
            lines.extend(
                wrap_text(&metrics, content_width(self.columns, 5, 96))
                    .into_iter()
                    .map(|line| format!("     {}", self.theme.muted(&line))),
            );
            format!("{}\n", lines.join("\n"))
        } else {
            format!(
                "  {} {outcome}  {turns} {turn_word} · {tools} {tool_word} · {} tokens · {} model · {duration}{truncation}\n",
                self.theme.muted("╰─"),
                format_count(tokens),
                self.theme.muted(&model_time),
                truncation = if truncated == 0 {
                    String::new()
                } else {
                    format!("  {}", self.theme.warning(&format!("{truncated} bounded")))
                },
            )
        }
    }

    fn set_message(&self, message: impl AsRef<str>) {
        self.progress
            .set_message(sanitize_terminal_text(message.as_ref()));
    }

    fn row(&self, marker: &str, label: &str, detail: &str, tone: TimelineTone) {
        for line in timeline_row(
            &self.theme,
            self.columns,
            elapsed_millis(self.started),
            marker,
            label,
            detail,
            tone,
        ) {
            self.progress.println(line);
        }
    }

    fn on_run_started(&self, progress: &RunProgress) {
        if let RunProgress::RunStarted {
            run_id,
            goal,
            model,
        } = progress
        {
            self.progress.println(format!(
                "\n  {} {}  {}",
                self.theme.brand("╭─ RUN"),
                self.theme.code(&short_run_id(*run_id)),
                self.theme
                    .muted(&truncate(model, content_width(self.columns, 26, 42))),
            ));
            for line in wrap_text(goal, content_width(self.columns, 4, 88)) {
                self.progress.println(format!(
                    "  {} {}",
                    self.theme.muted("│"),
                    self.theme.text(&line)
                ));
            }
        }
    }

    fn on_state_progress(&self, progress: &RunProgress) {
        let RunProgress::StateChanged { state } = progress else {
            return;
        };
        if let Some(message) = state_activity(*state) {
            let tone = match state {
                RunState::Completed | RunState::AwaitingApply | RunState::Applied => {
                    TimelineTone::Success
                }
                RunState::Failed => TimelineTone::Danger,
                RunState::Cancelled | RunState::Discarded => TimelineTone::Warning,
                _ => TimelineTone::Muted,
            };
            self.row(
                "◇",
                "state",
                &format!("{} · {message}", format_state(*state)),
                tone,
            );
            self.set_message(message);
        }
    }

    fn on_context_progress(&self, progress: &RunProgress) {
        let RunProgress::ContextBuilt {
            indexed_files,
            cited_files,
            rendered_bytes,
            truncated,
            duration_ms,
            ..
        } = progress
        else {
            return;
        };
        let bounded = if *truncated { " · bounded" } else { "" };
        self.row(
            "◆",
            "context",
            &format!(
                "{indexed_files} files · {cited_files} cited · {} · {}{bounded}",
                format_bytes(u64::try_from(*rendered_bytes).unwrap_or(u64::MAX)),
                format_duration(Duration::from_millis(*duration_ms))
            ),
            TimelineTone::Brand,
        );
        self.set_message("context ready");
    }

    fn on_model_progress(&self, progress: &RunProgress) {
        match progress {
            RunProgress::ModelTurnStarted { turn, max_turns } => {
                self.turn.store(*turn, Ordering::Relaxed);
                self.set_message(format!("turn {turn}/{max_turns} · asking {}", self.model));
            }
            RunProgress::ModelTurnCompleted {
                turn,
                tool_calls,
                duration_ms,
                input_tokens,
                output_tokens,
                ..
            } => {
                let tokens = input_tokens.saturating_add(*output_tokens);
                self.model_tokens.fetch_add(tokens, Ordering::Relaxed);
                self.model_time_ms
                    .fetch_add(*duration_ms, Ordering::Relaxed);
                let duration = format_duration(Duration::from_millis(*duration_ms));
                let result = if *tool_calls == 0 {
                    "answer".to_owned()
                } else {
                    format!("{tool_calls} {}", plural(*tool_calls, "action", "actions"))
                };
                self.row(
                    "●",
                    "model",
                    &format!(
                        "turn {turn} · {result} · {} tokens · {duration}",
                        format_count(tokens)
                    ),
                    TimelineTone::Accent,
                );
                self.set_message(format!("turn {turn} complete · {result}"));
            }
            _ => {}
        }
    }

    fn on_tool_progress(&self, progress: &RunProgress) {
        match progress {
            RunProgress::ToolStarted { name } => {
                self.tool_calls.fetch_add(1, Ordering::Relaxed);
                self.set_message(format!(
                    "turn {} · {}",
                    self.turn.load(Ordering::Relaxed),
                    tool_activity(&sanitize_terminal_text(name))
                ));
            }
            RunProgress::ToolCompleted {
                name,
                succeeded,
                changed_files,
                duration_ms,
                output_bytes,
                truncated,
                ..
            } => {
                let turn = self.turn.load(Ordering::Relaxed);
                if *truncated {
                    self.truncated_outputs.fetch_add(1, Ordering::Relaxed);
                }
                let duration = format_duration(Duration::from_millis(*duration_ms));
                let (marker, detail, tone) = if let Some(path) = changed_files.first() {
                    (
                        "◆",
                        format!("{name} · changed {} · {duration}", truncate(path, 48)),
                        TimelineTone::Success,
                    )
                } else if *succeeded {
                    (
                        "●",
                        format!(
                            "{name} · {} · {duration}{}",
                            format_bytes(u64::try_from(*output_bytes).unwrap_or(u64::MAX)),
                            if *truncated { " · bounded" } else { "" }
                        ),
                        TimelineTone::Normal,
                    )
                } else {
                    (
                        "!",
                        format!("{name} · rejected · {duration}"),
                        TimelineTone::Warning,
                    )
                };
                self.row(marker, "tool", &detail, tone);
                self.set_message(format!("turn {turn} · {name} complete"));
            }
            _ => {}
        }
    }

    fn on_verification_progress(&self, progress: &RunProgress) {
        match progress {
            RunProgress::VerificationStarted { commands } => {
                let detail = if *commands == 0 {
                    "no deterministic commands detected".to_owned()
                } else {
                    format!(
                        "{commands} {} detected",
                        plural(*commands, "check", "checks")
                    )
                };
                self.row("◆", "verify", &detail, TimelineTone::Brand);
                self.set_message("grading available evidence");
            }
            RunProgress::VerificationCommandStarted {
                description,
                index,
                total,
            } => self.set_message(format!(
                "verifying {index}/{total} \u{00b7} {}",
                truncate(description, 52)
            )),
            RunProgress::VerificationCommandCompleted {
                description,
                succeeded,
                duration_ms,
            } => {
                let detail = format!(
                    "{} · {} · {}",
                    truncate(description, 52),
                    if *succeeded { "passed" } else { "inconclusive" },
                    format_duration(Duration::from_millis(*duration_ms))
                );
                self.row(
                    if *succeeded { "✓" } else { "!" },
                    "verify",
                    &detail,
                    if *succeeded {
                        TimelineTone::Success
                    } else {
                        TimelineTone::Warning
                    },
                );
                self.set_message("sealing verification evidence");
            }
            _ => {}
        }
    }

    fn on_recovery_progress(&self, progress: &RunProgress) {
        match progress {
            RunProgress::RecoveryStarted {
                repeated_turns,
                reason,
            } => {
                self.row(
                    "↻",
                    "recover",
                    &format!("{repeated_turns} repeated turns · {reason}"),
                    TimelineTone::Warning,
                );
                self.set_message("forcing an evidence-bounded answer");
            }
            RunProgress::RecoveryCompleted {
                text_bytes,
                duration_ms,
            } => {
                self.row(
                    "✓",
                    "recover",
                    &format!(
                        "answer synthesized · {} · {}",
                        format_bytes(u64::try_from(*text_bytes).unwrap_or(u64::MAX)),
                        format_duration(Duration::from_millis(*duration_ms))
                    ),
                    TimelineTone::Success,
                );
                self.set_message("recovery complete");
            }
            _ => {}
        }
    }
}

impl RunObserver for RunActivity {
    fn on_progress(&self, progress: &RunProgress) {
        match progress {
            RunProgress::RunStarted { .. } => self.on_run_started(progress),
            RunProgress::StateChanged { .. } => self.on_state_progress(progress),
            RunProgress::ContextBuilt { .. } => self.on_context_progress(progress),
            RunProgress::ModelTurnStarted { .. } | RunProgress::ModelTurnCompleted { .. } => {
                self.on_model_progress(progress);
            }
            RunProgress::ToolStarted { .. } | RunProgress::ToolCompleted { .. } => {
                self.on_tool_progress(progress);
            }
            RunProgress::RecoveryStarted { .. } | RunProgress::RecoveryCompleted { .. } => {
                self.on_recovery_progress(progress);
            }
            RunProgress::VerificationStarted { .. }
            | RunProgress::VerificationCommandStarted { .. }
            | RunProgress::VerificationCommandCompleted { .. } => {
                self.on_verification_progress(progress);
            }
            _ => {}
        }
    }
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
            let prompt = SessionPrompt::new(self.settings.effective_model(), self.pending_runs);
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
            "/help" | "/?" => self.render_help(arguments)?,
            "/status" | "/settings" | "/config" => self.render_status()?,
            "/doctor" => commands::doctor(false)?,
            "/tools" => self.render_tools()?,
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
            "/runs" | "/history" => self.render_runs()?,
            "/trace" => self.render_trace(self.resolve_run(arguments)?)?,
            "/memory" => self.render_memories(arguments)?,
            "/remember" => self.remember(arguments)?,
            "/forget" => self.forget(arguments)?,
            "/inspect" => self.inspect_run(arguments, false)?,
            "/review" => self.inspect_run(arguments, true)?,
            "/diff" => self.render_diff(self.resolve_run(arguments)?)?,
            "/apply" => self.apply_run(self.resolve_run(arguments)?)?,
            "/discard" => self.discard_run(self.resolve_run(arguments)?)?,
            "/clear" => write_stdout("\u{1b}[2J\u{1b}[H").map_err(CliError::Output)?,
            "/quit" | "/exit" => return Ok(SessionControl::Exit),
            _ => {
                let suggestion = closest_command(command).map_or_else(String::new, |candidate| {
                    format!("; did you mean {candidate}?")
                });
                return Err(CliError::Argument(format!(
                    "unknown command {command:?}{suggestion} Use /help to browse commands"
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
        let activity = RunActivity::new(&model, self.theme.clone());
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
            request_timeout_seconds: 300,
            disable_thinking: false,
            output: OutputFormat::Human,
        };

        let result = commands::execute_run_with_observer(
            &self.workspace,
            Some(&self.state),
            args,
            &activity,
        )
        .await;
        activity.finish();

        match result {
            Ok(completed) => {
                self.emit(&activity.summary(true))?;
                self.last_run = Some(completed.receipt.run_id);
                self.refresh_pending_runs()?;
                self.render_completed(&completed)?;
            }
            Err(error) => {
                self.emit(&activity.summary(false))?;
                if let Some(run_id) = error.run_id() {
                    self.last_run = Some(run_id);
                }
                self.render_error(&error.to_string())?;
                let guidance = error.run_id().map_or_else(
                    || {
                        "Any initialized run state remains under .pactrail for diagnosis."
                            .to_owned()
                    },
                    |run_id| {
                        format!(
                            "Run {} is durable · /trace inspects it · /runs keeps it discoverable",
                            short_run_id(run_id)
                        )
                    },
                );
                self.emit(&format!("{}\n", self.theme.muted(&guidance)))?;
            }
        }
        Ok(())
    }

    fn render_banner(&self) -> Result<(), CliError> {
        let columns = terminal_columns();
        let model = self
            .settings
            .effective_model()
            .unwrap_or_else(|| "not configured".to_owned());
        let process = banner_process(self.settings.allow_process);
        let review = banner_review(self.pending_runs);
        let memory = banner_memory(self.memory_count);
        let mut lines = vec![format!(
            "  {}  {}",
            self.theme.brand("╭─ P A C T R A I L"),
            self.theme.muted(&format!("v{}", env!("CARGO_PKG_VERSION")))
        )];
        lines.extend(frame_note(
            &self.theme,
            columns,
            "verification-native coding · every change carries evidence",
        ));
        lines.push(format!("  {}", self.theme.muted("├─")));
        lines.extend(frame_field(
            &self.theme,
            columns,
            "workspace",
            &display_path(&self.workspace),
            TimelineTone::Normal,
        ));
        lines.extend(frame_field(
            &self.theme,
            columns,
            "runtime",
            &format!("{} · {}", model, provider_label(self.settings.provider)),
            TimelineTone::Accent,
        ));
        lines.extend(frame_field(
            &self.theme,
            columns,
            "safety",
            &process.0,
            process.1,
        ));
        lines.extend(frame_field(
            &self.theme,
            columns,
            "trace",
            "live timeline · durable hash chain · /trace",
            TimelineTone::Brand,
        ));
        lines.extend(frame_field(
            &self.theme,
            columns,
            "review",
            &review.0,
            review.1,
        ));
        lines.extend(frame_field(
            &self.theme,
            columns,
            "memory",
            &memory.0,
            memory.1,
        ));
        let footer = if columns < 76 {
            "Task · /help · // escapes /"
        } else {
            "Describe a task · /help commands · // escapes a leading slash"
        };
        lines.push(format!(
            "  {}  {}",
            self.theme.muted("╰─"),
            self.theme.muted(footer)
        ));
        self.emit(&format!("\n{}\n", lines.join("\n")))?;
        if self.settings.effective_model().is_none() {
            self.emit(&format!(
                "\n  {}\n  {}\n\n",
                self.theme.warning("No model configured."),
                self.theme.muted(
                    "Use /models for local Ollama, or /connect <url> <model> for any compatible API."
                )
            ))
        } else {
            self.emit("\n")
        }
    }

    fn render_help(&self, topic: &str) -> Result<(), CliError> {
        let columns = terminal_columns();
        if !topic.is_empty() {
            let requested = format!("/{}", topic.trim_start_matches('/'));
            let command = COMMANDS
                .iter()
                .find(|command| command.name() == requested)
                .ok_or_else(|| {
                    let suggestion = closest_command(&requested)
                        .map_or_else(String::new, |candidate| format!("; try /help {candidate}"));
                    CliError::Argument(format!("no help for {topic:?}{suggestion}"))
                })?;
            return self.emit(&format!(
                "\n{}\n  {}\n  {}\n\n",
                self.theme.heading(command.name()),
                self.theme.code(command.usage),
                self.theme.text(command.description),
            ));
        }

        let mut lines = vec![self.theme.heading("Command palette")];
        for group in HELP_GROUPS {
            lines.push(String::new());
            lines.push(self.theme.muted(&group.to_uppercase()));
            lines.extend(
                COMMANDS
                    .iter()
                    .filter(|command| command.group == *group)
                    .flat_map(|command| {
                        command_lines(&self.theme, columns, command.usage, command.description)
                    }),
            );
        }
        lines.push(String::new());
        lines.extend(
            wrap_text(
                "Tab completes commands · arrows browse history · Ctrl-R searches · Ctrl-C cancels input · Ctrl-D exits",
                content_width(columns, 2, 96),
            )
            .into_iter()
            .map(|line| self.theme.muted(&line)),
        );
        self.emit(&format!("\n{}\n\n", lines.join("\n")))
    }

    fn render_tools(&self) -> Result<(), CliError> {
        let columns = terminal_columns();
        let descriptors = builtin_registry()?.descriptors();
        let parallel_reads = descriptors
            .iter()
            .filter(|tool| tool.annotations.read_only && tool.annotations.parallel_safe)
            .count();
        let mut lines = vec![format!(
            "{}  {}",
            self.theme.heading("Tool kernel"),
            self.theme.muted(&format!(
                "{} typed contracts · {parallel_reads} parallel-safe reads",
                descriptors.len()
            ))
        )];
        for descriptor in &descriptors {
            lines.extend(render_tool_descriptor(&self.theme, columns, descriptor));
        }
        lines.push(String::new());
        lines.extend(
            wrap_text(
                "Every call is schema-validated, capability-gated, output-bounded, and recorded in /trace.",
                content_width(columns, 2, 96),
            )
            .into_iter()
            .map(|line| self.theme.muted(&line)),
        );
        self.emit(&format!("\n{}\n\n", lines.join("\n")))
    }

    fn render_status(&self) -> Result<(), CliError> {
        let columns = terminal_columns();
        let model = self
            .settings
            .effective_model()
            .unwrap_or_else(|| "not configured".to_owned());
        let endpoint =
            provider_base_url(&self.settings).unwrap_or_else(|| "not configured".to_owned());
        let process = if self.settings.allow_process {
            (
                "trusted · host/network/secrets available".to_owned(),
                TimelineTone::Warning,
            )
        } else {
            ("blocked".to_owned(), TimelineTone::Success)
        };
        let key = if self.settings.provider == ProviderKind::Ollama {
            ("not required for Ollama".to_owned(), TimelineTone::Muted)
        } else if std::env::var(&self.settings.api_key_env).is_ok_and(|api_key| !api_key.is_empty())
        {
            (
                format!("{} is set", self.settings.api_key_env),
                TimelineTone::Success,
            )
        } else if self
            .settings
            .effective_base_url()
            .as_deref()
            .is_some_and(is_loopback_url)
        {
            (
                format!(
                    "{} is not set (optional for local endpoints)",
                    self.settings.api_key_env
                ),
                TimelineTone::Muted,
            )
        } else {
            (
                format!("{} is not set", self.settings.api_key_env),
                TimelineTone::Warning,
            )
        };
        let review = if self.pending_runs == 0 {
            ("none waiting".to_owned(), TimelineTone::Muted)
        } else {
            (
                format!(
                    "{} {} waiting",
                    self.pending_runs,
                    plural(self.pending_runs, "candidate", "candidates")
                ),
                TimelineTone::Warning,
            )
        };
        let memory = if self.memory_count == 0 {
            ("empty".to_owned(), TimelineTone::Muted)
        } else {
            (
                format!(
                    "{} active {}",
                    self.memory_count,
                    plural(self.memory_count, "entry", "entries")
                ),
                TimelineTone::Accent,
            )
        };
        let fields = [
            (
                "workspace",
                display_path(&self.workspace),
                TimelineTone::Normal,
            ),
            (
                "runtime",
                format!("{} · {}", model, provider_label(self.settings.provider)),
                TimelineTone::Accent,
            ),
            ("endpoint", endpoint, TimelineTone::Normal),
            ("credential", key.0, key.1),
            (
                "limits",
                format!(
                    "{} context · {} output · {} turns",
                    format_count(self.settings.context_tokens),
                    format_count(self.settings.max_output_tokens),
                    self.settings.max_turns
                ),
                TimelineTone::Normal,
            ),
            ("processes", process.0, process.1),
            ("review", review.0, review.1),
            ("memory", memory.0, memory.1),
        ];
        let mut lines = vec![self.theme.heading("Session")];
        for (label, value, tone) in fields {
            lines.extend(labelled_rows(&self.theme, columns, label, &value, tone));
        }
        self.emit(&format!("\n{}\n\n", lines.join("\n")))
    }

    fn render_memories(&self, query: &str) -> Result<(), CliError> {
        let columns = terminal_columns();
        let memories = if query.trim().is_empty() {
            commands::list_memories(&self.state, 20)?
        } else {
            commands::search_memories(&self.state, query, 20)?
                .into_iter()
                .map(|item| item.memory)
                .collect()
        };
        if memories.is_empty() {
            return self.emit(&format!(
                "\n{}\n  {}\n\n",
                self.theme.heading("Workspace memory"),
                self.theme.muted(if query.trim().is_empty() {
                    "No memories yet. Use /remember <text> to add one."
                } else {
                    "No memory matched that query."
                })
            ));
        }
        let mut lines = vec![format!(
            "{}  {}",
            self.theme.heading("Workspace memory"),
            self.theme.muted(&format!(
                "{} {}{}",
                memories.len(),
                plural(memories.len(), "entry", "entries"),
                if query.trim().is_empty() {
                    String::new()
                } else {
                    format!(" for {query:?}")
                }
            ))
        )];
        for memory in memories {
            let marker = match memory.kind {
                MemoryKind::Convention => self.theme.accent("◆"),
                MemoryKind::Decision => self.theme.success("●"),
                MemoryKind::Warning => self.theme.warning("!"),
                MemoryKind::AppliedRun => self.theme.muted("✓"),
            };
            lines.push(format!(
                "  {marker} {}  {}",
                self.theme.muted(&memory.kind.to_string()),
                self.theme
                    .text(&truncate(&memory.title, content_width(columns, 20, 62)))
            ));
            lines.push(format!(
                "      {}  {}",
                self.theme.muted("id"),
                self.theme.code(&memory.id.to_string())
            ));
            for content in wrapped_preview(
                &memory.content.replace('\n', " "),
                content_width(columns, 6, 78),
                2,
            ) {
                lines.push(format!("      {}", self.theme.muted(&content)));
            }
        }
        lines.push(self.theme.muted(
            "Memory is advisory and provenance-tagged; current files and task instructions win.",
        ));
        self.emit(&format!("\n{}\n\n", lines.join("\n")))
    }

    fn remember(&mut self, argument: &str) -> Result<(), CliError> {
        let argument = argument.trim();
        if argument.is_empty() {
            return Err(CliError::Argument(
                "usage: /remember [convention|decision|warning] <text>".to_owned(),
            ));
        }
        let (first, remaining) = argument
            .split_once(char::is_whitespace)
            .unwrap_or((argument, ""));
        let (kind, content) = match first.to_ascii_lowercase().as_str() {
            "convention" => (MemoryKind::Convention, remaining.trim()),
            "decision" => (MemoryKind::Decision, remaining.trim()),
            "warning" => (MemoryKind::Warning, remaining.trim()),
            _ => (MemoryKind::Convention, argument),
        };
        if content.is_empty() {
            return Err(CliError::Argument(
                "memory content cannot be empty".to_owned(),
            ));
        }
        let title = content
            .split_whitespace()
            .take(10)
            .collect::<Vec<_>>()
            .join(" ");
        let memory = commands::remember_memory(
            &self.state,
            MemoryDraft {
                kind,
                title,
                content: content.to_owned(),
                tags: Vec::new(),
            },
        )?;
        self.refresh_memory_count()?;
        self.emit(&format!(
            "\n{} Remembered for this workspace\n  {}  {}\n\n",
            self.theme.success("✓"),
            self.theme.code(&memory.id.to_string()[..8]),
            self.theme.text(&memory.title)
        ))
    }

    fn forget(&mut self, argument: &str) -> Result<(), CliError> {
        if argument.trim().is_empty() {
            return Err(CliError::Argument("usage: /forget <memory-id>".to_owned()));
        }
        let id = commands::resolve_memory_id(&self.state, argument.trim())?;
        commands::forget_memory(&self.state, id)?;
        self.refresh_memory_count()?;
        self.emit(&format!(
            "\n{} Forgot workspace memory {}\n\n",
            self.theme.warning("◇"),
            self.theme.code(&id.to_string()[..8])
        ))
    }

    async fn refresh_models(&mut self) -> Result<(), CliError> {
        let spinner = ProgressBar::new_spinner();
        spinner.enable_steady_tick(Duration::from_millis(100));
        spinner.set_message("discovering models");
        let models = available_models(&self.settings).await;
        spinner.finish_and_clear();
        self.known_models = match models {
            Ok(models) => models,
            Err(ModelListError::Provider(404, _)) => {
                return self.emit(&format!(
                    "{}\n{}\n",
                    self.theme
                        .warning("This endpoint does not expose model discovery."),
                    self.theme.muted(
                        "Your configured model is unchanged. Select a known identifier with /model <name>."
                    )
                ));
            }
            Err(error) => return Err(CliError::Argument(error.to_string())),
        };
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
        let selected = match argument.parse::<usize>() {
            Ok(index) => index
                .checked_sub(1)
                .and_then(|index| self.known_models.get(index))
                .cloned()
                .ok_or_else(|| {
                    CliError::Argument(format!(
                        "model selection {argument} is unavailable; use /models to refresh choices"
                    ))
                })?,
            Err(_) => argument.to_owned(),
        };
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
        settings.base_url = base_url.or_else(|| {
            (provider == ProviderKind::OpenAiCompatible)
                .then(|| self.settings.base_url.clone())
                .flatten()
        });
        if provider == ProviderKind::OpenAiCompatible && settings.base_url.is_none() {
            return Err(CliError::Argument(
                "open-ai-compatible requires a base URL; use /provider open-ai-compatible <base-url> or /connect <base-url> <model>"
                    .to_owned(),
            ));
        }
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
        let columns = terminal_columns();
        let runs = commands::run_history(&self.state)?;
        if runs.is_empty() {
            return self.emit(&format!(
                "{}\n",
                self.theme.muted("No durable runs in this workspace.")
            ));
        }
        let mut lines = vec![format!(
            "{}  {}",
            self.theme.heading("Run history"),
            self.theme.muted(&format!(
                "{} total · {} waiting",
                runs.len(),
                self.pending_runs
            ))
        )];
        let run_ids = runs
            .iter()
            .map(|run| run.run_id.to_string())
            .collect::<Vec<_>>();
        for run in runs.iter().take(12) {
            let marker = if self.last_run == Some(run.run_id) {
                self.theme.accent("●")
            } else {
                self.theme.muted("○")
            };
            let status = run.outcome.map_or_else(
                || run_state_text(&self.theme, run.state),
                |outcome| outcome_text(&self.theme, outcome),
            );
            lines.push(format!(
                "  {marker} {}  {}  {} {}",
                self.theme
                    .code(&unique_id_prefix(&run.run_id.to_string(), &run_ids)),
                status,
                run.changes,
                plural(run.changes, "file", "files"),
            ));
            lines.extend(
                wrapped_preview(&run.goal, content_width(columns, 6, 82), 2)
                    .into_iter()
                    .map(|line| format!("      {}", self.theme.text(&line))),
            );
        }
        lines.extend(
            wrap_text(
                "Commands accept a full run ID or the unique prefix shown above.",
                content_width(columns, 2, 96),
            )
            .into_iter()
            .map(|line| self.theme.muted(&line)),
        );
        self.emit(&format!("\n{}\n\n", lines.join("\n")))
    }

    fn render_trace(&self, run_id: RunId) -> Result<(), CliError> {
        let columns = terminal_columns();
        let events = commands::load_trace(&self.state, run_id)?;
        if events.is_empty() {
            return self.emit(&format!(
                "\n{}\n  {}\n\n",
                self.theme.heading("Execution trace"),
                self.theme.muted("No durable events found for this run.")
            ));
        }
        let started = &events[0];
        let action_count = events
            .iter()
            .filter(|event| matches!(&event.event, RunEvent::ActionCompleted(_)))
            .count();
        let evidence_count = events
            .iter()
            .filter(|event| matches!(&event.event, RunEvent::EvidenceRecorded(_)))
            .count();
        let terminal_state = events.iter().rev().find_map(|event| match &event.event {
            RunEvent::StateChanged { to, .. } => Some(*to),
            _ => None,
        });
        let elapsed_ms = events.last().map_or(0, |event| {
            u64::try_from(
                (event.timestamp - started.timestamp)
                    .whole_milliseconds()
                    .max(0),
            )
            .unwrap_or(u64::MAX)
        });
        let mut lines = vec![
            format!(
                "{} {}",
                self.theme.brand("╭─ EXECUTION TRACE"),
                self.theme.code(&short_run_id(run_id)),
            ),
            format!(
                "{} state · {}",
                self.theme.muted("│"),
                terminal_state.map_or_else(
                    || self.theme.muted("unknown"),
                    |state| run_state_text(&self.theme, state)
                )
            ),
            format!(
                "{} {} events · {action_count} actions · {evidence_count} evidence · {}",
                self.theme.muted("│"),
                events.len(),
                trace_duration(elapsed_ms)
            ),
            format!(
                "{} {}",
                self.theme.muted("╰─"),
                self.theme.success("BLAKE3 hash chain verified")
            ),
            String::new(),
        ];
        let legend = "◆ context · ● model · ● tool · ↻ recover · ✓ evidence · ◇ state";
        lines.extend(
            wrap_text(legend, content_width(columns, 2, 96))
                .into_iter()
                .map(|line| format!("  {}", self.theme.muted(&line))),
        );
        lines.push(String::new());
        for envelope in &events {
            lines.extend(render_trace_event(&self.theme, columns, started, envelope));
        }
        lines.push(String::new());
        lines.extend(
            wrap_text(
                &format!(
                    "Portable JSONL · {}",
                    display_path(&commands::run_root(&self.state, run_id).join("trace.jsonl"))
                ),
                content_width(columns, 2, 110),
            )
            .into_iter()
            .map(|line| self.theme.muted(&line)),
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
        let diff = render_receipt_diff(&run_root, &receipt)
            .map_err(|error| CliError::Argument(format!("diff failed: {error}")))?;
        let (bytes_added, bytes_removed) = change_bytes(&receipt);
        let mut output = format!(
            "\n{}  {}\n",
            self.theme.heading("Diff"),
            self.theme.muted(&format!(
                "{} {} · {} added · {} removed",
                receipt.changes.len(),
                plural(receipt.changes.len(), "file", "files"),
                format_bytes(bytes_added),
                format_bytes(bytes_removed)
            ))
        );
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
        self.refresh_pending_runs()?;
        self.refresh_memory_count()?;
        self.emit(&format!(
            "\n{} Applied {} {}\n  {}\n\n",
            self.theme.success("✓"),
            receipt.changes.len(),
            plural(receipt.changes.len(), "file", "files"),
            self.theme
                .text(&display_path_text(&receipt.contract.workspace_root))
        ))
    }

    fn discard_run(&mut self, run_id: RunId) -> Result<(), CliError> {
        commands::discard_run(&self.state, run_id)?;
        self.last_run = Some(run_id);
        self.refresh_pending_runs()?;
        self.emit(&format!(
            "\n{} Candidate discarded\n  {}\n  {}\n\n",
            self.theme.warning("◇"),
            self.theme.code(&run_id.to_string()),
            self.theme
                .muted("Receipt and immutable review evidence were retained.")
        ))
    }

    fn render_completed(&self, completed: &CompletedRun) -> Result<(), CliError> {
        self.render_receipt(&completed.receipt)?;
        let summary = if completed.model_summary.trim().is_empty() {
            "(model returned no summary)"
        } else {
            completed.model_summary.trim()
        };
        let tokens = format_count(completed.tokens);
        let report_title = if completed.receipt.outcome == ReceiptOutcome::Answered {
            "Answer"
        } else {
            "Model report"
        };
        self.emit(&format!(
            "{} {}\n{}\n\n{} {tokens} tokens  {}  {}\n",
            self.theme.accent("◆"),
            self.theme.heading(report_title),
            self.theme.text(summary),
            self.theme.muted("usage"),
            self.theme.code("/trace full timeline"),
            self.theme.code("/runs history"),
        ))?;
        if completed.receipt.outcome == ReceiptOutcome::ReadyToApply {
            let message = if completed.receipt.changes.is_empty() {
                self.theme
                    .warning("No file changes were produced. Nothing needs applying.")
            } else {
                format!(
                    "{}  {}  {}  {}",
                    self.theme.muted("next"),
                    self.theme.code("/diff review"),
                    self.theme.code("/apply land"),
                    self.theme.code("/discard reject")
                )
            };
            self.emit(&format!("{message}\n\n"))?;
        }
        Ok(())
    }

    fn render_receipt(&self, receipt: &ChangeReceipt) -> Result<(), CliError> {
        let columns = terminal_columns();
        let integrity = receipt.verify_integrity()?;
        let (bytes_added, bytes_removed) = change_bytes(receipt);
        let mut lines = vec![format!(
            "{} {}  {}",
            self.theme.muted("╭─"),
            outcome_text(&self.theme, receipt.outcome),
            self.theme.code(&receipt.run_id.to_string())
        )];
        for (index, line) in wrap_text(&receipt.contract.goal, content_width(columns, 13, 88))
            .iter()
            .enumerate()
        {
            lines.push(format!(
                "{} {} {}",
                self.theme.muted("│"),
                self.theme
                    .muted(if index == 0 { "goal     " } else { "         " }),
                self.theme.text(line)
            ));
        }
        if receipt.outcome != ReceiptOutcome::Answered {
            lines.push(format!(
                "{} {} {} {} · {} added · {} removed",
                self.theme.muted("│"),
                self.theme.muted("candidate"),
                receipt.changes.len(),
                plural(receipt.changes.len(), "file", "files"),
                format_bytes(bytes_added),
                format_bytes(bytes_removed),
            ));
        }
        lines.push(format!(
            "{} {} {}",
            self.theme.muted("│"),
            self.theme.muted("evidence "),
            evidence_summary(&self.theme, receipt)
        ));
        lines.push(format!(
            "{} {} {}",
            self.theme.muted("╰─"),
            self.theme.muted("receipt integrity"),
            if integrity {
                self.theme.success("verified")
            } else {
                self.theme.danger("INVALID")
            }
        ));

        if receipt.outcome != ReceiptOutcome::Answered {
            lines.push(String::new());
            lines.push(self.theme.heading("Changes"));
            if receipt.changes.is_empty() {
                lines.push(format!("  {}", self.theme.muted("(none)")));
            }
            for change in &receipt.changes {
                let path_width = content_width(columns, 24, 62);
                let path = format!(
                    "{:<width$}",
                    truncate(&change.path, path_width),
                    width = path_width
                );
                lines.push(format!(
                    "  {}  {} {}",
                    change_marker(&self.theme, change),
                    self.theme.code(&path),
                    self.theme.muted(&change_delta(change)),
                ));
            }
        }

        if !receipt.unresolved_risks.is_empty() {
            lines.push(String::new());
            lines.push(self.theme.heading("Review notes"));
            for risk in &receipt.unresolved_risks {
                for (index, line) in wrap_text(risk, content_width(columns, 6, 86))
                    .iter()
                    .enumerate()
                {
                    lines.push(format!(
                        "  {} {}",
                        self.theme.warning(if index == 0 { "!" } else { " " }),
                        self.theme.text(line)
                    ));
                }
            }
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
        let matches = commands::run_history(&self.state)?
            .into_iter()
            .map(|run| run.run_id)
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

    fn refresh_pending_runs(&mut self) -> Result<(), CliError> {
        let receipts = commands::completed_runs(&self.state)?;
        let (last_run, pending_runs) = run_focus(&receipts);
        self.last_run = last_run;
        self.pending_runs = pending_runs;
        Ok(())
    }

    fn refresh_memory_count(&mut self) -> Result<(), CliError> {
        self.memory_count = commands::list_memories(&self.state, 100)?.len();
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

fn state_activity(state: RunState) -> Option<String> {
    match state {
        RunState::Contracting => Some("validating task contract".to_owned()),
        RunState::Investigating => Some("indexing repository and instructions".to_owned()),
        RunState::Planning => Some("assembling model context".to_owned()),
        RunState::Executing => Some("starting model loop".to_owned()),
        RunState::Verifying => Some("detecting repository checks".to_owned()),
        RunState::Reviewing => Some("sealing evidence receipt".to_owned()),
        RunState::Completed => Some("answer sealed with an integrity receipt".to_owned()),
        RunState::AwaitingApply => Some("candidate ready for review".to_owned()),
        RunState::Failed => Some("run failed".to_owned()),
        RunState::Created | RunState::Applied | RunState::Discarded | RunState::Cancelled => None,
    }
}

fn tool_activity(name: &str) -> &'static str {
    match name {
        "list_files" => "mapping workspace files",
        "read_file" => "reading source",
        "read_many_files" => "reading related sources",
        "search" => "searching workspace",
        "recall_memory" => "recalling workspace memory",
        "write_file" => "writing candidate file",
        "replace_text" => "editing candidate file",
        "edit_file" => "applying atomic candidate edits",
        "remove_file" => "removing candidate file",
        "workspace_changes" => "inspecting candidate changes",
        "run_process" => "running trusted process",
        _ => "running typed tool",
    }
}

fn plural(value: usize, singular: &'static str, plural: &'static str) -> &'static str {
    if value == 1 { singular } else { plural }
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs_f64();
    if seconds < 60.0 {
        format!("{seconds:.1}s")
    } else {
        format!(
            "{}m {:02}s",
            duration.as_secs() / 60,
            duration.as_secs() % 60
        )
    }
}

fn render_tool_descriptor(theme: &Theme, columns: usize, tool: &ToolDescriptor) -> Vec<String> {
    let (marker, risk) = match tool.annotations.risk {
        ToolRisk::ReadOnly => (
            theme.success("◇"),
            theme.success(&format!("{:<11}", "read")),
        ),
        ToolRisk::WorkspaceMutation => {
            (theme.accent("◆"), theme.accent(&format!("{:<11}", "edit")))
        }
        ToolRisk::HostExecution => (
            theme.warning("!"),
            theme.warning(&format!("{:<11}", "host")),
        ),
    };
    let flags = [
        tool.annotations.read_only.then_some("read-only"),
        tool.annotations.idempotent.then_some("idempotent"),
        tool.annotations.parallel_safe.then_some("parallel-safe"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let mut lines = if columns < 72 {
        vec![
            format!("\n  {marker} {}", theme.code(&tool.name)),
            format!(
                "     {risk} {}",
                theme.muted(&tool.required_capability.to_string())
            ),
        ]
    } else {
        vec![format!(
            "\n  {marker} {}  {risk} {}",
            theme.code(&format!("{:<20}", tool.name)),
            theme.muted(&tool.required_capability.to_string())
        )]
    };
    for text in wrap_text(&tool.description, content_width(columns, 5, 76)) {
        lines.push(format!("     {}", theme.text(&text)));
    }
    if !flags.is_empty() {
        lines.extend(
            wrap_text(&flags, content_width(columns, 5, 76))
                .into_iter()
                .map(|line| format!("     {}", theme.muted(&line))),
        );
    }
    lines
}

fn render_trace_event(
    theme: &Theme,
    columns: usize,
    started: &EventEnvelope,
    envelope: &EventEnvelope,
) -> Vec<String> {
    let elapsed = envelope.timestamp - started.timestamp;
    let elapsed_ms = u64::try_from(elapsed.whole_milliseconds().max(0)).unwrap_or(u64::MAX);
    let time = theme.muted(&format!("{:>7}", trace_duration(elapsed_ms)));
    let sequence = theme.muted(&format!("#{:03}", envelope.sequence));
    match &envelope.event {
        RunEvent::ContractRegistered(contract) => trace_detail_rows(
            theme,
            columns,
            &time,
            &sequence,
            "◆",
            &format!("contract · {}", contract.goal),
            TimelineTone::Normal,
        ),
        RunEvent::StateChanged { from, to } => trace_detail_rows(
            theme,
            columns,
            &time,
            &sequence,
            "◇",
            &format!("state · {from:?} → {to:?}"),
            TimelineTone::Muted,
        ),
        RunEvent::ActionCompleted(action) => {
            render_trace_action(theme, columns, &time, &sequence, action)
        }
        RunEvent::EvidenceRecorded(evidence) => trace_detail_rows(
            theme,
            columns,
            &time,
            &sequence,
            "✓",
            &format!(
                "evidence · {:?}/{:?} · {}",
                evidence.grade, evidence.status, evidence.summary
            ),
            TimelineTone::Normal,
        ),
        RunEvent::PolicyEvaluated(decision) => trace_detail_rows(
            theme,
            columns,
            &time,
            &sequence,
            "!",
            &format!("policy · {decision:?}"),
            TimelineTone::Warning,
        ),
        RunEvent::CheckpointCreated { checkpoint } => trace_detail_rows(
            theme,
            columns,
            &time,
            &sequence,
            "•",
            &format!("checkpoint · {checkpoint}"),
            TimelineTone::Muted,
        ),
        RunEvent::NoteRecorded { message } => trace_detail_rows(
            theme,
            columns,
            &time,
            &sequence,
            "•",
            &format!("note · {message}"),
            TimelineTone::Muted,
        ),
    }
}

fn render_trace_action(
    theme: &Theme,
    columns: usize,
    time: &str,
    sequence: &str,
    action: &ActionRecord,
) -> Vec<String> {
    let (marker, label) = if action.action == "recover_read_only_answer" {
        (
            theme.warning("↻"),
            theme.warning(&format!("{:<8}", "recover")),
        )
    } else if action.actor == "context" {
        (theme.brand("◆"), theme.brand(&format!("{:<8}", "context")))
    } else if action.actor.starts_with("model:") {
        (theme.brand("●"), theme.accent(&format!("{:<8}", "model")))
    } else if action.actor.starts_with("tool:") {
        (theme.accent("●"), theme.text(&format!("{:<8}", "tool")))
    } else if action.actor == "verifier" {
        (
            theme.success("●"),
            theme.success(&format!("{:<8}", "verify")),
        )
    } else {
        (theme.muted("●"), theme.text(&format!("{:<8}", "action")))
    };
    let outcome = if action.succeeded {
        theme.success("ok")
    } else {
        theme.danger("failed")
    };
    let mut lines = vec![format!(
        "  {time} {sequence}  {marker} {label} {}  {outcome}",
        theme.muted(&trace_duration(action.duration_ms))
    )];
    let detail_width = content_width(columns, 18, 88);
    for summary in wrap_text(&action.summary, detail_width) {
        lines.push(format!("                  {}", theme.text(&summary)));
    }
    if !action.attributes.is_empty() {
        let attributes = action
            .attributes
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("  ");
        for text in wrap_text(&attributes, detail_width) {
            lines.push(format!("                  {}", theme.muted(&text)));
        }
    }
    if !action.observed_effects.is_empty() {
        let effects = format!("effects · {}", action.observed_effects.join(", "));
        for text in wrap_text(&effects, detail_width) {
            lines.push(format!("                  {}", theme.muted(&text)));
        }
    }
    lines
}

fn trace_duration(milliseconds: u64) -> String {
    if milliseconds < 1_000 {
        format!("{milliseconds}ms")
    } else if milliseconds < 60_000 {
        format!(
            "{}.{:02}s",
            milliseconds / 1_000,
            (milliseconds % 1_000) / 10
        )
    } else {
        format!(
            "{}m {:02}s",
            milliseconds / 60_000,
            (milliseconds / 1_000) % 60
        )
    }
}

fn trace_detail_rows(
    theme: &Theme,
    columns: usize,
    time: &str,
    sequence: &str,
    marker: &str,
    detail: &str,
    tone: TimelineTone,
) -> Vec<String> {
    let marker = match marker {
        "◆" => theme.accent(marker),
        "✓" => theme.success(marker),
        "!" => theme.warning(marker),
        _ => theme.muted(marker),
    };
    wrap_text(detail, content_width(columns, 18, 88))
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                format!("  {time} {sequence}  {marker} {}", tone.paint(theme, &line))
            } else {
                format!("                  {}", tone.paint(theme, &line))
            }
        })
        .collect()
}

enum SessionControl {
    Continue,
    Exit,
}

struct SessionPrompt {
    right: String,
}

impl SessionPrompt {
    fn new(model: Option<String>, pending_runs: usize) -> Self {
        let model = truncate(&model.unwrap_or_else(|| "no model".to_owned()), 32);
        Self {
            right: if pending_runs == 0 {
                model
            } else {
                format!("{pending_runs} review · {model}")
            },
        }
    }
}

impl Prompt for SessionPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Borrowed("pactrail")
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.right)
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
    parse_model_list(&serde_json::from_slice(&bytes)?)
}

fn parse_model_list(value: &Value) -> Result<Vec<String>, ModelListError> {
    let entries = value
        .get("data")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if entries.len() > MAX_DISCOVERED_MODELS {
        return Err(ModelListError::TooManyModels);
    }
    let mut models = Vec::with_capacity(entries.len());
    for model in entries
        .iter()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str))
    {
        if model.trim().is_empty()
            || model.len() > MAX_DISCOVERED_MODEL_BYTES
            || model.chars().any(char::is_control)
        {
            return Err(ModelListError::InvalidModelIdentifier);
        }
        models.push(model.to_owned());
    }
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

fn is_loopback_url(value: &str) -> bool {
    Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        })
}

fn display_path(path: &Path) -> String {
    display_path_text(&path.display().to_string())
}

fn display_path_text(value: &str) -> String {
    value
        .strip_prefix("\\\\?\\UNC\\")
        .map(|path| format!("\\\\{path}"))
        .or_else(|| value.strip_prefix("\\\\?\\").map(str::to_owned))
        .unwrap_or_else(|| value.to_owned())
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
        ReceiptOutcome::Answered => theme.success("ANSWERED"),
        ReceiptOutcome::ReadyToApply => theme.success("READY TO APPLY"),
        ReceiptOutcome::Applied => theme.success("APPLIED"),
        ReceiptOutcome::Discarded => theme.warning("DISCARDED"),
        ReceiptOutcome::Failed => theme.danger("FAILED"),
        ReceiptOutcome::Cancelled => theme.warning("CANCELLED"),
    }
}

fn run_state_text(theme: &Theme, state: RunState) -> String {
    match state {
        RunState::Failed => theme.danger("FAILED"),
        RunState::Cancelled => theme.warning("CANCELLED"),
        RunState::Applied => theme.success("APPLIED"),
        RunState::Discarded => theme.warning("DISCARDED"),
        RunState::Completed => theme.success("ANSWERED"),
        RunState::AwaitingApply => theme.success("READY TO APPLY"),
        _ => theme.muted(&format!("{state:?}").to_uppercase()),
    }
}

fn split_command(line: &str) -> (&str, &str) {
    line.split_once(char::is_whitespace)
        .map_or((line, ""), |(command, arguments)| {
            (command, arguments.trim())
        })
}

fn terminal_columns() -> usize {
    terminal_size()
        .map(|(Width(columns), _)| usize::from(columns))
        .or_else(|| {
            std::env::var("COLUMNS")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
        })
        .unwrap_or(DEFAULT_TERMINAL_COLUMNS)
        .clamp(MIN_TERMINAL_COLUMNS, MAX_TERMINAL_COLUMNS)
}

fn content_width(columns: usize, prefix: usize, maximum: usize) -> usize {
    columns.saturating_sub(prefix).clamp(12, maximum)
}

fn banner_process(enabled: bool) -> (String, TimelineTone) {
    if enabled {
        (
            "isolated edits · native processes trusted".to_owned(),
            TimelineTone::Warning,
        )
    } else {
        (
            "isolated edits · native processes blocked".to_owned(),
            TimelineTone::Success,
        )
    }
}

fn banner_review(pending: usize) -> (String, TimelineTone) {
    if pending == 0 {
        ("no candidates waiting".to_owned(), TimelineTone::Muted)
    } else {
        (
            format!(
                "{pending} {} waiting · /review",
                plural(pending, "candidate", "candidates")
            ),
            TimelineTone::Warning,
        )
    }
}

fn banner_memory(count: usize) -> (String, TimelineTone) {
    if count == 0 {
        (
            "empty · /remember to teach this workspace".to_owned(),
            TimelineTone::Muted,
        )
    } else {
        (
            format!("{count} {} · /memory", plural(count, "memory", "memories")),
            TimelineTone::Accent,
        )
    }
}

fn frame_field(
    theme: &Theme,
    columns: usize,
    label: &str,
    value: &str,
    tone: TimelineTone,
) -> Vec<String> {
    wrap_text(value, content_width(columns, 16, 120))
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            format!(
                "  {} {} {}",
                theme.muted("│"),
                theme.muted(&format!("{:<10}", if index == 0 { label } else { "" })),
                tone.paint(theme, &line)
            )
        })
        .collect()
}

fn frame_note(theme: &Theme, columns: usize, value: &str) -> Vec<String> {
    wrap_text(value, content_width(columns, 5, 120))
        .into_iter()
        .map(|line| format!("  {}  {}", theme.muted("│"), theme.muted(&line)))
        .collect()
}

fn labelled_rows(
    theme: &Theme,
    columns: usize,
    label: &str,
    value: &str,
    tone: TimelineTone,
) -> Vec<String> {
    wrap_text(value, content_width(columns, 16, 120))
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            format!(
                "  {} {}",
                theme.muted(&format!("{:<12}", if index == 0 { label } else { "" })),
                tone.paint(theme, &line)
            )
        })
        .collect()
}

fn command_lines(theme: &Theme, columns: usize, command: &str, description: &str) -> Vec<String> {
    if columns < 72 {
        let mut lines = vec![format!("  {}", theme.code(command))];
        lines.extend(
            wrap_text(description, content_width(columns, 4, 88))
                .into_iter()
                .map(|line| format!("    {}", theme.muted(&line))),
        );
        lines
    } else {
        wrap_text(description, content_width(columns, 33, 88))
            .into_iter()
            .enumerate()
            .map(|(index, line)| {
                let command = if index == 0 { command } else { "" };
                format!(
                    "  {} {}",
                    theme.code(&format!("{command:<29}")),
                    theme.muted(&line)
                )
            })
            .collect()
    }
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
    run_id.to_string().chars().take(13).collect()
}

fn unique_id_prefix(value: &str, candidates: &[String]) -> String {
    for length in 8..=value.chars().count() {
        let prefix = value.chars().take(length).collect::<String>();
        if candidates
            .iter()
            .filter(|candidate| candidate.starts_with(&prefix))
            .count()
            == 1
        {
            return prefix;
        }
    }
    value.to_owned()
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    if max_chars == 0 {
        return String::new();
    }
    let prefix = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    format!("{prefix}\u{2026}")
}

fn run_focus(receipts: &[ChangeReceipt]) -> (Option<RunId>, usize) {
    let pending_runs = receipts
        .iter()
        .filter(|receipt| is_pending(receipt))
        .count();
    let focused = receipts
        .iter()
        .rev()
        .find(|receipt| is_pending(receipt))
        .or_else(|| receipts.last())
        .map(|receipt| receipt.run_id);
    (focused, pending_runs)
}

fn is_pending(receipt: &ChangeReceipt) -> bool {
    receipt.outcome == ReceiptOutcome::ReadyToApply && !receipt.changes.is_empty()
}

fn closest_command(value: &str) -> Option<&'static str> {
    COMMANDS
        .iter()
        .map(|command| (edit_distance(value, command.name()), command.name()))
        .filter(|(distance, _)| *distance <= 3)
        .min_by_key(|(distance, command)| (*distance, command.len().abs_diff(value.len())))
        .map(|(_, command)| command)
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_character) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_character) in right.iter().enumerate() {
            let substitution =
                previous[right_index] + usize::from(left_character != *right_character);
            current[right_index + 1] = (current[right_index] + 1)
                .min(previous[right_index + 1] + 1)
                .min(substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

fn wrap_text(value: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in value.split_whitespace() {
        let mut remaining = word;
        loop {
            let line_length = line.chars().count();
            let separator = usize::from(!line.is_empty());
            let capacity = width.saturating_sub(line_length + separator);
            if remaining.chars().count() <= capacity {
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(remaining);
                break;
            }
            if capacity == 0 || !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                continue;
            }
            let split = remaining
                .char_indices()
                .nth(width)
                .map_or(remaining.len(), |(index, _)| index);
            let (chunk, rest) = remaining.split_at(split);
            lines.push(chunk.to_owned());
            remaining = rest;
            if remaining.is_empty() {
                break;
            }
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

fn wrapped_preview(value: &str, width: usize, max_lines: usize) -> Vec<String> {
    if max_lines == 0 {
        return Vec::new();
    }
    let width = width.max(1);
    let mut lines = wrap_text(value, width);
    if lines.len() <= max_lines {
        return lines;
    }
    lines.truncate(max_lines);
    let last = lines
        .last_mut()
        .unwrap_or_else(|| unreachable!("non-zero preview always retains one line"));
    let prefix = last
        .chars()
        .take(width.saturating_sub(1))
        .collect::<String>();
    *last = format!("{}…", prefix.trim_end());
    lines
}

fn change_bytes(receipt: &ChangeReceipt) -> (u64, u64) {
    receipt
        .changes
        .iter()
        .fold((0, 0), |(added, removed), change| {
            (
                added.saturating_add(change.bytes_added),
                removed.saturating_add(change.bytes_removed),
            )
        })
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format_binary_unit(bytes, 1024, "KiB")
    } else {
        format_binary_unit(bytes, 1024 * 1024, "MiB")
    }
}

fn format_binary_unit(bytes: u64, unit: u64, suffix: &str) -> String {
    let tenths = u128::from(bytes) * 10 / u128::from(unit);
    format!("{}.{:01} {suffix}", tenths / 10, tenths % 10)
}

fn change_marker(theme: &Theme, change: &FileChange) -> String {
    match (&change.before_digest, &change.after_digest) {
        (None, Some(_)) => theme.addition("A"),
        (Some(_), None) => theme.deletion("D"),
        _ => theme.accent("M"),
    }
}

fn change_delta(change: &FileChange) -> String {
    if change.bytes_added == 0 && change.bytes_removed == 0 {
        "mode changed".to_owned()
    } else {
        format!(
            "+{}  -{}",
            format_bytes(change.bytes_added),
            format_bytes(change.bytes_removed)
        )
    }
}

fn evidence_summary(theme: &Theme, receipt: &ChangeReceipt) -> String {
    let summary = format!(
        "{} passed · {} failed · {} inconclusive · {} skipped",
        receipt.verification.passed,
        receipt.verification.failed,
        receipt.verification.inconclusive,
        receipt.verification.skipped,
    );
    if receipt.verification.failed > 0 {
        theme.danger(&summary)
    } else if receipt.verification.inconclusive > 0 {
        theme.warning(&summary)
    } else {
        theme.success(&summary)
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
    #[error("model endpoint returned more than {MAX_DISCOVERED_MODELS} model entries")]
    TooManyModels,
    #[error("model endpoint returned an empty, oversized, or control-character model identifier")]
    InvalidModelIdentifier,
    #[error("model endpoint returned invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use pactrail_core::{Evidence, EvidenceKind, FileChange, ReceiptInput, TaskContract};

    use super::*;

    fn receipt(outcome: ReceiptOutcome, has_change: bool) -> ChangeReceipt {
        let contract = TaskContract::new("test task", ".");
        let evidence =
            Evidence::deterministic_pass(contract.obligations[0].id, EvidenceKind::Test, "passed");
        ChangeReceipt::build(ReceiptInput {
            run_id: RunId::new(),
            contract,
            outcome,
            baseline_digest: "baseline".to_owned(),
            final_event_hash: "event".to_owned(),
            changes: has_change
                .then(|| FileChange {
                    path: "src/lib.rs".to_owned(),
                    before_digest: None,
                    after_digest: Some("after".to_owned()),
                    before_unix_mode: None,
                    after_unix_mode: None,
                    bytes_added: 2_048,
                    bytes_removed: 0,
                })
                .into_iter()
                .collect(),
            evidence: vec![evidence],
            unresolved_risks: Vec::new(),
        })
        .unwrap_or_else(|error| unreachable!("receipt: {error}"))
    }

    #[test]
    fn endpoints_reject_remote_http_and_url_credentials() {
        assert!(validate_base_url("http://example.com/v1").is_err());
        assert!(validate_base_url("https://user:pass@example.com/v1").is_err());
        assert!(validate_base_url("http://127.0.0.1:8080/v1").is_ok());
        assert!(is_loopback_url("http://localhost:8080/v1"));
        assert!(!is_loopback_url("https://models.example.com/v1"));
    }

    #[test]
    fn model_discovery_is_bounded_and_rejects_unsafe_identifiers() {
        let models = parse_model_list(&serde_json::json!({
            "data": [
                {"id": "coder-b"},
                {"id": "coder-a"},
                {"id": "coder-b"},
                {"not_an_id": true}
            ]
        }))
        .unwrap_or_else(|error| unreachable!("model list: {error}"));
        assert_eq!(models, ["coder-a", "coder-b"]);
        assert!(matches!(
            parse_model_list(&serde_json::json!({"data": [{"id": "bad\u{001b}[2J"}]})),
            Err(ModelListError::InvalidModelIdentifier)
        ));

        let entries = (0..=MAX_DISCOVERED_MODELS)
            .map(|index| serde_json::json!({"id": format!("model-{index}")}))
            .collect::<Vec<_>>();
        assert!(matches!(
            parse_model_list(&serde_json::json!({"data": entries})),
            Err(ModelListError::TooManyModels)
        ));
    }

    #[test]
    fn counts_and_text_truncation_are_stable() {
        assert_eq!(format_count(1_234_567), "1,234,567");
        assert_eq!(truncate("abcdef", 3), "ab\u{2026}");
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(format_bytes(2_048), "2.0 KiB");
        assert_eq!(
            display_path_text(r"\\?\C:\Users\aarya\project"),
            r"C:\Users\aarya\project"
        );
        let ids = [
            "019f6b15-ac0a-7000-8000-000000000001".to_owned(),
            "019f6b15-ac0a-7000-8000-000000000002".to_owned(),
        ];
        assert_eq!(
            unique_id_prefix(&ids[0], &ids),
            "019f6b15-ac0a-7000-8000-000000000001"
        );
        assert_eq!(
            short_run_id(
                ids[0]
                    .parse::<RunId>()
                    .unwrap_or_else(|error| unreachable!("run id: {error}"))
            ),
            "019f6b15-ac0a"
        );
    }

    #[test]
    fn command_help_corrects_typos_and_wraps_review_text() {
        assert_eq!(closest_command("/modle"), Some("/model"));
        assert_eq!(closest_command("/unrelated"), None);
        assert_eq!(wrap_text("one two three", 7), ["one two", "three"]);
        assert_eq!(
            wrapped_preview("one two three four five", 7, 2),
            ["one two", "three…"]
        );
        assert_eq!(
            wrap_text("https://example.com/a/very/long/path", 10),
            ["https://ex", "ample.com/", "a/very/lon", "g/path"]
        );
    }

    #[test]
    fn pending_candidate_is_focused_over_a_newer_terminal_run() {
        let pending = receipt(ReceiptOutcome::ReadyToApply, true);
        let pending_id = pending.run_id;
        let applied = receipt(ReceiptOutcome::Applied, true);

        assert_eq!(run_focus(&[pending, applied]), (Some(pending_id), 1));
    }

    #[test]
    fn recovery_turn_has_a_distinct_trace_lane() {
        let action = ActionRecord {
            actor: "model:test/tiny".to_owned(),
            action: "recover_read_only_answer".to_owned(),
            summary: "bounded recovery produced an answer".to_owned(),
            declared_effects: Vec::new(),
            observed_effects: Vec::new(),
            succeeded: true,
            duration_ms: 12,
            attributes: std::collections::BTreeMap::new(),
        };

        let rendered =
            render_trace_action(&Theme::plain(), 80, "  12ms", "#008", &action).join("\n");

        assert!(rendered.contains("#008  ↻ recover"));
        assert!(rendered.contains("bounded recovery produced an answer"));
    }

    #[test]
    fn live_timeline_neutralizes_untrusted_terminal_controls() {
        let rendered = timeline_row(
            &Theme::plain(),
            80,
            12,
            "●",
            "tool",
            "read_file\u{1b}[2J · src/lib.rs",
            TimelineTone::Normal,
        )
        .join("\n");
        let activity = RunActivity::new("model\u{1b}[2J", Theme::plain());

        assert!(!rendered.contains('\u{1b}'));
        assert!(rendered.contains("read_file\u{fffd}[2J"));
        assert!(!activity.model.contains('\u{1b}'));
    }

    #[test]
    fn structured_views_fit_a_narrow_terminal() {
        let theme = Theme::plain();
        let columns = 60;
        let timeline = timeline_row(
            &theme,
            columns,
            12,
            "●",
            "tool",
            "read_file · https://example.com/a/very/long/path/that/must/remain/visible",
            TimelineTone::Normal,
        );
        let fields = frame_field(
            &theme,
            columns,
            "workspace",
            r"C:\Users\builder\a-very-long-workspace-name\project",
            TimelineTone::Normal,
        );
        let commands = command_lines(
            &theme,
            columns,
            "/output-tokens <tokens>",
            "set the maximum model output budget per turn",
        );
        let tool = builtin_registry()
            .unwrap_or_else(|error| unreachable!("tools: {error}"))
            .descriptors()
            .into_iter()
            .find(|tool| tool.name == "list_files")
            .unwrap_or_else(|| unreachable!("list_files descriptor"));
        let tools = render_tool_descriptor(&theme, columns, &tool);

        for line in timeline
            .iter()
            .chain(&fields)
            .chain(&commands)
            .chain(&tools)
            .flat_map(|line| line.lines())
        {
            assert!(
                line.chars().count() <= columns,
                "line exceeded {columns} columns: {line:?}"
            );
        }
        assert!(timeline.join("").contains("remain/visible"));
    }
}
