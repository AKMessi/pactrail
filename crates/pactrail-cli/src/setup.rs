//! Explicit guided onboarding. Keys never pass through task history or settings TOML.
use std::io::IsTerminal;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use reedline::Signal;
use secrecy::{ExposeSecret, SecretString};

use crate::cli::ProviderKind;
use crate::commands::CliError;
use crate::settings::{InteractiveSettings, SettingsStore};

const PREFIX: &str = "PACTRAIL_SAVED_";
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
    say(
        "\nConnect a model · /cancel or Ctrl-C cancels\n\n  1  OpenRouter\n  2  Ollama (local, no key)\n  3  OpenAI\n  4  Anthropic\n  5  Gemini\n  6  OpenAI Responses\n  7  Other OpenAI-compatible endpoint\n\n",
    )?;
    let Some(choice) = ask("Provider [number or name]: ")? else {
        return Ok(false);
    };
    let (provider, endpoint, env) = match choice.to_lowercase().as_str() {
        "1" | "openrouter" => (
            ProviderKind::OpenAiCompatible,
            "https://openrouter.ai/api/v1",
            "OPENROUTER_API_KEY",
        ),
        "2" | "ollama" => (ProviderKind::Ollama, "http://127.0.0.1:11434/v1", ""),
        "3" | "openai" => (
            ProviderKind::OpenAi,
            "https://api.openai.com/v1",
            "OPENAI_API_KEY",
        ),
        "4" | "anthropic" => (
            ProviderKind::Anthropic,
            "https://api.anthropic.com",
            "ANTHROPIC_API_KEY",
        ),
        "5" | "gemini" => (
            ProviderKind::Gemini,
            "https://generativelanguage.googleapis.com",
            "GEMINI_API_KEY",
        ),
        "6" | "responses" => (
            ProviderKind::OpenAiResponses,
            "https://api.openai.com/v1",
            "OPENAI_API_KEY",
        ),
        "7" | "other" | "compatible" => (ProviderKind::OpenAiCompatible, "", "OPENAI_API_KEY"),
        _ => {
            return Err(error(
                "Unknown provider. Run /setup and choose a listed name or number.",
            ));
        }
    };
    let mut settings = current.clone();
    settings.provider = provider;
    let endpoint = if endpoint.is_empty() {
        let Some(url) = ask("API base URL (include /v1 if required): ")? else {
            return Ok(false);
        };
        crate::interactive::validate_base_url(&url)?;
        url
    } else {
        endpoint.to_owned()
    };
    settings.base_url = Some(endpoint.clone());
    settings.api_key_env = if env.is_empty() {
        "OPENAI_API_KEY"
    } else {
        env
    }
    .to_owned();
    let mut new_key = None;
    let mut temporary_key = TemporaryKey(None);
    if provider != ProviderKind::Ollama {
        let saved_name = key_id(&endpoint);
        if !replace_key && credential(env)?.is_some() {
            say(&format!("Using {env} from the environment.\n"))?;
        } else if !replace_key && credential(&saved_name)?.is_some() {
            settings.api_key_env = saved_name;
            say("Using the previously saved key for this endpoint.\n")?;
        } else {
            // Read just the explicitly selected variable; never import repository environment wholesale.
            let dot_key = (!replace_key)
                .then(|| dotenvy::from_path_iter(workspace.join(".env")))
                .and_then(Result::ok)
                .and_then(|values| {
                    values
                        .filter_map(Result::ok)
                        .find(|(name, value)| name == env && !value.is_empty())
                        .map(|(_, value)| value)
                });
            let offered = if let Some(value) = dot_key {
                let Some(answer) = ask(&format!(
                    "Found {env} in this workspace's .env. Use it for {endpoint}? [Y/n]: "
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
                if provider == ProviderKind::OpenAiCompatible && env == "OPENAI_API_KEY" {
                    say(
                        "Paste API key (hidden; Unix private file / Windows user-encrypted), or Enter for no authentication: ",
                    )?;
                } else {
                    say(
                        "Paste API key (hidden; Unix private file / Windows user-encrypted). Empty cancels: ",
                    )?;
                }
                let Some(value) = read_secret()? else {
                    return Ok(false);
                };
                if value.is_empty()
                    && !(provider == ProviderKind::OpenAiCompatible && env == "OPENAI_API_KEY")
                {
                    return Ok(false);
                }
                value
            };
            if value.len() > 8192 || value.chars().any(char::is_control) {
                return Err(error("API key must be a single line of at most 8 KiB."));
            }
            if value.is_empty() {
                "PACTRAIL_NO_KEY".clone_into(&mut settings.api_key_env);
                say("No API authentication selected.\n")?;
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
    say("Looking for models (up to 8 seconds)…\n")?;
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
            "Model discovery was unavailable. Check the endpoint/key, or enter an exact model ID. No inference or tool call has been made.\n",
        )?;
        Vec::new()
    };
    if models.is_empty() && provider == ProviderKind::Ollama {
        say(
            "No local model was found. Start Ollama and pull a model, then rerun setup; or enter an installed model ID now.\n",
        )?;
    }
    let selected = loop {
        let Some(query) = ask("Model ID or search (e.g. part of a name): ")? else {
            return Ok(false);
        };
        if query.is_empty() {
            continue;
        }
        if models.iter().any(|model| model == &query) || models.is_empty() {
            break query;
        }
        let matches: Vec<_> = models
            .iter()
            .filter(|m| m.to_lowercase().contains(&query.to_lowercase()))
            .collect();
        if matches.is_empty() {
            let Some(answer) = ask("Not in the catalog. Use this exact ID anyway? [y/N]: ")? else {
                return Ok(false);
            };
            if answer.eq_ignore_ascii_case("y") {
                break query;
            }
            continue;
        }
        for (index, model) in matches.iter().take(30).enumerate() {
            say(&format!("  {}  {model}\n", index + 1))?;
        }
        let Some(answer) = ask("Select number, or Enter to refine the search: ")? else {
            return Ok(false);
        };
        if let Ok(index) = answer.parse::<usize>()
            && let Some(model) = index
                .checked_sub(1)
                .and_then(|i| matches.iter().take(30).nth(i))
        {
            break (*model).clone();
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
        say("Key saved privately in your user config (0600 file; not encrypted).\n")?;
        #[cfg(windows)]
        say("Key saved with Windows user-scoped DPAPI encryption.\n")?;
        #[cfg(not(any(unix, windows)))]
        say("Key available only in this process; use an environment key for future sessions.\n")?;
    }
    store.save(&settings).map_err(error)?;
    temporary_key.0 = None;
    say(&format!(
        "\nModel configured: {selected}\nEndpoint: {endpoint}\nCommand permissions unchanged. Catalog discovery does not verify inference or tool support.\nStart a task with `pactrail`, or reconnect with /setup.\n"
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
}
