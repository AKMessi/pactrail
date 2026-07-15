use std::collections::BTreeMap;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use pactrail_core::Capability;
use schemars::{JsonSchema, schema_for};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use crate::{Tool, ToolAnnotations, ToolContext, ToolDescriptor, ToolError, ToolOutput};

const DEFAULT_TIMEOUT_SECONDS: u64 = 120;
const MAX_TIMEOUT_SECONDS: u64 = 3_600;
const DEFAULT_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

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

/// Executes a program directly without shell interpolation.
pub struct RunProcessTool;

#[async_trait]
impl Tool for RunProcessTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "run_process".to_owned(),
            description: "Run a program directly in the isolated workspace. No shell interpolation is performed. Native execution is not a network sandbox.".to_owned(),
            input_schema: serde_json::to_value(schema_for!(RunProcessInput))
                .unwrap_or_else(|_| json!({})),
            required_capability: Capability::ProcessSpawn,
            annotations: ToolAnnotations::HOST_EXECUTION,
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
        context.authorize(
            &Capability::ProcessSpawn,
            request.program.clone(),
            "run_process",
        )?;

        let mut command = Command::new(&request.program);
        command
            .args(&request.args)
            .current_dir(context.workspace.workspace_root())
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear();
        copy_safe_environment(&mut command);
        for (name, value) in &request.environment {
            if !valid_environment_name(name) {
                return Err(ToolError::InvalidRange(format!(
                    "invalid environment variable name {name:?}"
                )));
            }
            command.env(name, value);
        }
        let mut child = command.spawn().map_err(|source| ToolError::Spawn {
            program: request.program.clone(),
            source,
        })?;
        let stdout = child.stdout.take().ok_or_else(|| ToolError::Spawn {
            program: request.program.clone(),
            source: std::io::Error::other("stdout pipe was not created"),
        })?;
        let stderr = child.stderr.take().ok_or_else(|| ToolError::Spawn {
            program: request.program.clone(),
            source: std::io::Error::other("stderr pipe was not created"),
        })?;
        let stdout_task = tokio::spawn(read_bounded(stdout, request.max_output_bytes));
        let stderr_task = tokio::spawn(read_bounded(stderr, request.max_output_bytes));

        let status = if let Ok(result) =
            tokio::time::timeout(Duration::from_secs(request.timeout_seconds), child.wait()).await
        {
            result.map_err(|source| ToolError::Spawn {
                program: request.program.clone(),
                source,
            })?
        } else {
            let _kill_result = child.kill().await;
            let _wait_result = child.wait().await;
            stdout_task.abort();
            stderr_task.abort();
            return Err(ToolError::Timeout {
                program: request.program,
                seconds: request.timeout_seconds,
            });
        };
        let (stdout, stdout_truncated) = stdout_task.await.map_err(ToolError::Join)??;
        let (stderr, stderr_truncated) = stderr_task.await.map_err(ToolError::Join)??;
        let succeeded = status.success();
        Ok(ToolOutput {
            content: json!({
                "program": request.program,
                "args": request.args,
                "exit_code": status.code(),
                "stdout": String::from_utf8_lossy(&stdout),
                "stderr": String::from_utf8_lossy(&stderr),
            }),
            summary: format!(
                "process exited {}",
                status
                    .code()
                    .map_or_else(|| "without a code".to_owned(), |code| code.to_string())
            ),
            observed_effects: vec![
                "process.spawn".to_owned(),
                "filesystem effects require post-call reconciliation".to_owned(),
            ],
            succeeded,
            truncated: stdout_truncated || stderr_truncated,
        })
    }
}

fn validate_request(request: &RunProcessInput) -> Result<(), ToolError> {
    if request.program.trim().is_empty() {
        return Err(ToolError::InvalidRange(
            "program cannot be empty".to_owned(),
        ));
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

async fn read_bounded<R: AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
) -> Result<(Vec<u8>, bool), ToolError> {
    let mut retained = Vec::with_capacity(limit.min(64 * 1024));
    let mut buffer = vec![0_u8; 16 * 1024].into_boxed_slice();
    let mut truncated = false;
    loop {
        let count = reader
            .read(&mut buffer)
            .await
            .map_err(|source| ToolError::Io {
                path: "<process-output>".into(),
                source,
            })?;
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

fn copy_safe_environment(command: &mut Command) {
    const SAFE_NAMES: &[&str] = &[
        "PATH",
        "PATHEXT",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "HOME",
        "USERPROFILE",
        "TMP",
        "TEMP",
        "CARGO_HOME",
        "RUSTUP_HOME",
    ];
    for name in SAFE_NAMES {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
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
        let context = ToolContext {
            workspace: &transaction,
            policy: &policy,
            memory: None,
        };
        let result = RunProcessTool
            .execute(&context, json!({"program":"cargo"}))
            .await;
        assert!(matches!(result, Err(ToolError::ApprovalRequired { .. })));
    }
}
