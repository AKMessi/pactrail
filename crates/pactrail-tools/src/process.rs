use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use pactrail_core::{ApprovalBinding, ApprovalRequest, Capability, RunId};
use schemars::{JsonSchema, schema_for};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::{
    DisabledProcessBackend, NativeProcessBackend, ProcessBackend, ProcessBackendDescriptor,
    ProcessBackendKind, ProcessRequest, Tool, ToolAnnotations, ToolContext, ToolDescriptor,
    ToolError, ToolOutput,
};

const DEFAULT_TIMEOUT_SECONDS: u64 = 120;
const MAX_TIMEOUT_SECONDS: u64 = 3_600;
const DEFAULT_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_PROGRAM_BYTES: usize = 4 * 1024;
const MAX_ARGUMENTS: usize = 4_096;
const MAX_ARGUMENT_BYTES: usize = 256 * 1024;
const MAX_AGGREGATE_ARGUMENT_BYTES: usize = 2 * 1024 * 1024;
const MAX_ENVIRONMENT_ENTRIES: usize = 256;
const MAX_ENVIRONMENT_VALUE_BYTES: usize = 256 * 1024;
const MAX_AGGREGATE_ENVIRONMENT_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const SAFE_ENVIRONMENT_NAMES: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SystemRoot",
    "WINDIR",
    "COMSPEC",
    "ProgramFiles",
    "ProgramFiles(Arm)",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "CommonProgramFiles",
    "CommonProgramFiles(Arm)",
    "CommonProgramFiles(x86)",
    "CommonProgramW6432",
    "ProgramData",
    "ALLUSERSPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "SystemDrive",
    "PROCESSOR_ARCHITECTURE",
    "OS",
    "VSINSTALLDIR",
    "VCINSTALLDIR",
    "VCToolsInstallDir",
    "WindowsSdkDir",
    "WindowsSDKVersion",
    "UniversalCRTSdkDir",
    "UCRTVersion",
    "INCLUDE",
    "LIB",
    "LIBPATH",
    "HOME",
    "HOMEDRIVE",
    "HOMEPATH",
    "USERPROFILE",
    "TMP",
    "TEMP",
    "CARGO_HOME",
    "RUSTUP_HOME",
];

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RunProcessInput {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
    #[serde(default = "default_output_limit")]
    max_output_bytes: usize,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

const fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT_SECONDS
}

const fn default_output_limit() -> usize {
    DEFAULT_OUTPUT_BYTES
}

/// Executes a program through the explicitly selected process backend.
pub struct RunProcessTool {
    backend: Arc<dyn ProcessBackend>,
    cancellation: CancellationToken,
}

impl RunProcessTool {
    /// Creates a process tool bound to one backend and run cancellation token.
    #[must_use]
    pub fn new(backend: Arc<dyn ProcessBackend>, cancellation: CancellationToken) -> Self {
        Self {
            backend,
            cancellation,
        }
    }

    /// Creates a fail-closed process tool.
    #[must_use]
    pub fn disabled() -> Self {
        Self::new(Arc::new(DisabledProcessBackend), CancellationToken::new())
    }

    /// Creates an explicitly trusted native process tool.
    #[must_use]
    pub fn native_trusted() -> Self {
        Self::new(Arc::new(NativeProcessBackend), CancellationToken::new())
    }
}

impl Default for RunProcessTool {
    fn default() -> Self {
        Self::disabled()
    }
}

#[async_trait]
impl Tool for RunProcessTool {
    fn descriptor(&self) -> ToolDescriptor {
        let backend = self.backend.descriptor();
        let (description, annotations) = match backend.kind {
            ProcessBackendKind::Disabled => (
                "Process execution is disabled for this run.".to_owned(),
                ToolAnnotations::RESTRICTED_EXECUTION,
            ),
            ProcessBackendKind::NativeTrusted => (
                "Run a program directly in the isolated candidate. No shell interpolation is performed. This explicitly trusted backend retains host filesystem and network authority."
                    .to_owned(),
                ToolAnnotations::HOST_EXECUTION,
            ),
            ProcessBackendKind::OciRestricted => (
                "Run a program without shell interpolation in a restricted OCI container with candidate-only bind access, no network, a read-only image, dropped capabilities, and bounded resources."
                    .to_owned(),
                ToolAnnotations::RESTRICTED_EXECUTION,
            ),
        };
        ToolDescriptor {
            name: "run_process".to_owned(),
            description,
            input_schema: serde_json::to_value(schema_for!(RunProcessInput))
                .unwrap_or_else(|_| json!({})),
            required_capability: Capability::ProcessSpawn,
            annotations,
        }
    }

    async fn execute(
        &self,
        context: &ToolContext<'_>,
        value: Value,
    ) -> Result<ToolOutput, ToolError> {
        let request: RunProcessInput =
            serde_json::from_value(value).map_err(|source| ToolError::InvalidInput {
                tool: "run_process",
                source,
            })?;
        validate_request(&request)?;
        let backend = self.backend.descriptor();
        if let Some(run_id) = context.run_id {
            context.authorize_request(process_approval_request(
                run_id,
                context.workspace.workspace_root(),
                &request,
                &backend,
            ))?;
        } else {
            context.authorize(
                &Capability::ProcessSpawn,
                request.program.clone(),
                "run_process",
            )?;
        }

        let execution = self
            .backend
            .execute(
                context.workspace.workspace_root(),
                &ProcessRequest {
                    program: request.program.clone(),
                    args: request.args.clone(),
                    timeout: Duration::from_secs(request.timeout_seconds),
                    max_output_bytes: request.max_output_bytes,
                    environment: request.environment.clone(),
                },
                &self.cancellation,
            )
            .await?;
        let succeeded = execution.exit_code == Some(0);
        let backend = serde_json::to_value(&execution.backend).map_err(ToolError::Serialization)?;
        Ok(ToolOutput {
            content: json!({
                "program": request.program,
                "args": request.args,
                "exit_code": execution.exit_code,
                "stdout": String::from_utf8_lossy(&execution.stdout),
                "stderr": String::from_utf8_lossy(&execution.stderr),
                "backend": backend,
            }),
            summary: format!(
                "process exited {}",
                execution
                    .exit_code
                    .map_or_else(|| "without a code".to_owned(), |code| code.to_string())
            ),
            observed_effects: vec![
                "process.spawn".to_owned(),
                format!("process.backend:{:?}", execution.backend.kind).to_lowercase(),
                match execution.backend.kind {
                    ProcessBackendKind::OciRestricted => {
                        "filesystem.candidate-only;network.denied".to_owned()
                    }
                    ProcessBackendKind::NativeTrusted => {
                        "host effects require post-call reconciliation".to_owned()
                    }
                    ProcessBackendKind::Disabled => "process.denied".to_owned(),
                },
            ],
            succeeded,
            truncated: execution.stdout_truncated || execution.stderr_truncated,
        })
    }
}

fn process_approval_request(
    run_id: RunId,
    workspace: &std::path::Path,
    request: &RunProcessInput,
    backend: &ProcessBackendDescriptor,
) -> ApprovalRequest {
    let environment_names = request.environment.keys().cloned().collect::<Vec<_>>();
    let resource = json!({
        "program": &request.program,
        "args": &request.args,
        "environment_names": &environment_names,
        "timeout_seconds": request.timeout_seconds,
        "max_output_bytes": request.max_output_bytes,
    })
    .to_string();
    let backend_kind = serde_json::to_value(backend.kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned());
    ApprovalRequest {
        binding: ApprovalBinding {
            run_id,
            capability: Capability::ProcessSpawn,
            actor_fingerprint: blake3::hash(resource.as_bytes()).to_hex().to_string(),
            resource: resource.clone(),
            backend_kind: backend_kind.clone(),
            backend_identity: backend
                .image_identity
                .clone()
                .or_else(|| backend.runtime_fingerprint.clone()),
            profile_digest: backend.profile_digest.clone(),
        },
        reason: "the model requested execution of an exact process command".to_owned(),
        presentation: BTreeMap::from([
            ("command".to_owned(), resource),
            ("workspace".to_owned(), workspace.display().to_string()),
            ("backend".to_owned(), backend_kind),
            ("network".to_owned(), backend.network.clone()),
            ("filesystem".to_owned(), backend.filesystem.clone()),
            (
                "environment_names".to_owned(),
                if environment_names.is_empty() {
                    "none".to_owned()
                } else {
                    environment_names.join(", ")
                },
            ),
        ]),
    }
}

fn validate_request(request: &RunProcessInput) -> Result<(), ToolError> {
    if request.program.trim().is_empty()
        || request.program.len() > MAX_PROGRAM_BYTES
        || request.program.chars().any(char::is_control)
    {
        return Err(ToolError::InvalidRange(format!(
            "program must be non-empty, at most {MAX_PROGRAM_BYTES} bytes, and contain no control characters"
        )));
    }
    if request.args.len() > MAX_ARGUMENTS {
        return Err(ToolError::InvalidRange(format!(
            "args cannot contain more than {MAX_ARGUMENTS} values"
        )));
    }
    let aggregate_argument_bytes = request.args.iter().try_fold(0_usize, |total, argument| {
        if argument.len() > MAX_ARGUMENT_BYTES || argument.contains('\0') {
            return None;
        }
        total.checked_add(argument.len())
    });
    if aggregate_argument_bytes.is_none_or(|total| total > MAX_AGGREGATE_ARGUMENT_BYTES) {
        return Err(ToolError::InvalidRange(format!(
            "each argument must be at most {MAX_ARGUMENT_BYTES} bytes, contain no NUL byte, and aggregate arguments must not exceed {MAX_AGGREGATE_ARGUMENT_BYTES} bytes"
        )));
    }
    if request.environment.len() > MAX_ENVIRONMENT_ENTRIES {
        return Err(ToolError::InvalidRange(format!(
            "environment cannot contain more than {MAX_ENVIRONMENT_ENTRIES} entries"
        )));
    }
    let aggregate_environment_bytes =
        request
            .environment
            .iter()
            .try_fold(0_usize, |total, (name, value)| {
                if !valid_environment_name(name)
                    || value.len() > MAX_ENVIRONMENT_VALUE_BYTES
                    || value.contains('\0')
                {
                    return None;
                }
                total.checked_add(name.len())?.checked_add(value.len())
            });
    if aggregate_environment_bytes.is_none_or(|total| total > MAX_AGGREGATE_ENVIRONMENT_BYTES) {
        return Err(ToolError::InvalidRange(format!(
            "environment names must be valid, each value must be at most {MAX_ENVIRONMENT_VALUE_BYTES} bytes with no NUL byte, and aggregate environment data must not exceed {MAX_AGGREGATE_ENVIRONMENT_BYTES} bytes"
        )));
    }
    if request.timeout_seconds == 0 || request.timeout_seconds > MAX_TIMEOUT_SECONDS {
        return Err(ToolError::InvalidRange(format!(
            "timeout_seconds must be between 1 and {MAX_TIMEOUT_SECONDS}"
        )));
    }
    if request.max_output_bytes == 0 || request.max_output_bytes > MAX_OUTPUT_BYTES {
        return Err(ToolError::InvalidRange(format!(
            "max_output_bytes must be between 1 and {MAX_OUTPUT_BYTES}"
        )));
    }
    Ok(())
}

fn valid_environment_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PolicyEngine;
    use pactrail_workspace::WorkspaceTransaction;

    #[tokio::test]
    async fn process_requires_explicit_capability() {
        let source = tempfile::tempdir().unwrap_or_else(|error| unreachable!("source: {error}"));
        let control = tempfile::tempdir().unwrap_or_else(|error| unreachable!("control: {error}"));
        let transaction = WorkspaceTransaction::create(
            source.path(),
            control.path().join("run"),
            &[".".to_owned()],
        )
        .unwrap_or_else(|error| unreachable!("transaction: {error}"));
        let policy = PolicyEngine::local_default();
        let context = ToolContext::new(&transaction, &policy, None);
        let result = RunProcessTool::native_trusted()
            .execute(&context, json!({"program":"cargo"}))
            .await;
        assert!(matches!(result, Err(ToolError::ApprovalRequired { .. })));
    }

    #[test]
    fn inherited_environment_is_an_explicit_toolchain_only_allowlist() {
        assert!(SAFE_ENVIRONMENT_NAMES.contains(&"LOCALAPPDATA"));
        assert!(SAFE_ENVIRONMENT_NAMES.contains(&"ProgramFiles(x86)"));
        assert!(SAFE_ENVIRONMENT_NAMES.contains(&"VCToolsInstallDir"));
        assert!(!SAFE_ENVIRONMENT_NAMES.contains(&"OPENROUTER_API_KEY"));
        assert!(!SAFE_ENVIRONMENT_NAMES.contains(&"CARGO_TARGET_DIR"));
        assert!(!SAFE_ENVIRONMENT_NAMES.contains(&"RUSTC_WRAPPER"));
    }

    #[test]
    fn default_process_tool_fails_closed() {
        assert_eq!(
            RunProcessTool::default().backend.descriptor().kind,
            ProcessBackendKind::Disabled
        );
    }
}
