use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::cli::{CapabilitySetting, OciRuntimeArg, ProcessBackendArg, ProviderKind};

const SETTINGS_SCHEMA: u16 = 4;
const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;
const MAX_MODEL_BYTES: usize = 512;
const MAX_BASE_URL_BYTES: usize = 2_048;
const MAX_API_KEY_ENV_BYTES: usize = 256;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InteractiveSettings {
    pub schema: u16,
    pub provider: ProviderKind,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key_env: String,
    pub context_tokens: u64,
    pub max_output_tokens: u64,
    pub max_turns: u16,
    pub streaming: bool,
    pub native_tools: CapabilitySetting,
    pub parallel_tools: CapabilitySetting,
    pub structured_output: CapabilitySetting,
    pub vision: CapabilitySetting,
    pub prompt_caching: CapabilitySetting,
    pub reasoning_controls: CapabilitySetting,
    pub process_backend: ProcessBackendArg,
    pub sandbox_runtime: OciRuntimeArg,
    pub sandbox_runtime_executable: Option<String>,
    pub sandbox_image: Option<String>,
    pub sandbox_memory_mib: u64,
    pub sandbox_cpu_millis: u32,
    pub sandbox_pids: u32,
    pub sandbox_tmpfs_mib: u64,
}

#[derive(Debug, Deserialize)]
struct SettingsHeader {
    schema: u16,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InteractiveSettingsV1 {
    schema: u16,
    provider: ProviderKind,
    model: Option<String>,
    base_url: Option<String>,
    api_key_env: String,
    context_tokens: u64,
    max_output_tokens: u64,
    max_turns: u16,
    allow_process: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InteractiveSettingsV2 {
    schema: u16,
    provider: ProviderKind,
    model: Option<String>,
    base_url: Option<String>,
    api_key_env: String,
    context_tokens: u64,
    max_output_tokens: u64,
    max_turns: u16,
    process_backend: ProcessBackendArg,
    sandbox_runtime: OciRuntimeArg,
    sandbox_runtime_executable: Option<String>,
    sandbox_image: Option<String>,
    sandbox_memory_mib: u64,
    sandbox_cpu_millis: u32,
    sandbox_pids: u32,
    sandbox_tmpfs_mib: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InteractiveSettingsV3 {
    schema: u16,
    provider: ProviderKind,
    model: Option<String>,
    base_url: Option<String>,
    api_key_env: String,
    context_tokens: u64,
    max_output_tokens: u64,
    max_turns: u16,
    streaming: bool,
    process_backend: ProcessBackendArg,
    sandbox_runtime: OciRuntimeArg,
    sandbox_runtime_executable: Option<String>,
    sandbox_image: Option<String>,
    sandbox_memory_mib: u64,
    sandbox_cpu_millis: u32,
    sandbox_pids: u32,
    sandbox_tmpfs_mib: u64,
}

impl From<InteractiveSettingsV1> for InteractiveSettings {
    fn from(legacy: InteractiveSettingsV1) -> Self {
        debug_assert_eq!(legacy.schema, 1);
        Self {
            schema: SETTINGS_SCHEMA,
            provider: legacy.provider,
            model: legacy.model,
            base_url: legacy.base_url,
            api_key_env: legacy.api_key_env,
            context_tokens: legacy.context_tokens,
            max_output_tokens: legacy.max_output_tokens,
            max_turns: legacy.max_turns,
            streaming: false,
            native_tools: CapabilitySetting::Auto,
            parallel_tools: CapabilitySetting::Auto,
            structured_output: CapabilitySetting::Auto,
            vision: CapabilitySetting::Auto,
            prompt_caching: CapabilitySetting::Auto,
            reasoning_controls: CapabilitySetting::Auto,
            process_backend: if legacy.allow_process {
                ProcessBackendArg::Native
            } else {
                ProcessBackendArg::Disabled
            },
            sandbox_runtime: OciRuntimeArg::Docker,
            sandbox_runtime_executable: None,
            sandbox_image: None,
            sandbox_memory_mib: 2_048,
            sandbox_cpu_millis: 2_000,
            sandbox_pids: 128,
            sandbox_tmpfs_mib: 512,
        }
    }
}

impl From<InteractiveSettingsV2> for InteractiveSettings {
    fn from(legacy: InteractiveSettingsV2) -> Self {
        debug_assert_eq!(legacy.schema, 2);
        Self {
            schema: SETTINGS_SCHEMA,
            provider: legacy.provider,
            model: legacy.model,
            base_url: legacy.base_url,
            api_key_env: legacy.api_key_env,
            context_tokens: legacy.context_tokens,
            max_output_tokens: legacy.max_output_tokens,
            max_turns: legacy.max_turns,
            streaming: false,
            native_tools: CapabilitySetting::Auto,
            parallel_tools: CapabilitySetting::Auto,
            structured_output: CapabilitySetting::Auto,
            vision: CapabilitySetting::Auto,
            prompt_caching: CapabilitySetting::Auto,
            reasoning_controls: CapabilitySetting::Auto,
            process_backend: legacy.process_backend,
            sandbox_runtime: legacy.sandbox_runtime,
            sandbox_runtime_executable: legacy.sandbox_runtime_executable,
            sandbox_image: legacy.sandbox_image,
            sandbox_memory_mib: legacy.sandbox_memory_mib,
            sandbox_cpu_millis: legacy.sandbox_cpu_millis,
            sandbox_pids: legacy.sandbox_pids,
            sandbox_tmpfs_mib: legacy.sandbox_tmpfs_mib,
        }
    }
}

impl From<InteractiveSettingsV3> for InteractiveSettings {
    fn from(legacy: InteractiveSettingsV3) -> Self {
        debug_assert_eq!(legacy.schema, 3);
        Self {
            schema: SETTINGS_SCHEMA,
            provider: legacy.provider,
            model: legacy.model,
            base_url: legacy.base_url,
            api_key_env: legacy.api_key_env,
            context_tokens: legacy.context_tokens,
            max_output_tokens: legacy.max_output_tokens,
            max_turns: legacy.max_turns,
            streaming: legacy.streaming,
            native_tools: CapabilitySetting::Auto,
            parallel_tools: CapabilitySetting::Auto,
            structured_output: CapabilitySetting::Auto,
            vision: CapabilitySetting::Auto,
            prompt_caching: CapabilitySetting::Auto,
            reasoning_controls: CapabilitySetting::Auto,
            process_backend: legacy.process_backend,
            sandbox_runtime: legacy.sandbox_runtime,
            sandbox_runtime_executable: legacy.sandbox_runtime_executable,
            sandbox_image: legacy.sandbox_image,
            sandbox_memory_mib: legacy.sandbox_memory_mib,
            sandbox_cpu_millis: legacy.sandbox_cpu_millis,
            sandbox_pids: legacy.sandbox_pids,
            sandbox_tmpfs_mib: legacy.sandbox_tmpfs_mib,
        }
    }
}

impl Default for InteractiveSettings {
    fn default() -> Self {
        Self {
            schema: SETTINGS_SCHEMA,
            provider: ProviderKind::Ollama,
            model: None,
            base_url: None,
            api_key_env: "OPENAI_API_KEY".to_owned(),
            context_tokens: 32_768,
            max_output_tokens: 4_096,
            max_turns: 24,
            streaming: true,
            native_tools: CapabilitySetting::Auto,
            parallel_tools: CapabilitySetting::Auto,
            structured_output: CapabilitySetting::Auto,
            vision: CapabilitySetting::Auto,
            prompt_caching: CapabilitySetting::Auto,
            reasoning_controls: CapabilitySetting::Auto,
            process_backend: ProcessBackendArg::Disabled,
            sandbox_runtime: OciRuntimeArg::Docker,
            sandbox_runtime_executable: None,
            sandbox_image: None,
            sandbox_memory_mib: 2_048,
            sandbox_cpu_millis: 2_000,
            sandbox_pids: 128,
            sandbox_tmpfs_mib: 512,
        }
    }
}

impl InteractiveSettings {
    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.schema != SETTINGS_SCHEMA {
            return Err(SettingsError::Invalid(format!(
                "unsupported settings schema {}; expected {SETTINGS_SCHEMA}",
                self.schema
            )));
        }
        if self.model.as_deref().is_some_and(|model| {
            model.trim().is_empty() || model.len() > MAX_MODEL_BYTES || contains_control(model)
        }) {
            return Err(SettingsError::Invalid(format!(
                "model must be non-empty, at most {MAX_MODEL_BYTES} bytes, and contain no control characters"
            )));
        }
        if self.base_url.as_deref().is_some_and(|url| {
            url.trim().is_empty() || url.len() > MAX_BASE_URL_BYTES || contains_control(url)
        }) {
            return Err(SettingsError::Invalid(format!(
                "base URL must be non-empty, at most {MAX_BASE_URL_BYTES} bytes, and contain no control characters"
            )));
        }
        if self.context_tokens < 1_024 || self.context_tokens > 4_194_304 {
            return Err(SettingsError::Invalid(
                "context tokens must be between 1,024 and 4,194,304".to_owned(),
            ));
        }
        if self.max_output_tokens == 0 || self.max_output_tokens > 131_072 {
            return Err(SettingsError::Invalid(
                "maximum output tokens must be between 1 and 131,072".to_owned(),
            ));
        }
        if self.max_output_tokens >= self.context_tokens {
            return Err(SettingsError::Invalid(
                "maximum output tokens must be smaller than context tokens".to_owned(),
            ));
        }
        if self.max_turns == 0 || self.max_turns > 256 {
            return Err(SettingsError::Invalid(
                "maximum turns must be between 1 and 256".to_owned(),
            ));
        }
        if !(64..=1_048_576).contains(&self.sandbox_memory_mib) {
            return Err(SettingsError::Invalid(
                "sandbox memory must be between 64 MiB and 1 TiB".to_owned(),
            ));
        }
        if !(100..=256_000).contains(&self.sandbox_cpu_millis) {
            return Err(SettingsError::Invalid(
                "sandbox CPU limit must be between 0.1 and 256 CPUs".to_owned(),
            ));
        }
        if !(16..=32_768).contains(&self.sandbox_pids) {
            return Err(SettingsError::Invalid(
                "sandbox PID limit must be between 16 and 32,768".to_owned(),
            ));
        }
        if !(1..=65_536).contains(&self.sandbox_tmpfs_mib) {
            return Err(SettingsError::Invalid(
                "sandbox temporary space must be between 1 MiB and 64 GiB".to_owned(),
            ));
        }
        if self
            .sandbox_runtime_executable
            .as_deref()
            .is_some_and(|value| {
                value.trim().is_empty() || value.len() > 4_096 || contains_control(value)
            })
        {
            return Err(SettingsError::Invalid(
                "sandbox runtime executable must be non-empty, at most 4,096 bytes, and contain no control characters"
                    .to_owned(),
            ));
        }
        if self.sandbox_image.as_deref().is_some_and(|value| {
            value.trim().is_empty()
                || value.len() > 1_024
                || value.starts_with('-')
                || contains_control(value)
        }) {
            return Err(SettingsError::Invalid(
                "sandbox image must be non-empty, at most 1,024 bytes, contain no control characters, and not begin with '-'"
                    .to_owned(),
            ));
        }
        if self.process_backend == ProcessBackendArg::Oci && self.sandbox_image.is_none() {
            return Err(SettingsError::Invalid(
                "the OCI process backend requires a sandbox image".to_owned(),
            ));
        }
        if self.api_key_env.is_empty()
            || self.api_key_env.len() > MAX_API_KEY_ENV_BYTES
            || !self
                .api_key_env
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(SettingsError::Invalid(format!(
                "API key environment variable must be at most {MAX_API_KEY_ENV_BYTES} bytes and contain only ASCII letters, numbers, and underscores"
            )));
        }
        Ok(())
    }

    #[must_use]
    pub fn effective_model(&self) -> Option<String> {
        std::env::var("PACTRAIL_MODEL")
            .ok()
            .filter(|value| {
                !value.trim().is_empty()
                    && value.len() <= MAX_MODEL_BYTES
                    && !contains_control(value)
            })
            .or_else(|| self.model.clone())
    }

    #[must_use]
    pub fn effective_base_url(&self) -> Option<String> {
        std::env::var("PACTRAIL_BASE_URL")
            .ok()
            .filter(|value| {
                !value.trim().is_empty()
                    && value.len() <= MAX_BASE_URL_BYTES
                    && !contains_control(value)
            })
            .or_else(|| self.base_url.clone())
    }
}

pub(crate) struct SettingsStore {
    directory: PathBuf,
}

impl SettingsStore {
    pub fn discover() -> Result<Self, SettingsError> {
        let base = BaseDirs::new().ok_or(SettingsError::NoConfigDirectory)?;
        Ok(Self {
            directory: base.config_dir().join("pactrail"),
        })
    }

    #[cfg(test)]
    fn at(directory: PathBuf) -> Self {
        Self { directory }
    }

    #[must_use]
    pub fn history_path(&self) -> PathBuf {
        self.directory.join("history.txt")
    }

    #[must_use]
    pub fn settings_path(&self) -> PathBuf {
        self.directory.join("settings.toml")
    }

    pub fn load(&self) -> Result<InteractiveSettings, SettingsError> {
        let path = self.settings_path();
        if !path.exists() {
            return Ok(InteractiveSettings::default());
        }
        let metadata = fs::metadata(&path).map_err(|source| SettingsError::Io {
            path: path.clone(),
            source,
        })?;
        if metadata.len() > MAX_SETTINGS_BYTES {
            return Err(SettingsError::Invalid(format!(
                "settings file exceeds {MAX_SETTINGS_BYTES} bytes"
            )));
        }
        let mut text = String::new();
        fs::File::open(&path)
            .and_then(|mut file| file.read_to_string(&mut text))
            .map_err(|source| SettingsError::Io {
                path: path.clone(),
                source,
            })?;
        let header: SettingsHeader = toml::from_str(&text).map_err(SettingsError::Toml)?;
        match header.schema {
            SETTINGS_SCHEMA => {
                let settings: InteractiveSettings =
                    toml::from_str(&text).map_err(SettingsError::Toml)?;
                settings.validate()?;
                Ok(settings)
            }
            1 => {
                let legacy: InteractiveSettingsV1 =
                    toml::from_str(&text).map_err(SettingsError::Toml)?;
                let settings = InteractiveSettings::from(legacy);
                settings.validate()?;
                self.save(&settings)?;
                Ok(settings)
            }
            2 => {
                let legacy: InteractiveSettingsV2 =
                    toml::from_str(&text).map_err(SettingsError::Toml)?;
                let settings = InteractiveSettings::from(legacy);
                settings.validate()?;
                self.save(&settings)?;
                Ok(settings)
            }
            3 => {
                let legacy: InteractiveSettingsV3 =
                    toml::from_str(&text).map_err(SettingsError::Toml)?;
                let settings = InteractiveSettings::from(legacy);
                settings.validate()?;
                self.save(&settings)?;
                Ok(settings)
            }
            schema => Err(SettingsError::Invalid(format!(
                "unsupported settings schema {schema}; expected {SETTINGS_SCHEMA}"
            ))),
        }
    }

    pub fn save(&self, settings: &InteractiveSettings) -> Result<(), SettingsError> {
        settings.validate()?;
        fs::create_dir_all(&self.directory).map_err(|source| SettingsError::Io {
            path: self.directory.clone(),
            source,
        })?;
        let text = toml::to_string_pretty(settings).map_err(SettingsError::TomlSerialize)?;
        let path = self.settings_path();
        let backup = self.directory.join("settings.toml.bak");
        let mut temporary =
            NamedTempFile::new_in(&self.directory).map_err(|source| SettingsError::Io {
                path: self.directory.clone(),
                source,
            })?;
        temporary
            .write_all(text.as_bytes())
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|source| SettingsError::Io {
                path: temporary.path().to_path_buf(),
                source,
            })?;

        recover_backup(&path, &backup)?;
        if path.exists() {
            fs::rename(&path, &backup).map_err(|source| SettingsError::Io {
                path: path.clone(),
                source,
            })?;
        }
        if let Err(error) = temporary.persist(&path) {
            let _restore = fs::rename(&backup, &path);
            return Err(SettingsError::Io {
                path,
                source: error.error,
            });
        }
        if backup.exists() {
            fs::remove_file(&backup).map_err(|source| SettingsError::Io {
                path: backup,
                source,
            })?;
        }
        Ok(())
    }

    pub fn ensure_directory(&self) -> Result<(), SettingsError> {
        fs::create_dir_all(&self.directory).map_err(|source| SettingsError::Io {
            path: self.directory.clone(),
            source,
        })
    }
}

fn recover_backup(path: &Path, backup: &Path) -> Result<(), SettingsError> {
    if !backup.exists() {
        return Ok(());
    }
    if path.exists() {
        fs::remove_file(backup).map_err(|source| SettingsError::Io {
            path: backup.to_path_buf(),
            source,
        })
    } else {
        fs::rename(backup, path).map_err(|source| SettingsError::Io {
            path: backup.to_path_buf(),
            source,
        })
    }
}

fn contains_control(value: &str) -> bool {
    value.chars().any(char::is_control)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SettingsError {
    #[error("the operating system did not provide a configuration directory")]
    NoConfigDirectory,
    #[error("invalid interactive settings: {0}")]
    Invalid(String),
    #[error("could not access settings at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not parse interactive settings: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("could not serialize interactive settings: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_atomically() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("tempdir: {error}"));
        let store = SettingsStore::at(root.path().join("config"));
        let settings = InteractiveSettings {
            model: Some("coder".to_owned()),
            ..InteractiveSettings::default()
        };
        store
            .save(&settings)
            .unwrap_or_else(|error| unreachable!("save: {error}"));
        let loaded = store
            .load()
            .unwrap_or_else(|error| unreachable!("load: {error}"));
        assert_eq!(loaded.model.as_deref(), Some("coder"));
    }

    #[test]
    fn future_settings_schema_fails_closed() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("tempdir: {error}"));
        let store = SettingsStore::at(root.path().to_path_buf());
        fs::write(
            store.settings_path(),
            "schema = 999\nprovider = \"ollama\"\napi_key_env = \"KEY\"\ncontext_tokens = 4096\nmax_output_tokens = 512\nmax_turns = 4\nallow_process = false\n",
        )
        .unwrap_or_else(|error| unreachable!("fixture: {error}"));
        assert!(matches!(store.load(), Err(SettingsError::Invalid(_))));
    }

    #[test]
    fn schema_one_process_boolean_migrates_atomically() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("tempdir: {error}"));
        let store = SettingsStore::at(root.path().to_path_buf());
        fs::write(
            store.settings_path(),
            "schema = 1\nprovider = \"ollama\"\napi_key_env = \"KEY\"\ncontext_tokens = 4096\nmax_output_tokens = 512\nmax_turns = 4\nallow_process = true\n",
        )
        .unwrap_or_else(|error| unreachable!("fixture: {error}"));

        let loaded = store
            .load()
            .unwrap_or_else(|error| unreachable!("migration: {error}"));
        assert_eq!(loaded.schema, SETTINGS_SCHEMA);
        assert_eq!(loaded.process_backend, ProcessBackendArg::Native);
        let persisted = fs::read_to_string(store.settings_path())
            .unwrap_or_else(|error| unreachable!("persisted: {error}"));
        assert!(persisted.contains("schema = 4"));
        assert!(persisted.contains("streaming = false"));
        assert!(persisted.contains("process_backend = \"native\""));
        assert!(!persisted.contains("allow_process"));
    }

    #[test]
    fn schema_two_configuration_migrates_without_silently_enabling_streaming() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("tempdir: {error}"));
        let store = SettingsStore::at(root.path().to_path_buf());
        fs::write(
            store.settings_path(),
            "schema = 2\nprovider = \"ollama\"\napi_key_env = \"KEY\"\ncontext_tokens = 4096\nmax_output_tokens = 512\nmax_turns = 4\nprocess_backend = \"disabled\"\nsandbox_runtime = \"docker\"\nsandbox_memory_mib = 2048\nsandbox_cpu_millis = 2000\nsandbox_pids = 128\nsandbox_tmpfs_mib = 512\n",
        )
        .unwrap_or_else(|error| unreachable!("fixture: {error}"));

        let loaded = store
            .load()
            .unwrap_or_else(|error| unreachable!("migration: {error}"));
        assert_eq!(loaded.schema, SETTINGS_SCHEMA);
        assert!(!loaded.streaming);
        let persisted = fs::read_to_string(store.settings_path())
            .unwrap_or_else(|error| unreachable!("persisted: {error}"));
        assert!(persisted.contains("schema = 4"));
        assert!(persisted.contains("streaming = false"));
    }

    #[test]
    fn schema_three_configuration_migrates_to_explicit_auto_capabilities() {
        let root = tempfile::tempdir().unwrap_or_else(|error| unreachable!("tempdir: {error}"));
        let store = SettingsStore::at(root.path().to_path_buf());
        fs::write(
            store.settings_path(),
            "schema = 3\nprovider = \"gemini\"\napi_key_env = \"GEMINI_API_KEY\"\ncontext_tokens = 32768\nmax_output_tokens = 4096\nmax_turns = 24\nstreaming = true\nprocess_backend = \"disabled\"\nsandbox_runtime = \"docker\"\nsandbox_memory_mib = 2048\nsandbox_cpu_millis = 2000\nsandbox_pids = 128\nsandbox_tmpfs_mib = 512\n",
        )
        .unwrap_or_else(|error| unreachable!("fixture: {error}"));

        let loaded = store
            .load()
            .unwrap_or_else(|error| unreachable!("migration: {error}"));
        assert_eq!(loaded.schema, SETTINGS_SCHEMA);
        assert_eq!(loaded.native_tools, CapabilitySetting::Auto);
        assert_eq!(loaded.parallel_tools, CapabilitySetting::Auto);
        assert!(loaded.streaming);
        let persisted = fs::read_to_string(store.settings_path())
            .unwrap_or_else(|error| unreachable!("persisted: {error}"));
        assert!(persisted.contains("schema = 4"));
        assert!(persisted.contains("native_tools = \"auto\""));
        assert!(persisted.contains("parallel_tools = \"auto\""));
    }

    #[test]
    fn output_budget_must_leave_room_for_model_input() {
        let settings = InteractiveSettings {
            context_tokens: 4_096,
            max_output_tokens: 4_096,
            ..InteractiveSettings::default()
        };

        assert!(matches!(
            settings.validate(),
            Err(SettingsError::Invalid(_))
        ));
    }
}
