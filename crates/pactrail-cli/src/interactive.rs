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
const HELP_GROUPS: &[&str] = &["Work", "Memory", "Model", "Safety", "Session"];
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

struct RunActivity {
    progress: ProgressBar,
    model: String,
    turn: AtomicU16,
    tool_calls: AtomicUsize,
    model_tokens: AtomicU64,
    model_time_ms: AtomicU64,
    truncated_outputs: AtomicUsize,
    started: Instant,
}

impl RunActivity {
    fn new(model: &str) -> Self {
        let progress = ProgressBar::new_spinner();
        let style =
            ProgressStyle::with_template("{spinner:.cyan}  {msg}  {elapsed_precise:.bright_black}")
                .unwrap_or_else(|_| ProgressStyle::default_spinner())
                .tick_strings(&["\u{25d0}", "\u{25d3}", "\u{25d1}", "\u{25d2}"]);
        progress.set_style(style);
        progress.set_message("starting isolated transaction");
        progress.enable_steady_tick(Duration::from_millis(90));
        Self {
            progress,
            model: truncate(model, 32),
            turn: AtomicU16::new(0),
            tool_calls: AtomicUsize::new(0),
            model_tokens: AtomicU64::new(0),
            model_time_ms: AtomicU64::new(0),
            truncated_outputs: AtomicUsize::new(0),
            started: Instant::now(),
        }
    }

    fn finish(&self) {
        self.progress.finish_and_clear();
    }

    fn summary(&self, theme: &Theme) -> String {
        let turns = self.turn.load(Ordering::Relaxed);
        let tools = self.tool_calls.load(Ordering::Relaxed);
        let turn_word = plural(turns.into(), "turn", "turns");
        let tool_word = plural(tools, "tool", "tools");
        let tokens = self.model_tokens.load(Ordering::Relaxed);
        let model_time = format_duration(Duration::from_millis(
            self.model_time_ms.load(Ordering::Relaxed),
        ));
        let elapsed = format_duration(self.started.elapsed());
        let duration = theme.muted(&elapsed);
        let truncated = self.truncated_outputs.load(Ordering::Relaxed);
        let truncation = if truncated == 0 {
            String::new()
        } else {
            format!("  {}", theme.warning(&format!("{truncated} bounded")))
        };
        format!(
            "{} {turns} {turn_word}  {tools} {tool_word}  {} tokens  {} model  {duration}{truncation}\n",
            theme.success("\u{2713}"),
            format_count(tokens),
            theme.muted(&model_time),
        )
    }

    fn set_message(&self, message: String) {
        self.progress.set_message(message);
    }

    fn on_verification_progress(&self, progress: &RunProgress) {
        match progress {
            RunProgress::VerificationStarted { commands } => self.set_message(if *commands == 0 {
                "grading available evidence".to_owned()
            } else {
                format!(
                    "preparing {commands} verification {}",
                    plural(*commands, "check", "checks")
                )
            }),
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
            } => self.set_message(format!(
                "{} \u{00b7} {} \u{00b7} {}",
                if *succeeded { "passed" } else { "inconclusive" },
                truncate(description, 52),
                format_duration(Duration::from_millis(*duration_ms))
            )),
            _ => {}
        }
    }
}

impl RunObserver for RunActivity {
    fn on_progress(&self, progress: &RunProgress) {
        match progress {
            RunProgress::StateChanged { state } => {
                if let Some(message) = state_activity(*state) {
                    self.progress.set_message(message);
                }
            }
            RunProgress::ContextBuilt {
                indexed_files,
                cited_files,
                rendered_bytes,
                truncated,
                duration_ms,
                ..
            } => {
                let bounded = if *truncated { " · bounded" } else { "" };
                self.set_message(format!(
                    "context · {indexed_files} files · {cited_files} cited · {} · {}{bounded}",
                    format_bytes(*rendered_bytes as u64),
                    format_duration(Duration::from_millis(*duration_ms))
                ));
            }
            RunProgress::ModelTurnStarted { turn, max_turns } => {
                self.turn.store(*turn, Ordering::Relaxed);
                self.set_message(format!(
                    "turn {turn}/{max_turns} \u{00b7} asking {}",
                    self.model
                ));
            }
            RunProgress::ModelTurnCompleted {
                turn,
                tool_calls,
                duration_ms,
                input_tokens,
                output_tokens,
                ..
            } => {
                self.model_tokens.fetch_add(
                    input_tokens.saturating_add(*output_tokens),
                    Ordering::Relaxed,
                );
                self.model_time_ms
                    .fetch_add(*duration_ms, Ordering::Relaxed);
                let duration = format_duration(Duration::from_millis(*duration_ms));
                self.set_message(if *tool_calls == 0 {
                    format!("turn {turn} \u{00b7} model finished in {duration}")
                } else {
                    format!(
                        "turn {turn} \u{00b7} {duration} \u{00b7} received {tool_calls} {}",
                        plural(*tool_calls, "action", "actions")
                    )
                });
            }
            RunProgress::ToolStarted { name } => {
                self.tool_calls.fetch_add(1, Ordering::Relaxed);
                self.set_message(format!(
                    "turn {} \u{00b7} {}",
                    self.turn.load(Ordering::Relaxed),
                    tool_activity(name)
                ));
            }
            RunProgress::ToolCompleted {
                name,
                succeeded,
                changed_files,
                duration_ms,
                truncated,
                ..
            } => {
                let turn = self.turn.load(Ordering::Relaxed);
                if *truncated {
                    self.truncated_outputs.fetch_add(1, Ordering::Relaxed);
                }
                let duration = format_duration(Duration::from_millis(*duration_ms));
                if let Some(path) = changed_files.first() {
                    self.set_message(format!(
                        "turn {turn} \u{00b7} changed {} \u{00b7} {duration}",
                        truncate(path, 48)
                    ));
                } else if *succeeded {
                    self.set_message(format!(
                        "turn {turn} \u{00b7} {name} complete \u{00b7} {duration}"
                    ));
                } else {
                    self.set_message(format!(
                        "turn {turn} \u{00b7} {name} rejected; steering model"
                    ));
                }
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
        let activity = RunActivity::new(&model);
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
                self.emit(&activity.summary(&self.theme))?;
                self.last_run = Some(completed.receipt.run_id);
                self.refresh_pending_runs()?;
                self.render_completed(&completed)?;
            }
            Err(error) => {
                self.render_error(&error.to_string())?;
                self.emit(&format!(
                    "{}\n",
                    self.theme
                        .muted("Any initialized run state remains under .pactrail for diagnosis.")
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
        let process = if self.settings.allow_process {
            self.theme
                .warning("isolated edits · native processes trusted")
        } else {
            self.theme
                .success("isolated edits · native processes blocked")
        };
        let review = if self.pending_runs == 0 {
            self.theme.muted("no candidates waiting")
        } else {
            self.theme.warning(&format!(
                "{} {} waiting · /review",
                self.pending_runs,
                plural(self.pending_runs, "candidate", "candidates")
            ))
        };
        let memory = if self.memory_count == 0 {
            self.theme
                .muted("empty · /remember to teach this workspace")
        } else {
            self.theme.accent(&format!(
                "{} {} · /memory",
                self.memory_count,
                plural(self.memory_count, "memory", "memories")
            ))
        };
        let banner = format!(
            "\n  {}  {}\n  {}\n\n  {:<10} {}\n  {:<10} {}\n  {:<10} {}\n  {:<10} {}\n  {:<10} {}\n\n  {}\n",
            self.theme.brand("P A C T R A I L"),
            self.theme.muted(env!("CARGO_PKG_VERSION")),
            self.theme
                .muted("verification-native coding · every change carries evidence"),
            "workspace",
            self.theme.text(&display_path(&self.workspace)),
            "model",
            self.theme.accent(&model),
            "safety",
            process,
            "review",
            review,
            "memory",
            memory,
            self.theme.muted(
                "Describe a change, or use /help. Prefix a task with // when it starts with /."
            ),
        );
        self.emit(&banner)?;
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
                    .map(|command| command_line(&self.theme, command.usage, command.description)),
            );
        }
        lines.push(String::new());
        lines.push(self.theme.muted(
            "Tab completes commands · arrows browse history · Ctrl-R searches · Ctrl-C cancels input · Ctrl-D exits",
        ));
        self.emit(&format!("\n{}\n\n", lines.join("\n")))
    }

    fn render_status(&self) -> Result<(), CliError> {
        let model = self
            .settings
            .effective_model()
            .unwrap_or_else(|| "not configured".to_owned());
        let endpoint =
            provider_base_url(&self.settings).unwrap_or_else(|| "not configured".to_owned());
        let process = if self.settings.allow_process {
            self.theme
                .warning("trusted · host/network/secrets available")
        } else {
            self.theme.success("blocked")
        };
        let key = if self.settings.provider == ProviderKind::Ollama {
            self.theme.muted("not required for Ollama")
        } else if std::env::var(&self.settings.api_key_env).is_ok_and(|api_key| !api_key.is_empty())
        {
            self.theme
                .success(&format!("{} is set", self.settings.api_key_env))
        } else if self
            .settings
            .effective_base_url()
            .as_deref()
            .is_some_and(is_loopback_url)
        {
            self.theme.muted(&format!(
                "{} is not set (optional for local endpoints)",
                self.settings.api_key_env
            ))
        } else {
            self.theme
                .warning(&format!("{} is not set", self.settings.api_key_env))
        };
        let review = if self.pending_runs == 0 {
            self.theme.muted("none waiting")
        } else {
            self.theme.warning(&format!(
                "{} {} waiting",
                self.pending_runs,
                plural(self.pending_runs, "candidate", "candidates")
            ))
        };
        let memory = if self.memory_count == 0 {
            self.theme.muted("empty")
        } else {
            self.theme.accent(&format!(
                "{} active {}",
                self.memory_count,
                plural(self.memory_count, "entry", "entries")
            ))
        };
        self.emit(&format!(
            "\n{}\n  {:<12} {}\n  {:<12} {} · {}\n  {:<12} {}\n  {:<12} {}\n  {:<12} {} context · {} output · {} turns\n  {:<12} {}\n  {:<12} {}\n  {:<12} {}\n\n",
            self.theme.heading("Session"),
            "workspace",
            self.theme.text(&display_path(&self.workspace)),
            "runtime",
            self.theme.accent(&model),
            provider_label(self.settings.provider),
            "endpoint",
            self.theme.text(&endpoint),
            "credential",
            key,
            "limits",
            format_count(self.settings.context_tokens),
            format_count(self.settings.max_output_tokens),
            self.settings.max_turns,
            "processes",
            process,
            "review",
            review,
            "memory",
            memory,
        ))
    }

    fn render_memories(&self, query: &str) -> Result<(), CliError> {
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
                "  {marker} {}  {}  {}",
                self.theme.code(&memory.id.to_string()[..8]),
                self.theme.muted(&memory.kind.to_string()),
                self.theme.text(&truncate(&memory.title, 62))
            ));
            for content in wrap_text(&memory.content.replace('\n', " "), 78)
                .into_iter()
                .take(2)
            {
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
        let receipts = commands::completed_runs(&self.state)?;
        if receipts.is_empty() {
            return self.emit(&format!(
                "{}\n",
                self.theme.muted("No completed runs in this workspace.")
            ));
        }
        let mut lines = vec![format!(
            "{}  {}",
            self.theme.heading("Run history"),
            self.theme.muted(&format!(
                "{} total · {} waiting",
                receipts.len(),
                self.pending_runs
            ))
        )];
        for receipt in receipts.iter().rev().take(12) {
            let marker = if self.last_run == Some(receipt.run_id) {
                self.theme.accent("●")
            } else {
                self.theme.muted("○")
            };
            lines.push(format!(
                "  {marker} {}  {}  {} {}  {}",
                self.theme.code(&short_run_id(receipt.run_id)),
                outcome_text(&self.theme, receipt.outcome),
                receipt.changes.len(),
                plural(receipt.changes.len(), "file", "files"),
                self.theme.text(&truncate(&receipt.contract.goal, 56)),
            ));
        }
        lines.push(
            self.theme
                .muted("Commands accept a full run ID or the unique prefix shown above."),
        );
        self.emit(&format!("\n{}\n\n", lines.join("\n")))
    }

    fn render_trace(&self, run_id: RunId) -> Result<(), CliError> {
        let events = commands::load_trace(&self.state, run_id)?;
        if events.is_empty() {
            return self.emit(&format!(
                "\n{}\n  {}\n\n",
                self.theme.heading("Execution trace"),
                self.theme.muted("No durable events found for this run.")
            ));
        }
        let started = &events[0];
        let mut lines = vec![format!(
            "{}  {}",
            self.theme.heading("Execution trace"),
            self.theme.muted(&format!(
                "{} · {} events · hash chain verified",
                short_run_id(run_id),
                events.len()
            ))
        )];
        for envelope in &events {
            lines.extend(render_trace_event(&self.theme, started, envelope));
        }
        lines.push(self.theme.muted(&format!(
            "Portable JSONL · {}",
            commands::run_root(&self.state, run_id)
                .join("trace.jsonl")
                .display()
        )));
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
        self.emit(&format!(
            "{}\n{}\n\n{} {tokens} tokens  {}\n",
            self.theme.heading("Model report"),
            self.theme.text(summary),
            self.theme.muted("usage"),
            self.theme.code("/trace inspect execution"),
        ))?;
        if completed.receipt.outcome == ReceiptOutcome::ReadyToApply {
            let message = if completed.receipt.changes.is_empty() {
                self.theme
                    .warning("No file changes were produced. Nothing needs applying.")
            } else {
                format!(
                    "{}  {}  {}",
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
        let integrity = receipt.verify_integrity()?;
        let (bytes_added, bytes_removed) = change_bytes(receipt);
        let mut lines = vec![format!(
            "{}  {}",
            outcome_text(&self.theme, receipt.outcome),
            self.theme.code(&receipt.run_id.to_string())
        )];
        for (index, line) in wrap_text(&receipt.contract.goal, 88).iter().enumerate() {
            lines.push(format!(
                "  {} {}",
                self.theme
                    .muted(if index == 0 { "goal     " } else { "         " }),
                self.theme.text(line)
            ));
        }
        lines.push(format!(
            "  {} {} {} · {} added · {} removed",
            self.theme.muted("candidate"),
            receipt.changes.len(),
            plural(receipt.changes.len(), "file", "files"),
            format_bytes(bytes_added),
            format_bytes(bytes_removed),
        ));
        lines.push(format!(
            "  {} {}",
            self.theme.muted("evidence "),
            evidence_summary(&self.theme, receipt)
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

        lines.push(String::new());
        lines.push(self.theme.heading("Changes"));
        if receipt.changes.is_empty() {
            lines.push(format!("  {}", self.theme.muted("(none)")));
        }
        for change in &receipt.changes {
            let path = format!("{:<62}", truncate(&change.path, 62));
            lines.push(format!(
                "  {}  {} {}",
                change_marker(&self.theme, change),
                self.theme.code(&path),
                self.theme.muted(&change_delta(change)),
            ));
        }

        if !receipt.unresolved_risks.is_empty() {
            lines.push(String::new());
            lines.push(self.theme.heading("Review notes"));
            for risk in &receipt.unresolved_risks {
                for (index, line) in wrap_text(risk, 86).iter().enumerate() {
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

fn render_trace_event(
    theme: &Theme,
    started: &EventEnvelope,
    envelope: &EventEnvelope,
) -> Vec<String> {
    let elapsed = envelope.timestamp - started.timestamp;
    let elapsed_ms = u64::try_from(elapsed.whole_milliseconds().max(0)).unwrap_or(u64::MAX);
    let time = theme.muted(&format!("{:>7}", trace_duration(elapsed_ms)));
    match &envelope.event {
        RunEvent::ContractRegistered(contract) => vec![format!(
            "  {time}  {} {}",
            theme.accent("◆"),
            theme.text(&format!("contract · {}", contract.goal))
        )],
        RunEvent::StateChanged { from, to } => vec![format!(
            "  {time}  {} {}",
            theme.muted("◇"),
            theme.muted(&format!("state · {from:?} → {to:?}"))
        )],
        RunEvent::ActionCompleted(action) => render_trace_action(theme, &time, action),
        RunEvent::EvidenceRecorded(evidence) => vec![format!(
            "  {time}  {} {}",
            theme.success("✓"),
            theme.text(&format!(
                "evidence · {:?}/{:?} · {}",
                evidence.grade, evidence.status, evidence.summary
            ))
        )],
        RunEvent::PolicyEvaluated(decision) => vec![format!(
            "  {time}  {} {}",
            theme.warning("!"),
            theme.text(&format!("policy · {decision:?}"))
        )],
        RunEvent::CheckpointCreated { checkpoint } => vec![format!(
            "  {time}  {} {}",
            theme.muted("•"),
            theme.muted(&format!("checkpoint · {checkpoint}"))
        )],
        RunEvent::NoteRecorded { message } => vec![format!(
            "  {time}  {} {}",
            theme.muted("•"),
            theme.muted(&format!("note · {message}"))
        )],
    }
}

fn render_trace_action(theme: &Theme, time: &str, action: &ActionRecord) -> Vec<String> {
    let (marker, label) = if action.actor == "context" {
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
        "  {time}  {marker} {label} {}  {outcome}",
        theme.muted(&trace_duration(action.duration_ms))
    )];
    for summary in wrap_text(&action.summary, 78).into_iter().take(3) {
        lines.push(format!("             {}", theme.text(&summary)));
    }
    if !action.attributes.is_empty() {
        let attributes = action
            .attributes
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("  ");
        for text in wrap_text(&attributes, 78).into_iter().take(3) {
            lines.push(format!("             {}", theme.muted(&text)));
        }
    }
    if !action.observed_effects.is_empty() {
        let effects = format!("effects · {}", action.observed_effects.join(", "));
        for text in wrap_text(&effects, 78).into_iter().take(2) {
            lines.push(format!("             {}", theme.muted(&text)));
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
    let command = format!("{command:<29}");
    format!("  {} {}", theme.code(&command), theme.muted(description))
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
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in value.split_whitespace() {
        let separator = usize::from(!line.is_empty());
        if !line.is_empty() && line.chars().count() + separator + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
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
    fn counts_and_text_truncation_are_stable() {
        assert_eq!(format_count(1_234_567), "1,234,567");
        assert_eq!(truncate("abcdef", 3), "abc\u{2026}");
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(format_bytes(2_048), "2.0 KiB");
        assert_eq!(
            display_path_text(r"\\?\C:\Users\aarya\project"),
            r"C:\Users\aarya\project"
        );
    }

    #[test]
    fn command_help_corrects_typos_and_wraps_review_text() {
        assert_eq!(closest_command("/modle"), Some("/model"));
        assert_eq!(closest_command("/unrelated"), None);
        assert_eq!(wrap_text("one two three", 7), ["one two", "three"]);
    }

    #[test]
    fn pending_candidate_is_focused_over_a_newer_terminal_run() {
        let pending = receipt(ReceiptOutcome::ReadyToApply, true);
        let pending_id = pending.run_id;
        let applied = receipt(ReceiptOutcome::Applied, true);

        assert_eq!(run_focus(&[pending, applied]), (Some(pending_id), 1));
    }
}
