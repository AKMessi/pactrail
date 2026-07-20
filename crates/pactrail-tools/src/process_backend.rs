use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const RUNTIME_PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const RUNTIME_PROBE_OUTPUT_BYTES: usize = 64 * 1024;
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(15);
const READ_CHUNK_BYTES: usize = 16 * 1024;
const CONTAINER_WORKSPACE: &str = "/workspace";
const CONTAINER_TMP: &str = "/tmp";
const RUNTIME_ENVIRONMENT_NAMES: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SystemRoot",
    "WINDIR",
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "XDG_RUNTIME_DIR",
    "DOCKER_HOST",
    "DOCKER_CONTEXT",
    "DOCKER_CONFIG",
    "CONTAINER_HOST",
    "CONTAINERS_CONF",
];

static CONTAINER_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Coarse process-execution boundary selected for a run.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessBackendKind {
    /// Process execution is unavailable.
    Disabled,
    /// Direct execution with the authority of the Pactrail host process.
    NativeTrusted,
    /// Execution through a restricted local OCI container runtime.
    OciRestricted,
}

/// Stable description recorded alongside process effects.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProcessBackendDescriptor {
    pub kind: ProcessBackendKind,
    pub strength: String,
    pub runtime: Option<String>,
    pub runtime_fingerprint: Option<String>,
    pub image: Option<String>,
    pub image_identity: Option<String>,
    pub profile_digest: String,
    pub network: String,
    pub filesystem: String,
}

impl ProcessBackendDescriptor {
    fn disabled() -> Self {
        Self {
            kind: ProcessBackendKind::Disabled,
            strength: "disabled".to_owned(),
            runtime: None,
            runtime_fingerprint: None,
            image: None,
            image_identity: None,
            profile_digest: blake3::hash(b"pactrail:process:disabled:v1")
                .to_hex()
                .to_string(),
            network: "denied".to_owned(),
            filesystem: "none".to_owned(),
        }
    }

    fn native() -> Self {
        Self {
            kind: ProcessBackendKind::NativeTrusted,
            strength: "native_trusted".to_owned(),
            runtime: None,
            runtime_fingerprint: None,
            image: None,
            image_identity: None,
            profile_digest: blake3::hash(b"pactrail:process:native-trusted:v1")
                .to_hex()
                .to_string(),
            network: "host".to_owned(),
            filesystem: "host authority; current directory is the isolated candidate".to_owned(),
        }
    }
}

/// Validated request passed to a process backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessRequest {
    pub program: String,
    pub args: Vec<String>,
    pub timeout: Duration,
    pub max_output_bytes: usize,
    pub environment: BTreeMap<String, String>,
}

/// Captured result of one backend execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessExecution {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub backend: ProcessBackendDescriptor,
}

/// Interface implemented by every process trust boundary.
#[async_trait]
pub trait ProcessBackend: Send + Sync + 'static {
    fn descriptor(&self) -> ProcessBackendDescriptor;

    async fn execute(
        &self,
        workspace: &Path,
        request: &ProcessRequest,
        cancellation: &CancellationToken,
    ) -> Result<ProcessExecution, ProcessBackendError>;
}

/// Backend that rejects every request.
#[derive(Clone, Copy, Debug, Default)]
pub struct DisabledProcessBackend;

#[async_trait]
impl ProcessBackend for DisabledProcessBackend {
    fn descriptor(&self) -> ProcessBackendDescriptor {
        ProcessBackendDescriptor::disabled()
    }

    async fn execute(
        &self,
        _workspace: &Path,
        _request: &ProcessRequest,
        _cancellation: &CancellationToken,
    ) -> Result<ProcessExecution, ProcessBackendError> {
        Err(ProcessBackendError::Disabled)
    }
}

/// Explicitly trusted direct host execution.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeProcessBackend;

#[async_trait]
impl ProcessBackend for NativeProcessBackend {
    fn descriptor(&self) -> ProcessBackendDescriptor {
        ProcessBackendDescriptor::native()
    }

    async fn execute(
        &self,
        workspace: &Path,
        request: &ProcessRequest,
        cancellation: &CancellationToken,
    ) -> Result<ProcessExecution, ProcessBackendError> {
        let mut command = Command::new(&request.program);
        command
            .args(&request.args)
            .current_dir(workspace)
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear();
        copy_native_toolchain_environment(&mut command);
        for (name, value) in &request.environment {
            command.env(name, value);
        }
        execute_child(
            command,
            &request.program,
            request.timeout,
            request.max_output_bytes,
            cancellation,
        )
        .await
        .map(|captured| captured.into_execution(self.descriptor()))
    }
}

/// Supported local OCI command-line interfaces.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OciRuntimeKind {
    Docker,
    Podman,
}

impl OciRuntimeKind {
    #[must_use]
    pub const fn executable_name(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
        }
    }

    const fn version_marker(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
        }
    }
}

/// Resource and namespace limits for one restricted container.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OciSandboxProfile {
    pub memory_bytes: u64,
    pub milli_cpus: u32,
    pub pids_limit: u32,
    pub tmpfs_bytes: u64,
}

impl Default for OciSandboxProfile {
    fn default() -> Self {
        Self {
            memory_bytes: 2 * 1024 * 1024 * 1024,
            milli_cpus: 2_000,
            pids_limit: 128,
            tmpfs_bytes: 512 * 1024 * 1024,
        }
    }
}

impl OciSandboxProfile {
    /// Validates limits before they can reach a runtime CLI.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessBackendError::InvalidConfiguration`] when any resource
    /// ceiling is zero, implausibly small, or outside Pactrail's hard maximum.
    pub fn validate(&self) -> Result<(), ProcessBackendError> {
        if !(64 * 1024 * 1024..=1024_u64.pow(4)).contains(&self.memory_bytes) {
            return Err(ProcessBackendError::InvalidConfiguration(
                "OCI memory must be between 64 MiB and 1 TiB".to_owned(),
            ));
        }
        if !(100..=256_000).contains(&self.milli_cpus) {
            return Err(ProcessBackendError::InvalidConfiguration(
                "OCI CPU limit must be between 0.1 and 256 CPUs".to_owned(),
            ));
        }
        if !(16..=32_768).contains(&self.pids_limit) {
            return Err(ProcessBackendError::InvalidConfiguration(
                "OCI PID limit must be between 16 and 32,768".to_owned(),
            ));
        }
        if !(1024 * 1024..=64 * 1024 * 1024 * 1024).contains(&self.tmpfs_bytes) {
            return Err(ProcessBackendError::InvalidConfiguration(
                "OCI temporary space must be between 1 MiB and 64 GiB".to_owned(),
            ));
        }
        Ok(())
    }

    fn digest(&self) -> Result<String, ProcessBackendError> {
        let encoded = serde_json::to_vec(self).map_err(ProcessBackendError::ProfileEncoding)?;
        Ok(blake3::hash(&encoded).to_hex().to_string())
    }
}

/// Validated OCI backend configuration supplied by the trusted user surface.
#[derive(Clone, Debug)]
pub struct OciProcessConfig {
    pub runtime: OciRuntimeKind,
    pub runtime_executable: OsString,
    pub image: String,
    pub profile: OciSandboxProfile,
}

impl OciProcessConfig {
    #[must_use]
    pub fn for_runtime(runtime: OciRuntimeKind, image: impl Into<String>) -> Self {
        Self {
            runtime,
            runtime_executable: OsString::from(runtime.executable_name()),
            image: image.into(),
            profile: OciSandboxProfile::default(),
        }
    }
}

/// Restricted execution through a local Docker or Podman CLI.
#[derive(Clone, Debug)]
pub struct OciProcessBackend {
    runtime: OciRuntimeKind,
    runtime_path: PathBuf,
    runtime_fingerprint: String,
    image: String,
    image_identity: String,
    profile: OciSandboxProfile,
    descriptor: ProcessBackendDescriptor,
}

impl OciProcessBackend {
    /// Resolves and probes the runtime and image without pulling from a registry.
    ///
    /// # Errors
    ///
    /// Returns an error when configuration is invalid, the runtime cannot be
    /// resolved outside the workspace, its identity is unexpected, or the
    /// selected image is not already present with an immutable SHA-256 ID.
    pub async fn initialize(
        config: OciProcessConfig,
        forbidden_roots: &[PathBuf],
    ) -> Result<Self, ProcessBackendError> {
        config.profile.validate()?;
        validate_image_reference(&config.image)?;
        let runtime_path = resolve_runtime_executable(&config.runtime_executable)?;
        reject_runtime_inside(&runtime_path, forbidden_roots)?;
        let runtime_fingerprint = digest_file(&runtime_path)?;
        probe_runtime(&runtime_path, config.runtime).await?;
        let image_identity = resolve_image_identity(&runtime_path, &config.image).await?;
        let profile_digest = config.profile.digest()?;
        let descriptor = ProcessBackendDescriptor {
            kind: ProcessBackendKind::OciRestricted,
            strength: "oci_restricted".to_owned(),
            runtime: Some(runtime_path.display().to_string()),
            runtime_fingerprint: Some(runtime_fingerprint.clone()),
            image: Some(config.image.clone()),
            image_identity: Some(image_identity.clone()),
            profile_digest,
            network: "none".to_owned(),
            filesystem: "candidate read-write; image read-only; bounded temporary filesystem"
                .to_owned(),
        };
        Ok(Self {
            runtime: config.runtime,
            runtime_path,
            runtime_fingerprint,
            image: config.image,
            image_identity,
            profile: config.profile,
            descriptor,
        })
    }

    #[cfg(test)]
    fn fixture(
        runtime: OciRuntimeKind,
        runtime_path: PathBuf,
        image: &str,
        image_identity: &str,
        profile: OciSandboxProfile,
    ) -> Result<Self, ProcessBackendError> {
        profile.validate()?;
        let profile_digest = profile.digest()?;
        let descriptor = ProcessBackendDescriptor {
            kind: ProcessBackendKind::OciRestricted,
            strength: "oci_restricted".to_owned(),
            runtime: Some(runtime_path.display().to_string()),
            runtime_fingerprint: Some("fixture-runtime-digest".to_owned()),
            image: Some(image.to_owned()),
            image_identity: Some(image_identity.to_owned()),
            profile_digest,
            network: "none".to_owned(),
            filesystem: "candidate read-write; image read-only; bounded temporary filesystem"
                .to_owned(),
        };
        Ok(Self {
            runtime,
            runtime_path,
            runtime_fingerprint: "fixture-runtime-digest".to_owned(),
            image: image.to_owned(),
            image_identity: image_identity.to_owned(),
            profile,
            descriptor,
        })
    }

    fn command_plan(
        &self,
        workspace: &Path,
        request: &ProcessRequest,
        container_name: &str,
    ) -> Result<Vec<OsString>, ProcessBackendError> {
        let workspace = workspace.canonicalize().map_err(|source| {
            ProcessBackendError::WorkspaceCanonicalization {
                path: workspace.to_path_buf(),
                source,
            }
        })?;
        let mount = bind_mount_argument(&workspace)?;
        let cpu = format!(
            "{}.{:03}",
            self.profile.milli_cpus / 1_000,
            self.profile.milli_cpus % 1_000
        );
        let mut args = vec![
            OsString::from("run"),
            OsString::from("--rm"),
            OsString::from("--init"),
            OsString::from("--pull=never"),
            OsString::from(format!("--name={container_name}")),
            OsString::from("--network=none"),
            OsString::from("--read-only"),
            OsString::from("--cap-drop=ALL"),
            OsString::from("--security-opt=no-new-privileges"),
            OsString::from("--ipc=none"),
            OsString::from(format!("--pids-limit={}", self.profile.pids_limit)),
            OsString::from(format!("--memory={}", self.profile.memory_bytes)),
            OsString::from(format!("--memory-swap={}", self.profile.memory_bytes)),
            OsString::from(format!("--cpus={cpu}")),
            OsString::from(format!(
                "--tmpfs={CONTAINER_TMP}:rw,noexec,nosuid,nodev,size={}",
                self.profile.tmpfs_bytes
            )),
            OsString::from(format!("--mount={mount}")),
            OsString::from(format!("--workdir={CONTAINER_WORKSPACE}")),
            OsString::from(format!("--entrypoint={}", request.program)),
        ];
        for (name, value) in &request.environment {
            args.push(OsString::from("--env"));
            args.push(OsString::from(format!("{name}={value}")));
        }
        args.push(OsString::from(&self.image_identity));
        args.extend(request.args.iter().map(OsString::from));
        Ok(args)
    }

    /// Returns the resolved runtime fingerprint used in approval binding.
    #[must_use]
    pub fn runtime_fingerprint(&self) -> &str {
        &self.runtime_fingerprint
    }

    /// Returns the configured human-readable image reference.
    #[must_use]
    pub fn image_reference(&self) -> &str {
        &self.image
    }
}

#[async_trait]
impl ProcessBackend for OciProcessBackend {
    fn descriptor(&self) -> ProcessBackendDescriptor {
        self.descriptor.clone()
    }

    async fn execute(
        &self,
        workspace: &Path,
        request: &ProcessRequest,
        cancellation: &CancellationToken,
    ) -> Result<ProcessExecution, ProcessBackendError> {
        if cancellation.is_cancelled() {
            return Err(ProcessBackendError::Cancelled {
                program: request.program.clone(),
            });
        }
        let container_name = unique_container_name();
        let args = self.command_plan(workspace, request, &container_name)?;
        let mut command = runtime_command(&self.runtime_path);
        command.args(&args);
        let captured = execute_child(
            command,
            self.runtime.executable_name(),
            request.timeout,
            request.max_output_bytes,
            cancellation,
        )
        .await;
        match captured {
            Ok(captured) => Ok(captured.into_execution(self.descriptor())),
            Err(primary) => match cleanup_container(&self.runtime_path, &container_name).await {
                Ok(()) => Err(primary),
                Err(cleanup) => Err(ProcessBackendError::CleanupAfterFailure {
                    primary: primary.to_string(),
                    cleanup: cleanup.to_string(),
                }),
            },
        }
    }
}

#[derive(Debug)]
struct CapturedProcess {
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

enum Completion {
    Exited(io::Result<std::process::ExitStatus>),
    Cancelled,
    TimedOut,
}

impl CapturedProcess {
    fn into_execution(self, backend: ProcessBackendDescriptor) -> ProcessExecution {
        ProcessExecution {
            exit_code: self.exit_code,
            stdout: self.stdout,
            stderr: self.stderr,
            stdout_truncated: self.stdout_truncated,
            stderr_truncated: self.stderr_truncated,
            backend,
        }
    }
}

async fn execute_child(
    mut command: Command,
    display_program: &str,
    timeout: Duration,
    max_output_bytes: usize,
    cancellation: &CancellationToken,
) -> Result<CapturedProcess, ProcessBackendError> {
    command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|source| ProcessBackendError::Spawn {
            program: display_program.to_owned(),
            source,
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ProcessBackendError::Pipe {
            program: display_program.to_owned(),
            stream: "stdout",
        })?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ProcessBackendError::Pipe {
            program: display_program.to_owned(),
            stream: "stderr",
        })?;
    let stdout_task = tokio::spawn(read_bounded(stdout, max_output_bytes));
    let stderr_task = tokio::spawn(read_bounded(stderr, max_output_bytes));

    let completion = tokio::select! {
        biased;
        () = cancellation.cancelled() => Completion::Cancelled,
        () = tokio::time::sleep(timeout) => Completion::TimedOut,
        result = child.wait() => Completion::Exited(result),
    };
    let status = match completion {
        Completion::Cancelled => {
            terminate_child(&mut child).await;
            abort_readers(&stdout_task, &stderr_task);
            return Err(ProcessBackendError::Cancelled {
                program: display_program.to_owned(),
            });
        }
        Completion::TimedOut => {
            terminate_child(&mut child).await;
            abort_readers(&stdout_task, &stderr_task);
            return Err(ProcessBackendError::Timeout {
                program: display_program.to_owned(),
                seconds: timeout.as_secs(),
            });
        }
        Completion::Exited(result) => result.map_err(|source| ProcessBackendError::Wait {
            program: display_program.to_owned(),
            source,
        })?,
    };
    let (stdout, stdout_truncated) = join_reader(stdout_task, "stdout").await?;
    let (stderr, stderr_truncated) = join_reader(stderr_task, "stderr").await?;
    Ok(CapturedProcess {
        exit_code: status.code(),
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
    })
}

async fn terminate_child(child: &mut tokio::process::Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

fn abort_readers(
    stdout: &JoinHandle<Result<(Vec<u8>, bool), io::Error>>,
    stderr: &JoinHandle<Result<(Vec<u8>, bool), io::Error>>,
) {
    stdout.abort();
    stderr.abort();
}

async fn join_reader(
    task: JoinHandle<Result<(Vec<u8>, bool), io::Error>>,
    stream: &'static str,
) -> Result<(Vec<u8>, bool), ProcessBackendError> {
    task.await
        .map_err(|source| ProcessBackendError::OutputTask { stream, source })?
        .map_err(|source| ProcessBackendError::OutputRead { stream, source })
}

async fn read_bounded<R: AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
) -> Result<(Vec<u8>, bool), io::Error> {
    let mut retained = Vec::with_capacity(limit.min(64 * 1024));
    let mut buffer = vec![0_u8; READ_CHUNK_BYTES].into_boxed_slice();
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let remaining = limit.saturating_sub(retained.len());
        let keep = remaining.min(count);
        retained.extend_from_slice(&buffer[..keep]);
        truncated |= keep < count;
    }
    Ok((retained, truncated))
}

fn runtime_command(runtime_path: &Path) -> Command {
    let mut command = Command::new(runtime_path);
    command.env_clear();
    for name in RUNTIME_ENVIRONMENT_NAMES {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
}

async fn probe_runtime(
    runtime_path: &Path,
    kind: OciRuntimeKind,
) -> Result<(), ProcessBackendError> {
    let mut command = runtime_command(runtime_path);
    command.arg("--version");
    let captured = execute_child(
        command,
        kind.executable_name(),
        RUNTIME_PROBE_TIMEOUT,
        RUNTIME_PROBE_OUTPUT_BYTES,
        &CancellationToken::new(),
    )
    .await?;
    let output = String::from_utf8_lossy(&captured.stdout).to_ascii_lowercase();
    if captured.exit_code != Some(0) || !output.contains(kind.version_marker()) {
        return Err(ProcessBackendError::RuntimeMismatch {
            path: runtime_path.to_path_buf(),
            expected: kind.executable_name(),
        });
    }
    Ok(())
}

async fn resolve_image_identity(
    runtime_path: &Path,
    image: &str,
) -> Result<String, ProcessBackendError> {
    let mut command = runtime_command(runtime_path);
    command.args(["image", "inspect", "--format", "{{.Id}}", image]);
    let captured = execute_child(
        command,
        &runtime_path.display().to_string(),
        RUNTIME_PROBE_TIMEOUT,
        RUNTIME_PROBE_OUTPUT_BYTES,
        &CancellationToken::new(),
    )
    .await?;
    if captured.exit_code != Some(0) {
        return Err(ProcessBackendError::ImageUnavailable {
            image: image.to_owned(),
        });
    }
    let identity = String::from_utf8_lossy(&captured.stdout).trim().to_owned();
    validate_image_identity(&identity)?;
    Ok(identity)
}

async fn cleanup_container(
    runtime_path: &Path,
    container_name: &str,
) -> Result<(), ProcessBackendError> {
    let mut remove = runtime_command(runtime_path);
    remove.args(["container", "rm", "--force", container_name]);
    let removal = execute_child(
        remove,
        &runtime_path.display().to_string(),
        CLEANUP_TIMEOUT,
        RUNTIME_PROBE_OUTPUT_BYTES,
        &CancellationToken::new(),
    )
    .await?;
    if removal.exit_code == Some(0) {
        return Ok(());
    }

    let mut inspect = runtime_command(runtime_path);
    inspect.args(["container", "inspect", container_name]);
    let inspection = execute_child(
        inspect,
        &runtime_path.display().to_string(),
        CLEANUP_TIMEOUT,
        RUNTIME_PROBE_OUTPUT_BYTES,
        &CancellationToken::new(),
    )
    .await?;
    if inspection.exit_code.is_some_and(|code| code != 0) {
        Ok(())
    } else {
        Err(ProcessBackendError::ContainerCleanup {
            container: container_name.to_owned(),
        })
    }
}

fn resolve_runtime_executable(value: &OsString) -> Result<PathBuf, ProcessBackendError> {
    let configured = PathBuf::from(value);
    let has_directory = configured.is_absolute() || configured.components().count() > 1;
    if has_directory {
        return canonical_runtime(&configured);
    }
    let path = std::env::var_os("PATH").ok_or_else(|| ProcessBackendError::RuntimeNotFound {
        executable: configured.clone(),
    })?;
    for directory in std::env::split_paths(&path) {
        for candidate in executable_candidates(&directory, &configured) {
            if candidate.is_file() {
                return canonical_runtime(&candidate);
            }
        }
    }
    Err(ProcessBackendError::RuntimeNotFound {
        executable: configured,
    })
}

fn executable_candidates(directory: &Path, executable: &Path) -> Vec<PathBuf> {
    let direct = directory.join(executable);
    #[cfg(windows)]
    {
        if direct.extension().is_some() {
            vec![direct]
        } else {
            vec![direct.with_extension("exe")]
        }
    }
    #[cfg(not(windows))]
    {
        vec![direct]
    }
}

fn canonical_runtime(path: &Path) -> Result<PathBuf, ProcessBackendError> {
    path.canonicalize()
        .map_err(|source| ProcessBackendError::RuntimeCanonicalization {
            path: path.to_path_buf(),
            source,
        })
}

fn reject_runtime_inside(
    runtime: &Path,
    forbidden_roots: &[PathBuf],
) -> Result<(), ProcessBackendError> {
    for root in forbidden_roots {
        let root = root.canonicalize().map_err(|source| {
            ProcessBackendError::WorkspaceCanonicalization {
                path: root.clone(),
                source,
            }
        })?;
        if runtime.starts_with(&root) {
            return Err(ProcessBackendError::RuntimeInsideWorkspace {
                runtime: runtime.to_path_buf(),
                workspace: root,
            });
        }
    }
    Ok(())
}

fn digest_file(path: &Path) -> Result<String, ProcessBackendError> {
    let mut file = File::open(path).map_err(|source| ProcessBackendError::RuntimeDigest {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count =
            file.read(&mut buffer)
                .map_err(|source| ProcessBackendError::RuntimeDigest {
                    path: path.to_path_buf(),
                    source,
                })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn validate_image_reference(image: &str) -> Result<(), ProcessBackendError> {
    if image.trim().is_empty()
        || image.len() > 1_024
        || image.chars().any(char::is_control)
        || image.starts_with('-')
    {
        return Err(ProcessBackendError::InvalidConfiguration(
            "OCI image reference must be non-empty, at most 1,024 bytes, contain no control characters, and not begin with '-'"
                .to_owned(),
        ));
    }
    Ok(())
}

fn validate_image_identity(identity: &str) -> Result<(), ProcessBackendError> {
    let Some(digest) = identity.strip_prefix("sha256:") else {
        return Err(ProcessBackendError::InvalidImageIdentity(
            identity.to_owned(),
        ));
    };
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ProcessBackendError::InvalidImageIdentity(
            identity.to_owned(),
        ));
    }
    Ok(())
}

fn bind_mount_argument(workspace: &Path) -> Result<String, ProcessBackendError> {
    let source = workspace
        .to_str()
        .ok_or_else(|| ProcessBackendError::NonUtf8Workspace(workspace.to_path_buf()))?;
    if source.contains('\0') || source.contains('\n') || source.contains('\r') {
        return Err(ProcessBackendError::UnsafeMountPath(
            workspace.to_path_buf(),
        ));
    }
    let escaped = source.replace('"', "\"\"");
    Ok(format!(
        "type=bind,source=\"{escaped}\",target={CONTAINER_WORKSPACE}"
    ))
}

fn unique_container_name() -> String {
    let sequence = CONTAINER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("pactrail-{}-{sequence}", std::process::id())
}

fn copy_native_toolchain_environment(command: &mut Command) {
    for name in super::process::SAFE_ENVIRONMENT_NAMES {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
}

/// Process backend initialization or execution failure.
#[derive(Debug, Error)]
pub enum ProcessBackendError {
    #[error("process execution is disabled for this run")]
    Disabled,
    #[error("invalid process backend configuration: {0}")]
    InvalidConfiguration(String),
    #[error("could not encode the sandbox profile: {0}")]
    ProfileEncoding(serde_json::Error),
    #[error("OCI runtime executable {executable:?} was not found on PATH")]
    RuntimeNotFound { executable: PathBuf },
    #[error("OCI runtime path {path:?} could not be resolved: {source}")]
    RuntimeCanonicalization {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("workspace path {path:?} could not be resolved: {source}")]
    WorkspaceCanonicalization {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("OCI runtime {runtime:?} is inside workspace {workspace:?}")]
    RuntimeInsideWorkspace {
        runtime: PathBuf,
        workspace: PathBuf,
    },
    #[error("OCI runtime {path:?} could not be fingerprinted: {source}")]
    RuntimeDigest {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("OCI runtime {path:?} did not identify itself as {expected}")]
    RuntimeMismatch {
        path: PathBuf,
        expected: &'static str,
    },
    #[error(
        "OCI image {image:?} is not available locally; Pactrail never pulls sandbox images implicitly"
    )]
    ImageUnavailable { image: String },
    #[error("OCI runtime returned invalid immutable image identity {0:?}")]
    InvalidImageIdentity(String),
    #[error("workspace path is not valid UTF-8 and cannot be mounted safely: {0:?}")]
    NonUtf8Workspace(PathBuf),
    #[error("workspace mount path contains an unsafe control character: {0:?}")]
    UnsafeMountPath(PathBuf),
    #[error("process {program:?} could not be started: {source}")]
    Spawn {
        program: String,
        #[source]
        source: io::Error,
    },
    #[error("process {program:?} could not be waited on: {source}")]
    Wait {
        program: String,
        #[source]
        source: io::Error,
    },
    #[error("process {program:?} did not expose a {stream} pipe")]
    Pipe {
        program: String,
        stream: &'static str,
    },
    #[error("process {program:?} timed out after {seconds} seconds")]
    Timeout { program: String, seconds: u64 },
    #[error("process {program:?} was cancelled")]
    Cancelled { program: String },
    #[error("{stream} output reader failed: {source}")]
    OutputRead {
        stream: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("{stream} output task failed: {source}")]
    OutputTask {
        stream: &'static str,
        #[source]
        source: tokio::task::JoinError,
    },
    #[error("container {container:?} still existed after forced cleanup")]
    ContainerCleanup { container: String },
    #[error("process failed ({primary}) and cleanup could not be proven ({cleanup})")]
    CleanupAfterFailure { primary: String, cleanup: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ProcessRequest {
        ProcessRequest {
            program: "cargo".to_owned(),
            args: vec!["test".to_owned(), "--workspace".to_owned()],
            timeout: Duration::from_mins(2),
            max_output_bytes: 1024,
            environment: BTreeMap::from([("RUST_BACKTRACE".to_owned(), "1".to_owned())]),
        }
    }

    fn backend(runtime: OciRuntimeKind) -> OciProcessBackend {
        OciProcessBackend::fixture(
            runtime,
            PathBuf::from(runtime.executable_name()),
            "pactrail/sandbox:fixture",
            &format!("sha256:{}", "a".repeat(64)),
            OciSandboxProfile::default(),
        )
        .unwrap_or_else(|error| unreachable!("backend: {error}"))
    }

    #[test]
    fn restricted_plan_contains_every_security_boundary() {
        let workspace = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        let args = backend(OciRuntimeKind::Docker)
            .command_plan(workspace.path(), &request(), "pactrail-fixture")
            .unwrap_or_else(|error| unreachable!("plan: {error}"));
        let args = args
            .iter()
            .map(|value| value.to_string_lossy())
            .collect::<Vec<_>>();
        for required in [
            "--rm",
            "--init",
            "--pull=never",
            "--network=none",
            "--read-only",
            "--cap-drop=ALL",
            "--security-opt=no-new-privileges",
            "--ipc=none",
            "--pids-limit=128",
            "--memory=2147483648",
            "--memory-swap=2147483648",
            "--cpus=2.000",
            "--workdir=/workspace",
            "--entrypoint=cargo",
        ] {
            assert!(
                args.iter().any(|argument| argument == required),
                "{required}"
            );
        }
        assert!(
            args.iter()
                .any(|argument| argument.starts_with("--mount=type=bind,source="))
        );
        assert!(args.iter().any(|argument| argument == "--env"));
        assert!(args.iter().any(|argument| argument == "RUST_BACKTRACE=1"));
        assert_eq!(args[args.len() - 2], "test");
        assert_eq!(args[args.len() - 1], "--workspace");
    }

    #[test]
    fn model_arguments_never_become_runtime_flags_or_shell_syntax() {
        let workspace = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        let mut request = request();
        request.args = vec![
            "--network=host".to_owned(),
            "; rm -rf /".to_owned(),
            "$(touch /host)".to_owned(),
        ];
        let args = backend(OciRuntimeKind::Podman)
            .command_plan(workspace.path(), &request, "pactrail-fixture")
            .unwrap_or_else(|error| unreachable!("plan: {error}"));
        let image_index = args
            .iter()
            .position(|argument| argument.to_string_lossy().starts_with("sha256:"))
            .unwrap_or_else(|| unreachable!("image identity"));
        assert_eq!(
            &args[image_index + 1..],
            request.args.iter().map(OsString::from).collect::<Vec<_>>()
        );
        assert!(
            args[..image_index]
                .iter()
                .any(|argument| argument == "--network=none")
        );
    }

    #[test]
    fn rejects_workspace_shadowed_runtime() {
        let workspace = tempfile::tempdir().unwrap_or_else(|error| unreachable!("root: {error}"));
        let runtime = workspace.path().join(if cfg!(windows) {
            "docker.exe"
        } else {
            "docker"
        });
        std::fs::write(&runtime, b"fixture")
            .unwrap_or_else(|error| unreachable!("runtime: {error}"));
        let runtime = runtime
            .canonicalize()
            .unwrap_or_else(|error| unreachable!("canonical: {error}"));
        assert!(matches!(
            reject_runtime_inside(&runtime, &[workspace.path().to_path_buf()]),
            Err(ProcessBackendError::RuntimeInsideWorkspace { .. })
        ));
    }

    #[test]
    fn image_identity_must_be_a_complete_sha256_digest() {
        assert!(validate_image_identity(&format!("sha256:{}", "a".repeat(64))).is_ok());
        assert!(validate_image_identity("pactrail/sandbox:latest").is_err());
        assert!(validate_image_identity("sha256:abcd").is_err());
    }

    #[test]
    fn profile_limits_fail_closed() {
        let profile = OciSandboxProfile {
            pids_limit: 0,
            ..OciSandboxProfile::default()
        };
        assert!(profile.validate().is_err());
        let profile = OciSandboxProfile {
            memory_bytes: u64::MAX,
            ..OciSandboxProfile::default()
        };
        assert!(profile.validate().is_err());
    }
}
