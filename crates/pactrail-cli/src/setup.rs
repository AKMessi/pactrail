//! Explicit guided onboarding. Keys never pass through task history or settings TOML.
use std::collections::BTreeSet;
use std::io::IsTerminal;
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use reedline::Signal;
use secrecy::{ExposeSecret, SecretString};

use crate::cli::ProviderKind;
use crate::commands::CliError;
use crate::settings::{InteractiveSettings, SettingsStore};
use crate::theme::Theme;

const PREFIX: &str = "PACTRAIL_SAVED_";

struct ProviderOption {
    name: &'static str,
    aliases: &'static [&'static str],
    group: &'static str,
    provider: ProviderKind,
    endpoint: &'static str,
    env: &'static str,
    local_port: Option<u16>,
    custom_endpoint: bool,
}

// Numbers are stable across releases: 1-7 retain their historical meaning.
const PROVIDERS: &[ProviderOption] = &[
    ProviderOption {
        name: "OpenRouter",
        aliases: &["openrouter", "open-router"],
        group: "Hosted providers",
        provider: ProviderKind::OpenAiCompatible,
        endpoint: "https://openrouter.ai/api/v1",
        env: "OPENROUTER_API_KEY",
        local_port: None,
        custom_endpoint: false,
    },
    ProviderOption {
        name: "Ollama",
        aliases: &["ollama"],
        group: "Local runtimes",
        provider: ProviderKind::Ollama,
        endpoint: "http://127.0.0.1:11434/v1",
        env: "PACTRAIL_NO_KEY",
        local_port: Some(11434),
        custom_endpoint: false,
    },
    ProviderOption {
        name: "OpenAI",
        aliases: &["openai", "open-ai"],
        group: "Hosted providers",
        provider: ProviderKind::OpenAi,
        endpoint: "https://api.openai.com/v1",
        env: "OPENAI_API_KEY",
        local_port: None,
        custom_endpoint: false,
    },
    ProviderOption {
        name: "Anthropic",
        aliases: &["anthropic", "claude"],
        group: "Hosted providers",
        provider: ProviderKind::Anthropic,
        endpoint: "https://api.anthropic.com",
        env: "ANTHROPIC_API_KEY",
        local_port: None,
        custom_endpoint: false,
    },
    ProviderOption {
        name: "Gemini",
        aliases: &["gemini", "google"],
        group: "Hosted providers",
        provider: ProviderKind::Gemini,
        endpoint: "https://generativelanguage.googleapis.com",
        env: "GEMINI_API_KEY",
        local_port: None,
        custom_endpoint: false,
    },
    ProviderOption {
        name: "OpenAI Responses",
        aliases: &["responses", "openai-responses"],
        group: "Hosted providers",
        provider: ProviderKind::OpenAiResponses,
        endpoint: "https://api.openai.com/v1",
        env: "OPENAI_API_KEY",
        local_port: None,
        custom_endpoint: false,
    },
    ProviderOption {
        name: "Other compatible",
        aliases: &["other", "compatible", "openai-compatible"],
        group: "Custom",
        provider: ProviderKind::OpenAiCompatible,
        endpoint: "",
        env: "OPENAI_API_KEY",
        local_port: None,
        custom_endpoint: true,
    },
    ProviderOption {
        name: "LM Studio",
        aliases: &["lmstudio", "lm-studio"],
        group: "Local runtimes",
        provider: ProviderKind::OpenAiCompatible,
        endpoint: "http://127.0.0.1:1234/v1",
        env: "PACTRAIL_NO_KEY",
        local_port: Some(1234),
        custom_endpoint: false,
    },
    ProviderOption {
        name: "llama.cpp",
        aliases: &["llamacpp", "llama-cpp"],
        group: "Local runtimes",
        provider: ProviderKind::OpenAiCompatible,
        endpoint: "http://127.0.0.1:8080/v1",
        env: "PACTRAIL_NO_KEY",
        local_port: Some(8080),
        custom_endpoint: false,
    },
    ProviderOption {
        name: "vLLM",
        aliases: &["vllm"],
        group: "Local runtimes",
        provider: ProviderKind::OpenAiCompatible,
        endpoint: "http://127.0.0.1:8000/v1",
        env: "PACTRAIL_NO_KEY",
        local_port: Some(8000),
        custom_endpoint: false,
    },
];

const PROVIDER_GROUPS: &[&str] = &["Local runtimes", "Hosted providers", "Custom"];

struct ProviderDetection {
    active: bool,
    status: String,
}

fn normalize_provider(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

fn find_provider(choice: &str) -> Option<&'static ProviderOption> {
    if let Ok(index) = choice.parse::<usize>()
        && let Some(option) = index.checked_sub(1).and_then(|index| PROVIDERS.get(index))
    {
        return Some(option);
    }
    let choice = normalize_provider(choice);
    PROVIDERS.iter().find(|option| {
        normalize_provider(option.name) == choice
            || option
                .aliases
                .iter()
                .any(|alias| normalize_provider(alias) == choice)
    })
}

fn local_runtime_running(port: u16) -> bool {
    TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(150),
    )
    .is_ok()
}

fn workspace_env_names(workspace: &Path) -> BTreeSet<String> {
    dotenvy::from_path_iter(workspace.join(".env"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|(_, value)| !value.is_empty())
        .map(|(name, _)| name)
        .collect()
}

fn provider_detections(replace_key: bool, env_names: &BTreeSet<String>) -> Vec<ProviderDetection> {
    PROVIDERS
        .iter()
        .map(|option| {
            if let Some(port) = option.local_port {
                let active = local_runtime_running(port);
                return ProviderDetection {
                    active,
                    status: if active {
                        format!("detected on 127.0.0.1:{port}")
                    } else {
                        format!("not running on 127.0.0.1:{port}")
                    },
                };
            }
            if option.custom_endpoint {
                return ProviderDetection {
                    active: false,
                    status: "you provide the URL".to_owned(),
                };
            }
            let environment = std::env::var(option.env).is_ok_and(|value| !value.is_empty());
            let workspace_file = env_names.contains(option.env);
            let saved = !replace_key
                && !option.endpoint.is_empty()
                && credential(&key_id(option.endpoint)).is_ok_and(|key| key.is_some());
            ProviderDetection {
                active: environment || workspace_file || saved,
                status: if environment {
                    format!("{} found", option.env)
                } else if workspace_file {
                    format!("{} found in .env", option.env)
                } else if saved {
                    "saved key available".to_owned()
                } else {
                    format!("{} not set", option.env)
                },
            }
        })
        .collect()
}

fn recommended_provider(current: &InteractiveSettings, detections: &[ProviderDetection]) -> usize {
    if current.model.is_some()
        && let Some(index) = PROVIDERS.iter().position(|option| {
            option.provider == current.provider
                && (option.endpoint.is_empty()
                    || current.base_url.as_deref() == Some(option.endpoint))
        })
    {
        return index;
    }
    detections
        .iter()
        .position(|detection| detection.active)
        .unwrap_or(1)
}

static SESSION_KEYS: OnceLock<Mutex<std::collections::HashMap<String, SecretString>>> =
    OnceLock::new();

#[allow(clippy::needless_pass_by_value)] // Adapter consumed by Result::map_err.
fn error(message: impl ToString) -> CliError {
    CliError::Argument(message.to_string())
}
fn say(message: &str) -> Result<(), CliError> {
    crate::output::write_human_stdout(message).map_err(error)
}
fn ask(label: &str) -> Result<Option<String>, CliError> {
    say(label)?;
    match crate::ui::input::read_terminal_line().map_err(error)? {
        Signal::Success(value) if value.trim() != "/cancel" => Ok(Some(value.trim().to_owned())),
        _ => Ok(None),
    }
}
#[cfg(any(unix, windows))]
fn key_store() -> Result<crate::composer::ComposerStore, CliError> {
    let store = SettingsStore::discover().map_err(error)?;
    let path = store.settings_path();
    Ok(crate::composer::ComposerStore::new(
        path.parent()
            .ok_or_else(|| error("No configuration directory"))?,
        Path::new("pactrail-provider-credentials-v1"),
    ))
}
fn key_id(endpoint: &str) -> String {
    format!(
        "{PREFIX}{}",
        blake3::hash(endpoint.trim_end_matches('/').as_bytes()).to_hex()
    )
}

pub(crate) fn check_binding(name: &str, endpoint: &str) -> Result<(), CliError> {
    if name.starts_with(PREFIX) && name != key_id(endpoint) {
        return Err(error(
            "Saved API key belongs to a different endpoint. Run /setup to connect this endpoint explicitly.",
        ));
    }
    Ok(())
}

fn cache_key(name: &str, value: Option<SecretString>) -> Result<Option<SecretString>, CliError> {
    if let Some(key) = &value {
        SESSION_KEYS
            .get_or_init(Mutex::default)
            .lock()
            .map_err(error)?
            .insert(
                name.to_owned(),
                SecretString::from(key.expose_secret().to_owned()),
            );
    }
    Ok(value)
}

pub(crate) fn credential(name: &str) -> Result<Option<SecretString>, CliError> {
    if name == "PACTRAIL_NO_KEY" {
        return Ok(None);
    }
    if let Ok(value) = std::env::var(name)
        && !value.is_empty()
    {
        return Ok(Some(value.into()));
    }
    if let Some(value) = SESSION_KEYS
        .get_or_init(Mutex::default)
        .lock()
        .map_err(error)?
        .get(name)
    {
        return Ok(Some(SecretString::from(value.expose_secret().to_owned())));
    }
    if !name.starts_with(PREFIX) {
        return Ok(None);
    }
    #[cfg(unix)]
    {
        let value = key_store()?
            .load_private(&name.replace('_', "-"))
            .map_err(error)?;
        cache_key(name, value.map(SecretString::from))
    }
    #[cfg(windows)]
    {
        let value = key_store()?.load(&name.replace('_', "-")).map_err(error)?;
        let decrypted = value
            .map(|text| {
                let encoded = text.strip_prefix("dpapi-v1:").ok_or_else(|| {
                    error("Unrecognized Windows credential format; reconnect with /setup new-key.")
                })?;
                windows_protect(encoded, true).map(SecretString::from)
            })
            .transpose()?;
        cache_key(name, decrypted)
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(None)
    }
}

fn remember_key(name: &str, key: SecretString) -> Result<(), CliError> {
    #[cfg(unix)]
    key_store()?
        .save(&name.replace('_', "-"), key.expose_secret())
        .map_err(error)?;
    #[cfg(windows)]
    key_store()?
        .save(
            &name.replace('_', "-"),
            &format!("dpapi-v1:{}", windows_protect(key.expose_secret(), false)?),
        )
        .map_err(error)?;
    SESSION_KEYS
        .get_or_init(Mutex::default)
        .lock()
        .map_err(error)?
        .insert(name.to_owned(), key);
    Ok(())
}

#[cfg(unix)]
fn read_secret() -> Result<Option<String>, CliError> {
    use nix::sys::termios::{LocalFlags, SetArg, tcgetattr, tcsetattr};
    let stdin = std::io::stdin();
    let original = tcgetattr(&stdin).map_err(error)?;
    let mut hidden = original.clone();
    hidden.local_flags.remove(LocalFlags::ECHO);
    tcsetattr(&stdin, SetArg::TCSAFLUSH, &hidden).map_err(error)?;
    let result = crate::ui::input::read_terminal_line();
    tcsetattr(&stdin, SetArg::TCSAFLUSH, &original).map_err(error)?;
    say("\n")?;
    match result.map_err(error)? {
        Signal::Success(value) => Ok(Some(value)),
        _ => Ok(None),
    }
}

#[cfg(not(unix))]
fn read_secret() -> Result<Option<String>, CliError> {
    console::Term::stdout()
        .read_secure_line()
        .map(Some)
        .map_err(error)
}

fn forget(name: &str) -> Result<(), CliError> {
    SESSION_KEYS
        .get_or_init(Mutex::default)
        .lock()
        .map_err(error)?
        .remove(name);
    #[cfg(any(unix, windows))]
    if name.starts_with(PREFIX) {
        key_store()?.clear(&name.replace('_', "-")).map_err(error)?;
    }
    Ok(())
}

/// Windows user-scoped encryption through the system PowerShell host. Secrets travel
/// over stdin/stdout, never argv; profiles and error output are disabled.
#[cfg(windows)]
fn windows_protect(value: &str, decrypt: bool) -> Result<String, CliError> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let system = std::env::var_os("SystemRoot")
        .ok_or_else(|| error("Windows system directory is unavailable"))?;
    let executable =
        std::path::PathBuf::from(system).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let script = if decrypt {
        "$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Security; [Console]::InputEncoding=[Text.UTF8Encoding]::new(); [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); $v=[Console]::ReadLine(); $b=[Convert]::FromBase64String($v); $p=[Security.Cryptography.ProtectedData]::Unprotect($b,$null,[Security.Cryptography.DataProtectionScope]::CurrentUser); [Console]::Write([Text.Encoding]::UTF8.GetString($p))"
    } else {
        "$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Security; [Console]::InputEncoding=[Text.UTF8Encoding]::new(); [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); $v=[Console]::ReadLine(); $b=[Text.Encoding]::UTF8.GetBytes($v); $p=[Security.Cryptography.ProtectedData]::Protect($b,$null,[Security.Cryptography.DataProtectionScope]::CurrentUser); [Console]::Write([Convert]::ToBase64String($p))"
    };
    let mut child = Command::new(executable).args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()
        .map_err(|_| error("Windows credential protection is unavailable; use an API key environment variable."))?;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| error("Credential input unavailable"))?;
    input
        .write_all(value.as_bytes())
        .and_then(|()| input.write_all(b"\n"))
        .map_err(|_| error("Credential protection input failed"))?;
    drop(input);
    let output = child
        .wait_with_output()
        .map_err(|_| error("Windows credential protection failed"))?;
    if !output.status.success() || output.stdout.len() > 65536 {
        return Err(error(
            "Windows credential protection failed for this user. Reconnect with /setup new-key or use an environment key.",
        ));
    }
    String::from_utf8(output.stdout).map_err(|_| error("Invalid protected credential encoding"))
}

fn model_bucket(model: &str, preferred: Option<&str>) -> u8 {
    let lower = model.to_lowercase();
    if preferred == Some(model) {
        0
    } else if [
        "embedding",
        "embed-",
        "whisper",
        "tts",
        "moderation",
        "dall-e",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        3
    } else if ["coder", "code", "instruct", "chat"]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        1
    } else {
        2
    }
}

fn ranked_models(models: &[String], preferred: Option<&str>) -> Vec<String> {
    let mut ranked = models.to_vec();
    ranked.sort_by(|left, right| {
        model_bucket(left, preferred)
            .cmp(&model_bucket(right, preferred))
            .then_with(|| left.to_lowercase().cmp(&right.to_lowercase()))
    });
    ranked
}

fn print_provider_menu(
    theme: &Theme,
    detections: &[ProviderDetection],
    recommended: usize,
) -> Result<(), CliError> {
    let columns = crate::interactive::terminal_columns();
    say(&format!(
        "\n  {}\n  {}\n\n",
        theme.heading("Connect a model"),
        theme.muted("Guided setup · /cancel keeps your current settings")
    ))?;
    for group in PROVIDER_GROUPS {
        say(&format!("  {}\n", theme.heading(group)))?;
        for (index, option) in PROVIDERS.iter().enumerate() {
            if option.group != *group {
                continue;
            }
            let detection = &detections[index];
            let marker = if detection.active {
                theme.brand(theme.symbol("●"))
            } else {
                theme.muted(theme.symbol("○"))
            };
            let wide = columns >= 60;
            let recommended_label = if index == recommended {
                theme.brand(if wide { " · recommended" } else { " *" })
            } else {
                String::new()
            };
            if wide {
                let status =
                    crate::terminal::truncate(&detection.status, columns.saturating_sub(41));
                say(&format!(
                    "  {marker} {:>2}  {} {}{recommended_label}\n",
                    index + 1,
                    theme.text(&format!("{:<18}", option.name)),
                    if detection.active {
                        theme.text(&status)
                    } else {
                        theme.muted(&status)
                    }
                ))?;
            } else {
                say(&format!(
                    "  {marker} {:>2}  {}{recommended_label}\n",
                    index + 1,
                    theme.text(option.name)
                ))?;
            }
        }
    }
    let recommended = PROVIDERS
        .get(recommended)
        .map_or("Ollama", |option| option.name);
    say(&format!(
        "\n  Type a number or provider name. Enter selects {}.\n",
        theme.accent(recommended)
    ))?;
    Ok(())
}

fn print_model_menu(
    theme: &Theme,
    columns: usize,
    models: &[String],
    preferred: Option<&str>,
) -> Result<(), CliError> {
    let shown = models.len().min(20);
    say(&format!(
        "\n  {} {}\n",
        theme.heading("Discovered models"),
        theme.muted(&format!(
            "· {} total{}",
            models.len(),
            if models.len() > shown {
                format!(" · showing {shown}")
            } else {
                String::new()
            }
        ))
    ))?;
    for (index, model) in models.iter().take(shown).enumerate() {
        let current = preferred == Some(model);
        let suffix_width = if current { 10 } else { 0 };
        let name = crate::terminal::truncate(model, columns.saturating_sub(6 + suffix_width));
        say(&format!(
            "  {:>2}  {}{}\n",
            index + 1,
            theme.code(&name),
            if current {
                theme.brand(" · current")
            } else {
                String::new()
            }
        ))?;
    }
    say("\n")?;
    Ok(())
}

pub(crate) async fn launch(
    workspace: &Path,
    forget_key: bool,
    replace_key: bool,
) -> Result<(), CliError> {
    let store = SettingsStore::discover().map_err(error)?;
    let settings = store.load().map_err(error)?;
    if forget_key {
        forget(&settings.api_key_env)?;
        say(
            "Saved key removed for the configured endpoint. Environment variables and workspace .env files were not changed.\n",
        )?;
    } else {
        configure(workspace, &store, &settings, replace_key).await?;
    }
    Ok(())
}

struct TemporaryKey(Option<String>);
impl Drop for TemporaryKey {
    fn drop(&mut self) {
        if let Some(name) = &self.0
            && let Ok(mut keys) = SESSION_KEYS.get_or_init(Mutex::default).lock()
        {
            keys.remove(name);
        }
    }
}

/// Configuration is committed only after all steps finish; cancellation keeps existing settings.
#[allow(clippy::too_many_lines)]
pub(crate) async fn configure(
    workspace: &Path,
    store: &SettingsStore,
    current: &InteractiveSettings,
    replace_key: bool,
) -> Result<bool, CliError> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(error(
            "Setup needs a terminal. For automation use explicit `pactrail run` provider/model flags and an API key environment variable.",
        ));
    }
    if std::env::var("PACTRAIL_BASE_URL").is_ok_and(|value| !value.is_empty())
        || std::env::var("PACTRAIL_MODEL").is_ok_and(|value| !value.is_empty())
    {
        say(
            "PACTRAIL_MODEL / PACTRAIL_BASE_URL overrides are active. Unset them before guided setup so your selection can take effect.\n",
        )?;
        return Ok(false);
    }
    let theme = Theme::detect();
    let columns = crate::interactive::terminal_columns();
    let env_names = workspace_env_names(workspace);
    let detections = provider_detections(replace_key, &env_names);
    let recommended = recommended_provider(current, &detections);
    print_provider_menu(&theme, &detections, recommended)?;
    let default_name = PROVIDERS
        .get(recommended)
        .map_or("Ollama", |option| option.name);
    let option = loop {
        let Some(choice) = ask(&format!(
            "Provider [1-{}, name, Enter for {default_name}]: ",
            PROVIDERS.len()
        ))?
        else {
            return Ok(false);
        };
        if choice.is_empty() {
            break PROVIDERS.get(recommended).unwrap_or(&PROVIDERS[0]);
        }
        if let Some(option) = find_provider(&choice) {
            break option;
        }
        say(&format!(
            "  Unknown provider {choice:?}. Use a listed number or name, or /cancel.\n"
        ))?;
    };
    let mut settings = current.clone();
    settings.provider = option.provider;
    let endpoint = if option.custom_endpoint {
        let Some(url) = ask("API base URL (include /v1 if required): ")? else {
            return Ok(false);
        };
        crate::interactive::validate_base_url(&url)?;
        url
    } else {
        option.endpoint.to_owned()
    };
    settings.base_url = Some(endpoint.clone());
    settings.api_key_env = option.env.to_owned();
    let local_runtime = option.local_port.is_some();
    let mut new_key = None;
    let mut temporary_key = TemporaryKey(None);
    if local_runtime {
        "PACTRAIL_NO_KEY".clone_into(&mut settings.api_key_env);
        say(&format!(
            "  {} Local runtime selected; no API key is required.\n",
            theme.brand(theme.symbol("●"))
        ))?;
    } else {
        let saved_name = key_id(&endpoint);
        if !replace_key && credential(option.env)?.is_some() {
            say(&format!("  Using {} from the environment.\n", option.env))?;
        } else if !replace_key && credential(&saved_name)?.is_some() {
            settings.api_key_env = saved_name;
            say("  Using the previously saved key for this endpoint.\n")?;
        } else {
            // Read just the explicitly selected variable; never import repository environment wholesale.
            let dot_key = (!replace_key)
                .then(|| dotenvy::from_path_iter(workspace.join(".env")))
                .and_then(Result::ok)
                .and_then(|values| {
                    values
                        .filter_map(Result::ok)
                        .find(|(name, value)| name == option.env && !value.is_empty())
                        .map(|(_, value)| value)
                });
            let offered = if let Some(value) = dot_key {
                let Some(answer) = ask(&format!(
                    "  Found {} in this workspace's .env. Use it for {endpoint}? [Y/n]: ",
                    option.env
                ))?
                else {
                    return Ok(false);
                };
                if answer.is_empty() || answer.eq_ignore_ascii_case("y") {
                    Some(value)
                } else {
                    None
                }
            } else {
                None
            };
            let value = if let Some(value) = offered {
                value
            } else {
                let no_key_allowed = option.custom_endpoint;
                if no_key_allowed {
                    say("  Paste API key (hidden), or Enter for no authentication: ")?;
                } else {
                    say(
                        "  Paste API key (hidden; Unix private file / Windows user-encrypted). Empty cancels: ",
                    )?;
                }
                let Some(value) = read_secret()? else {
                    return Ok(false);
                };
                if value.is_empty() && !no_key_allowed {
                    return Ok(false);
                }
                value
            };
            if value.len() > 8192 || value.chars().any(char::is_control) {
                return Err(error("API key must be a single line of at most 8 KiB."));
            }
            if value.is_empty() {
                "PACTRAIL_NO_KEY".clone_into(&mut settings.api_key_env);
                say("  No API authentication selected.\n")?;
            } else {
                settings.api_key_env = saved_name.clone();
                // Session only until the complete setup is accepted.
                SESSION_KEYS
                    .get_or_init(Mutex::default)
                    .lock()
                    .map_err(error)?
                    .insert(saved_name.clone(), value.clone().into());
                temporary_key.0 = Some(saved_name.clone());
                new_key = Some((saved_name, SecretString::from(value)));
            }
        }
    }
    say(&format!(
        "\n  Looking for models at {} (up to 8 seconds, no inference)…\n\n",
        theme.code(&endpoint)
    ))?;
    let discovery = tokio::select! {
        result = crate::interactive::available_models(&settings) => result,
        signal = tokio::signal::ctrl_c() => {
            signal.map_err(error)?;
            say("Setup cancelled during model discovery. Existing settings and stored keys are unchanged.\n")?;
            return Ok(false);
        }
    };
    let models = if let Ok(models) = discovery {
        models
    } else {
        say(
            "  Model discovery was unavailable. Check the endpoint or key, then enter an exact model ID.\n  No inference or tool call has been made.\n",
        )?;
        Vec::new()
    };
    if models.is_empty() && local_runtime {
        say(&format!(
            "  No model was reported by {}. Start the runtime and load or pull a model, or enter an installed model ID now.\n",
            option.name
        ))?;
    }
    let preferred = (current.provider == settings.provider)
        .then_some(current.model.as_deref())
        .flatten();
    let ranked = ranked_models(&models, preferred);
    let selected = if ranked.len() == 1 {
        say(&format!(
            "  Using the only discovered model: {}\n",
            theme.accent(&ranked[0])
        ))?;
        ranked[0].clone()
    } else {
        if !ranked.is_empty() {
            print_model_menu(&theme, columns, &ranked, preferred)?;
        }
        loop {
            let prompt = if ranked.is_empty() {
                "Model ID or search: ".to_owned()
            } else {
                format!(
                    "Model ID or search [1-{}, number or text; Enter=1]: ",
                    ranked.len().min(20)
                )
            };
            let Some(query) = ask(&prompt)? else {
                return Ok(false);
            };
            if query.is_empty() {
                if let Some(model) = ranked.first() {
                    break model.clone();
                }
                continue;
            }
            if let Ok(index) = query.parse::<usize>()
                && let Some(model) = index.checked_sub(1).and_then(|index| ranked.get(index))
            {
                break model.clone();
            }
            if ranked.is_empty() || ranked.iter().any(|model| model == &query) {
                break query;
            }
            let matches = ranked
                .iter()
                .filter(|model| model.to_lowercase().contains(&query.to_lowercase()))
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [] => {
                    let Some(answer) =
                        ask("  Not in the catalog. Use this exact ID anyway? [y/N]: ")?
                    else {
                        return Ok(false);
                    };
                    if answer.eq_ignore_ascii_case("y") {
                        break query;
                    }
                }
                [model] => break (*model).clone(),
                matches => {
                    for (index, model) in matches.iter().take(20).enumerate() {
                        let name = crate::terminal::truncate(model, columns.saturating_sub(6));
                        say(&format!("  {:>2}  {}\n", index + 1, theme.code(&name)))?;
                    }
                    let Some(answer) = ask("  Select number, or Enter to refine the search: ")?
                    else {
                        return Ok(false);
                    };
                    if let Ok(index) = answer.parse::<usize>()
                        && let Some(model) =
                            index.checked_sub(1).and_then(|index| matches.get(index))
                    {
                        break (*model).clone();
                    }
                }
            }
        }
    };
    settings.model = Some(selected.clone());
    settings.validate().map_err(error)?;
    if let Some((name, key)) = new_key {
        store.ensure_directory().map_err(error)?;
        let config = store.settings_path();
        let directory = std::fs::canonicalize(
            config
                .parent()
                .ok_or_else(|| error("No configuration directory"))?,
        )
        .map_err(error)?;
        let workspace = std::fs::canonicalize(workspace).map_err(error)?;
        if directory.starts_with(workspace) {
            return Err(error(
                "API keys cannot be saved inside the workspace. Use an environment key or set PACTRAIL_CONFIG_DIR to a directory outside this workspace.",
            ));
        }
        remember_key(&name, key)?;
        #[cfg(unix)]
        say("  Key saved privately in your user config (0600 file; not encrypted).\n")?;
        #[cfg(windows)]
        say("  Key saved with Windows user-scoped DPAPI encryption.\n")?;
        #[cfg(not(any(unix, windows)))]
        say("  Key available only in this process; use an environment key for future sessions.\n")?;
    }
    store.save(&settings).map_err(error)?;
    temporary_key.0 = None;
    let columns = crate::interactive::terminal_columns();
    let credential = if local_runtime {
        "not required".to_owned()
    } else if settings.api_key_env == "PACTRAIL_NO_KEY" {
        "none selected".to_owned()
    } else if settings.api_key_env.starts_with(PREFIX) {
        "saved with user-scoped protection".to_owned()
    } else {
        format!("{} (environment or saved)", settings.api_key_env)
    };
    let mut body = Vec::new();
    body.extend(crate::ui::card_field(
        &theme,
        columns,
        "model",
        &selected,
        |line| theme.accent(line),
    ));
    body.extend(crate::ui::card_field(
        &theme,
        columns,
        "provider",
        option.name,
        |line| theme.text(line),
    ));
    body.extend(crate::ui::card_field(
        &theme,
        columns,
        "endpoint",
        &endpoint,
        |line| theme.text(line),
    ));
    body.extend(crate::ui::card_field(
        &theme,
        columns,
        "credential",
        &credential,
        |line| theme.text(line),
    ));
    body.push(String::new());
    for line in crate::terminal::wrap(
        "Command permissions are unchanged. Catalog discovery does not verify inference or tool support; use /probe to test one bounded turn.",
        crate::ui::card_body_width(columns),
    ) {
        body.push(theme.muted(&line));
    }
    say(&format!(
        "\n  Model configured: {}\n",
        theme.accent(&selected)
    ))?;
    let lines = crate::ui::card(&theme, columns, "Setup complete", "", body);
    say(&format!("\n{}\n\n", lines.join("\n")))?;
    say(&format!(
        "  Start a task with {}. Reconnect or replace the key with /setup.\n",
        theme.accent("pactrail")
    ))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn windows_keys_roundtrip_with_user_scoped_protection() {
        let key = "fixture-windows-secret-日本語";
        let encrypted =
            windows_protect(key, false).unwrap_or_else(|e| unreachable!("protect: {e}"));
        assert!(!encrypted.contains(key));
        assert_eq!(
            windows_protect(&encrypted, true).unwrap_or_else(|e| unreachable!("unprotect: {e}")),
            key
        );
        assert!(windows_protect("not valid ciphertext", true).is_err());
    }

    #[test]
    fn saved_keys_are_bound_to_the_endpoint() {
        let name = key_id("https://example.com/v1");
        assert!(check_binding(&name, "https://example.com/v1/").is_ok());
        assert!(check_binding(&name, "https://evil.example/v1").is_err());
        assert!(check_binding("OPENAI_API_KEY", "https://example.com/v1").is_ok());
    }

    #[test]
    fn provider_numbers_keep_their_historical_meaning() {
        let names = PROVIDERS
            .iter()
            .map(|option| option.name)
            .collect::<Vec<_>>();
        assert_eq!(
            &names[..7],
            [
                "OpenRouter",
                "Ollama",
                "OpenAI",
                "Anthropic",
                "Gemini",
                "OpenAI Responses",
                "Other compatible"
            ]
        );
        assert_eq!(&names[7..], ["LM Studio", "llama.cpp", "vLLM"]);
        assert_eq!(
            find_provider("7").map(|option| option.name),
            Some("Other compatible")
        );
        assert_eq!(find_provider("10").map(|option| option.name), Some("vLLM"));
        assert_eq!(
            find_provider("lm-studio").map(|option| option.name),
            Some("LM Studio")
        );
        assert_eq!(
            find_provider("Claude").map(|option| option.name),
            Some("Anthropic")
        );
        assert!(find_provider("0").is_none());
        assert!(find_provider("11").is_none());
        assert!(find_provider("not-a-provider").is_none());
    }

    #[test]
    fn model_ranking_prefers_current_then_coding_models() {
        let models = [
            "text-embedding-3-large",
            "vision-model",
            "general-chat",
            "qwen-coder",
            "current-model",
        ]
        .map(str::to_owned);
        let ranked = ranked_models(&models, Some("current-model"));
        assert_eq!(
            ranked,
            [
                "current-model",
                "general-chat",
                "qwen-coder",
                "vision-model",
                "text-embedding-3-large"
            ]
        );
        assert_eq!(
            ranked_models(&models, None).last().map(String::as_str),
            Some("text-embedding-3-large")
        );
    }
}
