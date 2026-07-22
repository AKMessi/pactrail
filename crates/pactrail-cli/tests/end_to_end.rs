use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

#[test]
fn mcp_offline_lifecycle_is_scriptable_and_fails_closed() {
    let workspace = tempfile::tempdir().unwrap_or_else(|error| unreachable!("workspace: {error}"));

    let initialized = pactrail(workspace.path(), ["mcp", "init"]);
    assert!(
        initialized.status.success(),
        "MCP init failed: {}",
        String::from_utf8_lossy(&initialized.stderr)
    );
    let manifest_path = workspace.path().join(".pactrail").join("mcp.toml");
    assert!(manifest_path.is_file());

    let checked = pactrail(workspace.path(), ["mcp", "check", "--json"]);
    assert!(checked.status.success());
    let report: Value = serde_json::from_slice(&checked.stdout)
        .unwrap_or_else(|error| unreachable!("MCP check JSON: {error}"));
    assert_eq!(report["valid"], true);
    assert_eq!(report["enabled_servers"], 0);

    std::fs::write(
        &manifest_path,
        r#"schema = 1

[[servers]]
name = "demo"
enabled = false
startup_timeout_seconds = 10
request_timeout_seconds = 10
max_output_bytes = 4096
environment = []
resources = []
prompts = []

[servers.transport]
kind = "streamable-http"
url = "http://127.0.0.1:65535/mcp"
allow_loopback_http = true

[servers.tools]
"#,
    )
    .unwrap_or_else(|error| unreachable!("write manifest: {error}"));

    let listed = pactrail(workspace.path(), ["mcp", "list", "--json"]);
    assert!(listed.status.success());
    let servers: Value = serde_json::from_slice(&listed.stdout)
        .unwrap_or_else(|error| unreachable!("MCP list JSON: {error}"));
    assert_eq!(servers[0]["name"], "demo");
    assert_eq!(servers[0]["snapshot"], "missing");

    let enabled = pactrail(workspace.path(), ["mcp", "enable", "demo"]);
    assert!(!enabled.status.success());
    assert!(String::from_utf8_lossy(&enabled.stderr).contains("snapshot"));
    let preserved = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|error| unreachable!("read manifest: {error}"));
    assert!(preserved.contains("enabled = false"));
}

#[test]
#[allow(clippy::too_many_lines)]
fn interrupted_process_resumes_the_same_hash_linked_run() {
    let workspace = tempfile::tempdir().unwrap_or_else(|error| unreachable!("workspace: {error}"));
    let listener = TcpListener::bind("127.0.0.1:0")
        .unwrap_or_else(|error| unreachable!("mock provider: {error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| unreachable!("provider address: {error}"));
    let (request_started, request_observed) = mpsc::sync_channel(1);
    let server = thread::spawn(move || {
        let (mut interrupted_stream, _address) = listener
            .accept()
            .unwrap_or_else(|error| unreachable!("first provider accept: {error}"));
        read_request(&mut interrupted_stream);
        request_started
            .send(())
            .unwrap_or_else(|error| unreachable!("request signal: {error}"));
        let mut sink = Vec::new();
        let _closed = interrupted_stream.read_to_end(&mut sink);

        let (mut resumed_stream, _address) = listener
            .accept()
            .unwrap_or_else(|error| unreachable!("resume provider accept: {error}"));
        read_request(&mut resumed_stream);
        let response = json!({
            "id": "resumed-summary",
            "choices": [{
                "message": {"content": "The interrupted run resumed from its durable checkpoint."},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 18, "completion_tokens": 9}
        });
        write_response(&mut resumed_stream, &response);
    });

    let mut interrupted = Command::new(env!("CARGO_BIN_EXE_pactrail"));
    interrupted
        .args([
            "--workspace",
            path_text(workspace.path()),
            "run",
            "Explain this workspace",
            "--provider",
            "open-ai-compatible",
            "--base-url",
            &format!("http://{address}/v1"),
            "--model",
            "mock-coder",
            "--no-stream",
            "--output",
            "json",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = interrupted
        .spawn()
        .unwrap_or_else(|error| unreachable!("interrupted run: {error}"));
    if let Err(error) = request_observed.recv_timeout(Duration::from_secs(10)) {
        let _kill = child.kill();
        let _wait = child.wait();
        unreachable!("model request was not observed: {error}");
    }
    let runs_root = workspace.path().join(".pactrail").join("runs");
    let run_ids = std::fs::read_dir(&runs_root)
        .unwrap_or_else(|error| unreachable!("runs: {error}"))
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(run_ids.len(), 1);
    let run_id = &run_ids[0];
    assert!(runs_root.join(run_id).join("run.json").is_file());
    let concurrent = pactrail(workspace.path(), ["resume", run_id, "--output", "json"]);
    assert!(!concurrent.status.success());
    assert!(
        String::from_utf8_lossy(&concurrent.stderr)
            .contains("already active in another Pactrail process")
    );
    child
        .kill()
        .unwrap_or_else(|error| unreachable!("terminate interrupted run: {error}"));
    child
        .wait()
        .unwrap_or_else(|error| unreachable!("reap interrupted run: {error}"));

    let resumed = pactrail(workspace.path(), ["resume", run_id, "--output", "json"]);
    assert!(
        resumed.status.success(),
        "resume failed: {}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    server
        .join()
        .unwrap_or_else(|_| unreachable!("provider thread panicked"));
    let result: Value = serde_json::from_slice(&resumed.stdout)
        .unwrap_or_else(|error| unreachable!("resume JSON: {error}"));
    assert_eq!(result["run_id"], run_id.as_str());
    assert_eq!(result["outcome"], "answered");
    assert_eq!(result["tokens"], 27);

    let trace = pactrail(workspace.path(), ["trace", run_id, "--json"]);
    assert!(trace.status.success());
    let trace: Value = serde_json::from_slice(&trace.stdout)
        .unwrap_or_else(|error| unreachable!("trace JSON: {error}"));
    let events = trace
        .as_array()
        .unwrap_or_else(|| unreachable!("trace is not an array"));
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event"]["type"] == "contract_registered")
            .count(),
        1
    );
    assert!(events.iter().any(|event| {
        event["event"]["type"] == "note_recorded"
            && event["event"]["data"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("resumed from safe session checkpoint"))
    }));
}

#[test]
fn complete_run_is_isolated_then_applies() {
    let workspace = tempfile::tempdir().unwrap_or_else(|error| unreachable!("workspace: {error}"));
    let listener = TcpListener::bind("127.0.0.1:0")
        .unwrap_or_else(|error| unreachable!("mock provider: {error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| unreachable!("provider address: {error}"));
    let server = thread::spawn(move || serve_model(&listener));

    let output = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args([
            "--workspace",
            path_text(workspace.path()),
            "run",
            "Create a README",
            "--provider",
            "open-ai-compatible",
            "--base-url",
            &format!("http://{address}/v1"),
            "--model",
            "mock-coder",
            "--no-stream",
            "--output",
            "json",
        ])
        .output()
        .unwrap_or_else(|error| unreachable!("run command: {error}"));
    assert!(
        output.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    server
        .join()
        .unwrap_or_else(|_| unreachable!("provider thread panicked"));
    let result: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| unreachable!("run JSON: {error}"));
    let run_id = result["run_id"]
        .as_str()
        .unwrap_or_else(|| unreachable!("run id missing"));
    assert_eq!(result["outcome"], "ready_to_apply");
    assert!(!workspace.path().join("README.md").exists());
    let review_path = workspace
        .path()
        .join(".pactrail")
        .join("runs")
        .join(run_id)
        .join("review.diff");
    let review_before = std::fs::read_to_string(&review_path)
        .unwrap_or_else(|error| unreachable!("review artifact: {error}"));
    assert!(review_before.contains("+# Created by Pactrail"));

    assert_cross_workspace_run_commands_fail(workspace.path(), run_id);
    assert!(!workspace.path().join("README.md").exists());
    assert_review_commands(workspace.path(), run_id);
    apply_and_assert_receipt(workspace.path(), run_id, &review_path, &review_before);
    assert_applied_memory(workspace.path(), run_id);
}

fn assert_review_commands(workspace: &Path, run_id: &str) {
    let diff = pactrail(workspace, ["diff", run_id, "--json"]);
    assert!(
        diff.status.success(),
        "diff failed: {}",
        String::from_utf8_lossy(&diff.stderr)
    );
    let diff: Value = serde_json::from_slice(&diff.stdout)
        .unwrap_or_else(|error| unreachable!("diff JSON: {error}"));
    assert_eq!(diff["schema_version"], 1);
    assert_eq!(diff["run_id"], run_id);
    assert_eq!(diff["receipt_integrity"], "verified");
    assert!(
        diff["unified_diff"]
            .as_str()
            .is_some_and(|text| text.contains("+# Created by Pactrail"))
    );

    let runs_alias = pactrail(workspace, ["runs", "--json"]);
    assert!(runs_alias.status.success());
    let runs: Value = serde_json::from_slice(&runs_alias.stdout)
        .unwrap_or_else(|error| unreachable!("runs alias JSON: {error}"));
    assert!(runs.as_array().is_some_and(|runs| !runs.is_empty()));
}

fn apply_and_assert_receipt(
    workspace: &Path,
    run_id: &str,
    review_path: &Path,
    review_before: &str,
) {
    let apply = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args([
            "--workspace",
            path_text(workspace),
            "apply",
            run_id,
            "--json",
        ])
        .output()
        .unwrap_or_else(|error| unreachable!("apply command: {error}"));
    assert!(
        apply.status.success(),
        "apply failed: {}",
        String::from_utf8_lossy(&apply.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("README.md")).ok(),
        Some("# Created by Pactrail\n".to_owned())
    );
    assert_eq!(
        std::fs::read_to_string(review_path).ok(),
        Some(review_before.to_owned())
    );
    let applied: Value = serde_json::from_slice(&apply.stdout)
        .unwrap_or_else(|error| unreachable!("apply JSON: {error}"));
    assert_eq!(applied["outcome"], "applied");

    let repeated = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args([
            "--workspace",
            path_text(workspace),
            "apply",
            run_id,
            "--json",
        ])
        .output()
        .unwrap_or_else(|error| unreachable!("repeated apply command: {error}"));
    assert!(
        repeated.status.success(),
        "repeated apply failed: {}",
        String::from_utf8_lossy(&repeated.stderr)
    );
    let repeated_receipt: Value = serde_json::from_slice(&repeated.stdout)
        .unwrap_or_else(|error| unreachable!("repeated apply JSON: {error}"));
    assert_eq!(
        repeated_receipt["integrity_hash"],
        applied["integrity_hash"]
    );
}

#[test]
fn image_run_is_provider_native_path_free_and_trace_safe() {
    let workspace = tempfile::tempdir().unwrap_or_else(|error| unreachable!("workspace: {error}"));
    let attachments =
        tempfile::tempdir().unwrap_or_else(|error| unreachable!("attachments: {error}"));
    let image_path = attachments.path().join("failure.png");
    let attachment_directory = attachments
        .path()
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_else(|| unreachable!("attachment directory name"))
        .to_owned();
    std::fs::write(&image_path, tiny_png(320, 200))
        .unwrap_or_else(|error| unreachable!("image: {error}"));
    let listener = TcpListener::bind("127.0.0.1:0")
        .unwrap_or_else(|error| unreachable!("mock provider: {error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| unreachable!("provider address: {error}"));
    let server = thread::spawn(move || {
        let (mut stream, _address) = listener
            .accept()
            .unwrap_or_else(|error| unreachable!("provider accept: {error}"));
        let request = capture_request(&mut stream);
        write_response(
            &mut stream,
            &json!({
                "id": "vision-summary",
                "choices": [{
                    "message": {"content": "The screenshot shows a bounded fixture."},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 30, "completion_tokens": 8}
            }),
        );
        request
    });

    let output = pactrail(
        workspace.path(),
        [
            "run",
            "Explain what the screenshot shows",
            "--provider",
            "open-ai-compatible",
            "--base-url",
            &format!("http://{address}/v1"),
            "--model",
            "mock-vision",
            "--vision",
            "on",
            "--image",
            path_text(&image_path),
            "--no-stream",
            "--output",
            "json",
        ],
    );
    assert!(
        output.status.success(),
        "vision run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request = server
        .join()
        .unwrap_or_else(|_| unreachable!("provider thread panicked"));
    let header_end =
        find_bytes(&request, b"\r\n\r\n").unwrap_or_else(|| unreachable!("HTTP headers missing"));
    let body: Value = serde_json::from_slice(&request[header_end + 4..])
        .unwrap_or_else(|error| unreachable!("request JSON: {error}"));
    let data_url = body["messages"][1]["content"][2]["image_url"]["url"]
        .as_str()
        .unwrap_or_else(|| unreachable!("image data URL missing"));
    assert!(data_url.starts_with("data:image/png;base64,"));
    assert!(!String::from_utf8_lossy(&request).contains(path_text(&image_path)));
    assert!(!String::from_utf8_lossy(&request).contains(&attachment_directory));

    let result: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| unreachable!("run JSON: {error}"));
    assert_eq!(result["outcome"], "answered");
    let run_id = result["run_id"]
        .as_str()
        .unwrap_or_else(|| unreachable!("run id missing"));
    let manifest: Value = serde_json::from_slice(
        &std::fs::read(
            workspace
                .path()
                .join(".pactrail")
                .join("runs")
                .join(run_id)
                .join("run.json"),
        )
        .unwrap_or_else(|error| unreachable!("manifest: {error}")),
    )
    .unwrap_or_else(|error| unreachable!("manifest JSON: {error}"));
    assert!(manifest["args"].get("images").is_none());
    let trace = pactrail(workspace.path(), ["trace", run_id, "--json"]);
    assert!(trace.status.success());
    let trace_text = String::from_utf8_lossy(&trace.stdout);
    assert!(trace_text.contains("seal_image_artifacts"));
    assert!(!trace_text.contains("data:image"));
    assert!(!trace_text.contains(path_text(&image_path)));
    assert!(!trace_text.contains("failure.png"));
}

#[test]
fn failed_run_exports_trace_and_remains_discoverable() {
    let workspace = tempfile::tempdir().unwrap_or_else(|error| unreachable!("workspace: {error}"));
    let listener = TcpListener::bind("127.0.0.1:0")
        .unwrap_or_else(|error| unreachable!("mock provider: {error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| unreachable!("provider address: {error}"));
    let response = json!({
        "id": "invalid-turn",
        "choices": [{
            "message": {"content": ""},
            "finish_reason": "tool_calls"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 1}
    });
    let server = thread::spawn(move || serve_responses(&listener, &[response]));

    let output = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args([
            "--workspace",
            path_text(workspace.path()),
            "run",
            "Trigger a protocol failure",
            "--provider",
            "open-ai-compatible",
            "--base-url",
            &format!("http://{address}/v1"),
            "--model",
            "broken-mock",
            "--no-stream",
        ])
        .output()
        .unwrap_or_else(|error| unreachable!("run command: {error}"));
    server
        .join()
        .unwrap_or_else(|_| unreachable!("provider thread panicked"));
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let marker = "engine failed for run ";
    let start = stderr.find(marker).map_or_else(
        || unreachable!("run id missing from: {stderr}"),
        |index| index + marker.len(),
    );
    let run_id = stderr
        .get(start..start + 36)
        .unwrap_or_else(|| unreachable!("run id was not a UUID"));
    assert!(stderr.contains("Portable trace:"));

    let trace_path = workspace
        .path()
        .join(".pactrail")
        .join("runs")
        .join(run_id)
        .join("trace.jsonl");
    let trace = std::fs::read_to_string(trace_path)
        .unwrap_or_else(|error| unreachable!("failed trace: {error}"));
    assert!(trace.contains("\"to\":\"failed\""));

    let list = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args(["--workspace", path_text(workspace.path()), "list", "--json"])
        .output()
        .unwrap_or_else(|error| unreachable!("list command: {error}"));
    assert!(list.status.success());
    let runs: Value = serde_json::from_slice(&list.stdout)
        .unwrap_or_else(|error| unreachable!("list JSON: {error}"));
    assert_eq!(runs[0]["run_id"], run_id);
    assert_eq!(runs[0]["state"], "failed");
    assert!(runs[0]["outcome"].is_null());

    let trace_command = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args([
            "--workspace",
            path_text(workspace.path()),
            "trace",
            run_id,
            "--json",
        ])
        .output()
        .unwrap_or_else(|error| unreachable!("trace command: {error}"));
    assert!(trace_command.status.success());
}

#[test]
#[allow(clippy::too_many_lines)]
fn answered_run_is_inspectable_and_has_no_apply_boundary() {
    let workspace = tempfile::tempdir().unwrap_or_else(|error| unreachable!("workspace: {error}"));
    let listener = TcpListener::bind("127.0.0.1:0")
        .unwrap_or_else(|error| unreachable!("mock provider: {error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| unreachable!("provider address: {error}"));
    let response = json!({
        "id": "answer-turn",
        "choices": [{
            "message": {"content": "This is an empty test workspace."},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 12, "completion_tokens": 7}
    });
    let server = thread::spawn(move || serve_responses(&listener, &[response]));

    let output = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args([
            "--workspace",
            path_text(workspace.path()),
            "run",
            "What is this directory about?",
            "--provider",
            "open-ai-compatible",
            "--base-url",
            &format!("http://{address}/v1"),
            "--model",
            "mock-coder",
            "--no-stream",
            "--output",
            "json",
        ])
        .output()
        .unwrap_or_else(|error| unreachable!("answer run: {error}"));
    server
        .join()
        .unwrap_or_else(|_| unreachable!("provider thread panicked"));
    assert!(
        output.status.success(),
        "answer failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| unreachable!("answer JSON: {error}"));
    let run_id = result["run_id"]
        .as_str()
        .unwrap_or_else(|| unreachable!("run id missing"));
    assert_eq!(result["outcome"], "answered");
    assert_eq!(result["changes"], json!([]));

    let inspect = pactrail(workspace.path(), ["inspect", run_id, "--json"]);
    assert!(inspect.status.success());
    let receipt: Value = serde_json::from_slice(&inspect.stdout)
        .unwrap_or_else(|error| unreachable!("inspect JSON: {error}"));
    assert_eq!(receipt["outcome"], "answered");

    let trace = pactrail(workspace.path(), ["trace", run_id]);
    assert!(trace.status.success());
    let trace = String::from_utf8_lossy(&trace.stdout);
    assert!(trace.contains("hash chain verified"));
    assert!(trace.contains("Reviewing -> Completed"));

    let list = pactrail(workspace.path(), ["list", "--json"]);
    let runs: Value = serde_json::from_slice(&list.stdout)
        .unwrap_or_else(|error| unreachable!("list JSON: {error}"));
    assert_eq!(runs[0]["state"], "completed");
    assert_eq!(runs[0]["outcome"], "answered");

    let migration = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args([
            "--workspace",
            path_text(workspace.path()),
            "migrate",
            "--json",
        ])
        .env("PACTRAIL_CONFIG_DIR", workspace.path().join("config"))
        .output()
        .unwrap_or_else(|error| unreachable!("state audit: {error}"));
    assert!(
        migration.status.success(),
        "state audit failed: {}",
        String::from_utf8_lossy(&migration.stderr)
    );
    let migration: Value = serde_json::from_slice(&migration.stdout)
        .unwrap_or_else(|error| unreachable!("state audit JSON: {error}"));
    assert_eq!(migration["validation"]["event_runs"], 1);
    assert_eq!(migration["validation"]["receipts"], 1);
    assert!(
        migration["validation"]["checkpoints"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
}

#[test]
fn ready_run_can_be_discarded_idempotently() {
    let workspace = tempfile::tempdir().unwrap_or_else(|error| unreachable!("workspace: {error}"));
    let listener = TcpListener::bind("127.0.0.1:0")
        .unwrap_or_else(|error| unreachable!("mock provider: {error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| unreachable!("provider address: {error}"));
    let server = thread::spawn(move || serve_model(&listener));
    let output = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args([
            "--workspace",
            path_text(workspace.path()),
            "run",
            "Create a README",
            "--provider",
            "open-ai-compatible",
            "--base-url",
            &format!("http://{address}/v1"),
            "--model",
            "mock-coder",
            "--no-stream",
            "--output",
            "json",
        ])
        .output()
        .unwrap_or_else(|error| unreachable!("run command: {error}"));
    server
        .join()
        .unwrap_or_else(|_| unreachable!("provider thread panicked"));
    assert!(output.status.success());
    let result: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| unreachable!("run JSON: {error}"));
    let run_id = result["run_id"]
        .as_str()
        .unwrap_or_else(|| unreachable!("run id missing"));
    let review = workspace
        .path()
        .join(".pactrail")
        .join("runs")
        .join(run_id)
        .join("review.diff");
    let review_before = std::fs::read_to_string(&review)
        .unwrap_or_else(|error| unreachable!("review diff: {error}"));

    let discarded = pactrail(workspace.path(), ["discard", run_id, "--json"]);
    assert!(discarded.status.success());
    let receipt: Value = serde_json::from_slice(&discarded.stdout)
        .unwrap_or_else(|error| unreachable!("discard JSON: {error}"));
    assert_eq!(receipt["outcome"], "discarded");
    assert!(!workspace.path().join("README.md").exists());
    assert_eq!(
        std::fs::read_to_string(&review).ok().as_deref(),
        Some(review_before.as_str())
    );

    let repeated = pactrail(workspace.path(), ["discard", run_id, "--json"]);
    assert!(repeated.status.success());
    let repeated: Value = serde_json::from_slice(&repeated.stdout)
        .unwrap_or_else(|error| unreachable!("repeated discard JSON: {error}"));
    assert_eq!(repeated["integrity_hash"], receipt["integrity_hash"]);

    let list = pactrail(workspace.path(), ["list", "--json"]);
    let runs: Value = serde_json::from_slice(&list.stdout)
        .unwrap_or_else(|error| unreachable!("list JSON: {error}"));
    assert_eq!(runs[0]["state"], "discarded");
    assert_eq!(runs[0]["outcome"], "discarded");
}

#[test]
fn task_contract_can_apply_immediately() {
    let workspace = tempfile::tempdir().unwrap_or_else(|error| unreachable!("workspace: {error}"));
    let template = pactrail(workspace.path(), ["task-template", "Create a README"]);
    assert!(template.status.success());
    let task_path = workspace.path().join("task.toml");
    std::fs::write(&task_path, &template.stdout)
        .unwrap_or_else(|error| unreachable!("write task contract: {error}"));

    let listener = TcpListener::bind("127.0.0.1:0")
        .unwrap_or_else(|error| unreachable!("mock provider: {error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| unreachable!("provider address: {error}"));
    let server = thread::spawn(move || serve_model(&listener));
    let output = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args([
            "--workspace",
            path_text(workspace.path()),
            "run",
            "--task",
            path_text(&task_path),
            "--provider",
            "open-ai-compatible",
            "--base-url",
            &format!("http://{address}/v1"),
            "--model",
            "mock-coder",
            "--no-stream",
            "--apply",
            "--output",
            "json",
        ])
        .output()
        .unwrap_or_else(|error| unreachable!("task run: {error}"));
    server
        .join()
        .unwrap_or_else(|_| unreachable!("provider thread panicked"));
    assert!(
        output.status.success(),
        "task run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| unreachable!("task run JSON: {error}"));
    assert_eq!(result["outcome"], "applied");
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("README.md")).ok(),
        Some("# Created by Pactrail\n".to_owned())
    );
}

fn assert_applied_memory(workspace: &Path, run_id: &str) {
    let memory = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args([
            "--workspace",
            path_text(workspace),
            "memory",
            "search",
            "Create README",
            "--json",
        ])
        .output()
        .unwrap_or_else(|error| unreachable!("memory search: {error}"));
    assert!(
        memory.status.success(),
        "memory search failed: {}",
        String::from_utf8_lossy(&memory.stderr)
    );
    let memories: Value = serde_json::from_slice(&memory.stdout)
        .unwrap_or_else(|error| unreachable!("memory JSON: {error}"));
    assert_eq!(memories[0]["memory"]["source"], "applied_receipt");
    assert_eq!(memories[0]["memory"]["source_run_id"], run_id);

    let trace_path = workspace
        .join(".pactrail")
        .join("runs")
        .join(run_id)
        .join("trace.jsonl");
    let trace_text = std::fs::read_to_string(&trace_path)
        .unwrap_or_else(|error| unreachable!("portable trace: {error}"));
    assert!(trace_text.lines().count() >= 10);
    assert!(trace_text.contains("\"duration_ms\""));
    assert!(trace_text.contains("\"to\":\"applied\""));

    let trace = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args([
            "--workspace",
            path_text(workspace),
            "trace",
            run_id,
            "--json",
        ])
        .output()
        .unwrap_or_else(|error| unreachable!("trace command: {error}"));
    assert!(trace.status.success());
    let events: Value = serde_json::from_slice(&trace.stdout)
        .unwrap_or_else(|error| unreachable!("trace JSON: {error}"));
    assert!(events.as_array().is_some_and(|events| events.len() >= 10));
}

#[test]
fn no_subcommand_fails_fast_without_a_terminal() {
    let output = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|error| unreachable!("interactive command: {error}"));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(
        "interactive mode requires a terminal; use `pactrail run <goal>` for automation"
    ));
}

#[test]
fn completion_is_generated_and_prompt_cannot_shadow_a_subcommand() {
    let completion = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args(["completion", "powershell"])
        .output()
        .unwrap_or_else(|error| unreachable!("completion command: {error}"));
    assert!(completion.status.success());
    assert!(
        String::from_utf8_lossy(&completion.stdout)
            .contains("Register-ArgumentCompleter -Native -CommandName 'pactrail'")
    );

    let conflict = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args(["initial task", "doctor"])
        .output()
        .unwrap_or_else(|error| unreachable!("conflicting prompt: {error}"));
    assert!(!conflict.status.success());
    assert!(
        String::from_utf8_lossy(&conflict.stderr)
            .contains("an interactive PROMPT cannot be combined with a subcommand")
    );

    let workspace = tempfile::tempdir().unwrap_or_else(|error| unreachable!("workspace: {error}"));
    let empty_key = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args([
            "--workspace",
            path_text(workspace.path()),
            "run",
            "test empty credential handling",
            "--provider",
            "open-ai",
            "--model",
            "mock-coder",
            "--api-key-env",
            "PACTRAIL_TEST_EMPTY_KEY",
        ])
        .env("PACTRAIL_TEST_EMPTY_KEY", "")
        .output()
        .unwrap_or_else(|error| unreachable!("empty API key command: {error}"));
    assert!(!empty_key.status.success());
    assert!(String::from_utf8_lossy(&empty_key.stderr).contains(
        "required API key environment variable \"PACTRAIL_TEST_EMPTY_KEY\" is not set or is empty"
    ));
}

#[test]
fn static_commands_and_memory_lifecycle_are_scriptable() {
    let workspace = tempfile::tempdir().unwrap_or_else(|error| unreachable!("workspace: {error}"));

    let tools = pactrail(workspace.path(), ["tools", "--json"]);
    assert!(tools.status.success());
    let tools: Value = serde_json::from_slice(&tools.stdout)
        .unwrap_or_else(|error| unreachable!("tools JSON: {error}"));
    let tools = tools
        .as_array()
        .unwrap_or_else(|| unreachable!("tool descriptors were not an array"));
    assert_eq!(tools.len(), 17);
    assert!(tools.iter().any(|tool| tool["name"] == "run_process"));
    let patch = tools
        .iter()
        .find(|tool| tool["name"] == "apply_patch")
        .unwrap_or_else(|| unreachable!("strict patch tool was not registered"));
    assert_eq!(patch["required_capability"], "file_write");
    assert_eq!(patch["annotations"]["risk"], "workspace_mutation");
    assert_eq!(patch["annotations"]["parallel_safe"], false);
    let graph = tools
        .iter()
        .find(|tool| tool["name"] == "search_code_graph")
        .unwrap_or_else(|| unreachable!("repository evidence graph tool was not registered"));
    assert_eq!(graph["required_capability"], "file_read");
    assert_eq!(graph["annotations"]["read_only"], true);
    let impact = tools
        .iter()
        .find(|tool| tool["name"] == "search_change_impact")
        .unwrap_or_else(|| unreachable!("change-impact tool was not registered"));
    assert_eq!(impact["required_capability"], "file_read");
    assert_eq!(impact["annotations"]["parallel_safe"], true);
    for name in ["git_status", "git_diff", "git_history"] {
        let git_tool = tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| unreachable!("Git evidence tool {name} was not registered"));
        assert_eq!(git_tool["required_capability"], "file_read");
        assert_eq!(git_tool["annotations"]["read_only"], true);
        assert_eq!(git_tool["annotations"]["parallel_safe"], true);
    }

    let schema = pactrail(workspace.path(), ["schema"]);
    assert!(schema.status.success());
    let schema: Value = serde_json::from_slice(&schema.stdout)
        .unwrap_or_else(|error| unreachable!("schema JSON: {error}"));
    assert_eq!(schema["title"], "TaskContract");

    let template = pactrail(workspace.path(), ["task-template", "Audit the CLI"]);
    assert!(template.status.success());
    let template: toml::Value = toml::from_slice(&template.stdout)
        .unwrap_or_else(|error| unreachable!("task template TOML: {error}"));
    assert_eq!(template["schema_version"].as_integer(), Some(1));
    assert_eq!(template["goal"].as_str(), Some("Audit the CLI"));

    let doctor = pactrail(workspace.path(), ["doctor", "--json"]);
    assert!(doctor.status.success());
    let doctor: Value = serde_json::from_slice(&doctor.stdout)
        .unwrap_or_else(|error| unreachable!("doctor JSON: {error}"));
    assert_eq!(doctor["commands"].as_array().map(Vec::len), Some(6));
    assert!(
        doctor["native_process_isolation"]
            .as_str()
            .is_some_and(|value| value.contains("not a host-filesystem or network sandbox"))
    );
    let process_backends = doctor["process_backends"]
        .as_array()
        .unwrap_or_else(|| unreachable!("process backend diagnostics missing"));
    assert_eq!(process_backends.len(), 4);
    assert_eq!(process_backends[0]["id"], "disabled");
    assert_eq!(process_backends[0]["available"], true);
    assert_eq!(process_backends[1]["id"], "native");

    let compatibility = pactrail(workspace.path(), ["compatibility", "--json"]);
    assert!(compatibility.status.success());
    let compatibility: Value = serde_json::from_slice(&compatibility.stdout)
        .unwrap_or_else(|error| unreachable!("compatibility JSON: {error}"));
    assert_eq!(compatibility["manifest_schema"], 1);
    assert_eq!(compatibility["formats"].as_array().map(Vec::len), Some(18));
    assert!(
        compatibility["formats"]
            .as_array()
            .is_some_and(|formats| formats.iter().any(|format| {
                format["id"] == "event_envelope"
                    && format["current_schema"] == 2
                    && format["minimum_readable_schema"] == 1
            }))
    );

    for shell in ["bash", "elvish", "fish", "powershell", "zsh"] {
        let completion = pactrail(workspace.path(), ["completion", shell]);
        assert!(completion.status.success(), "{shell} completion failed");
        assert!(
            completion.stdout.len() > 100,
            "{shell} completion was unexpectedly empty"
        );
    }
    assert!(!workspace.path().join(".pactrail").exists());
}

#[test]
fn unknown_run_inspection_fails_without_creating_state() {
    let workspace = tempfile::tempdir().unwrap_or_else(|error| unreachable!("workspace: {error}"));
    let unknown = "00000000-0000-7000-8000-000000000000";
    for command in ["inspect", "trace", "diff"] {
        let output = pactrail(workspace.path(), [command, unknown]);
        assert!(!output.status.success(), "{command} unexpectedly succeeded");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("not found"),
            "{command} did not explain the missing run: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !workspace.path().join(".pactrail").exists(),
            "{command} created state while inspecting an unknown run"
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn state_migration_is_explicit_preflighted_and_machine_readable() {
    let workspace = tempfile::tempdir().unwrap_or_else(|error| unreachable!("workspace: {error}"));
    let state = workspace.path().join("state");
    std::fs::create_dir_all(&state).unwrap_or_else(|error| unreachable!("state: {error}"));
    let events = state.join("events.sqlite3");
    drop(
        pactrail_store::EventStore::open(&events)
            .unwrap_or_else(|error| unreachable!("events fixture: {error}")),
    );
    let connection = rusqlite::Connection::open(&events)
        .unwrap_or_else(|error| unreachable!("events fixture: {error}"));
    connection
        .pragma_update(
            None,
            "user_version",
            pactrail_store::MIN_EVENT_DATABASE_SCHEMA_VERSION,
        )
        .unwrap_or_else(|error| unreachable!("events schema: {error}"));
    drop(connection);
    let memory = state.join("memory.sqlite3");
    drop(
        rusqlite::Connection::open(&memory)
            .unwrap_or_else(|error| unreachable!("memory fixture: {error}")),
    );

    let config = workspace.path().join("config").join("pactrail");
    let settings = config.join("settings.toml");
    std::fs::create_dir_all(&config)
        .unwrap_or_else(|error| unreachable!("config fixture: {error}"));
    std::fs::write(&settings, "schema = 1\nprovider = \"ollama\"\napi_key_env = \"KEY\"\ncontext_tokens = 4096\nmax_output_tokens = 512\nmax_turns = 4\nallow_process = false\n")
        .unwrap_or_else(|error| unreachable!("settings fixture: {error}"));

    let audit = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args([
            "--workspace",
            path_text(workspace.path()),
            "--state-dir",
            path_text(&state),
            "migrate",
            "--json",
        ])
        .env("PACTRAIL_CONFIG_DIR", &config)
        .output()
        .unwrap_or_else(|error| unreachable!("migration audit: {error}"));
    assert!(
        audit.status.success(),
        "{}",
        String::from_utf8_lossy(&audit.stderr)
    );
    let audit: Value = serde_json::from_slice(&audit.stdout)
        .unwrap_or_else(|error| unreachable!("migration audit JSON: {error}"));
    assert_eq!(audit["pending_components"], 3, "{audit:#}");
    assert_eq!(audit["changed_components"], 0);
    assert_eq!(
        pactrail_store::EventStore::database_schema_version(&events).ok(),
        Some(Some(pactrail_store::MIN_EVENT_DATABASE_SCHEMA_VERSION))
    );
    assert_eq!(
        pactrail_memory::MemoryStore::database_schema_version(&memory).ok(),
        Some(Some(0))
    );
    let audited_settings = std::fs::read_to_string(&settings)
        .unwrap_or_else(|error| unreachable!("audited settings: {error}"));
    assert!(audited_settings.contains("schema = 1"));

    let upgrade = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args([
            "--workspace",
            path_text(workspace.path()),
            "--state-dir",
            path_text(&state),
            "upgrade",
            "--json",
        ])
        .env("PACTRAIL_CONFIG_DIR", &config)
        .output()
        .unwrap_or_else(|error| unreachable!("upgrade preflight: {error}"));
    assert!(
        upgrade.status.success(),
        "{}",
        String::from_utf8_lossy(&upgrade.stderr)
    );
    let upgrade: Value = serde_json::from_slice(&upgrade.stdout)
        .unwrap_or_else(|error| unreachable!("upgrade preflight JSON: {error}"));
    assert_eq!(upgrade["schema"], 1);
    assert_eq!(upgrade["changes_applied"], false);
    assert_eq!(upgrade["ready_for_current_version"], false);
    assert_eq!(upgrade["state"]["pending_components"], 3);
    assert_eq!(
        upgrade["deprecations"]["entries"].as_array().map(Vec::len),
        Some(2)
    );
    assert!(std::fs::read_to_string(&settings).is_ok_and(|text| text.contains("schema = 1")));

    let apply = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args([
            "--workspace",
            path_text(workspace.path()),
            "--state-dir",
            path_text(&state),
            "migrate",
            "--apply",
            "--json",
        ])
        .env("PACTRAIL_CONFIG_DIR", &config)
        .output()
        .unwrap_or_else(|error| unreachable!("migration apply: {error}"));
    assert!(
        apply.status.success(),
        "{}",
        String::from_utf8_lossy(&apply.stderr)
    );
    let apply: Value = serde_json::from_slice(&apply.stdout)
        .unwrap_or_else(|error| unreachable!("migration apply JSON: {error}"));
    assert_eq!(apply["pending_components"], 3);
    assert_eq!(apply["changed_components"], 3);
    assert!(apply["components"].as_array().is_some_and(|components| {
        components
            .iter()
            .all(|component| component["status"] == "current")
    }));

    let ready = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args([
            "--workspace",
            path_text(workspace.path()),
            "--state-dir",
            path_text(&state),
            "upgrade",
            "--json",
        ])
        .env("PACTRAIL_CONFIG_DIR", &config)
        .output()
        .unwrap_or_else(|error| unreachable!("ready upgrade preflight: {error}"));
    assert!(ready.status.success());
    let ready: Value = serde_json::from_slice(&ready.stdout)
        .unwrap_or_else(|error| unreachable!("ready upgrade JSON: {error}"));
    assert_eq!(ready["ready_for_current_version"], true);
    assert_eq!(ready["state"]["pending_components"], 0);
    let settings_path = apply["components"]
        .as_array()
        .and_then(|components| {
            components
                .iter()
                .find(|component| component["id"] == "interactive_settings")
        })
        .and_then(|component| component["path"].as_str())
        .unwrap_or_else(|| unreachable!("settings component path"));
    let migrated_settings = std::fs::read_to_string(settings_path)
        .unwrap_or_else(|error| unreachable!("migrated settings: {error}"));
    assert!(migrated_settings.contains("schema = 4"));
    assert_eq!(
        pactrail_store::EventStore::database_schema_version(events).ok(),
        Some(Some(pactrail_store::EVENT_DATABASE_SCHEMA_VERSION))
    );
    assert_eq!(
        pactrail_memory::MemoryStore::database_schema_version(memory).ok(),
        Some(Some(pactrail_memory::MEMORY_DATABASE_SCHEMA_VERSION))
    );
}

#[test]
fn memory_lifecycle_is_scriptable_with_an_external_state_directory() {
    let workspace = tempfile::tempdir().unwrap_or_else(|error| unreachable!("workspace: {error}"));
    let add = pactrail_with_state(
        workspace.path(),
        [
            "memory",
            "add",
            "Run cargo fmt before committing",
            "--title",
            "Formatting convention",
            "--kind",
            "convention",
            "--tag",
            "rust",
            "--json",
        ],
    );
    assert!(add.status.success());
    let added: Value = serde_json::from_slice(&add.stdout)
        .unwrap_or_else(|error| unreachable!("memory add JSON: {error}"));
    let memory_id = added["id"]
        .as_str()
        .unwrap_or_else(|| unreachable!("memory id missing"));

    let human_list = pactrail_with_state(workspace.path(), ["memory", "list"]);
    assert!(human_list.status.success());
    assert!(String::from_utf8_lossy(&human_list.stdout).contains(memory_id));

    let search = pactrail_with_state(
        workspace.path(),
        ["memory", "search", "cargo fmt", "--json"],
    );
    assert!(search.status.success());
    let matches: Value = serde_json::from_slice(&search.stdout)
        .unwrap_or_else(|error| unreachable!("memory search JSON: {error}"));
    assert_eq!(matches[0]["memory"]["id"], memory_id);

    let prefix = memory_id
        .get(..13)
        .unwrap_or_else(|| unreachable!("memory UUID was too short"));
    let forget = pactrail_with_state(workspace.path(), ["memory", "forget", prefix, "--json"]);
    assert!(forget.status.success());
    let forgotten: Value = serde_json::from_slice(&forget.stdout)
        .unwrap_or_else(|error| unreachable!("memory forget JSON: {error}"));
    assert_eq!(forgotten["forgotten"], memory_id);

    let list = pactrail_with_state(workspace.path(), ["memory", "list", "--json"]);
    assert!(list.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&list.stdout).ok(),
        Some(json!([]))
    );
    assert!(
        workspace
            .path()
            .join("state")
            .join("memory.sqlite3")
            .is_file()
    );
    assert!(!workspace.path().join(".pactrail").exists());
}

#[test]
fn incomplete_oci_configuration_fails_without_creating_run_state() {
    let workspace = tempfile::tempdir().unwrap_or_else(|error| unreachable!("workspace: {error}"));
    let output = pactrail(
        workspace.path(),
        [
            "run",
            "Execute the test suite",
            "--provider",
            "ollama",
            "--model",
            "local-model",
            "--process-backend",
            "oci",
        ],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("--process-backend oci requires --sandbox-image <local-image>")
    );
    assert!(!workspace.path().join(".pactrail").exists());
}

#[test]
fn noninteractive_process_approval_is_explicit_and_durable() {
    let allowed_workspace =
        tempfile::tempdir().unwrap_or_else(|error| unreachable!("workspace: {error}"));
    let allowed = run_process_approval_fixture(allowed_workspace.path(), true);
    assert!(
        allowed.status.success(),
        "approved run failed: {}",
        String::from_utf8_lossy(&allowed.stderr)
    );
    let allowed: Value = serde_json::from_slice(&allowed.stdout)
        .unwrap_or_else(|error| unreachable!("approved run JSON: {error}"));
    assert_eq!(allowed["approvals"][0]["decision"], "allow_run");
    assert_eq!(
        allowed["approvals"][0]["binding"]["capability"],
        "process_spawn"
    );
    assert_eq!(
        allowed["approvals"][0]["binding"]["backend_kind"],
        "native_trusted"
    );
    let run_id = allowed["run_id"]
        .as_str()
        .unwrap_or_else(|| unreachable!("approved run id missing"));
    let trace = pactrail(allowed_workspace.path(), ["trace", run_id, "--json"]);
    assert!(trace.status.success());
    let trace: Value = serde_json::from_slice(&trace.stdout)
        .unwrap_or_else(|error| unreachable!("approval trace JSON: {error}"));
    let event_types = trace
        .as_array()
        .unwrap_or_else(|| unreachable!("trace is not an array"))
        .iter()
        .filter_map(|event| event["event"]["type"].as_str())
        .collect::<Vec<_>>();
    let policy_index = event_types
        .iter()
        .position(|kind| *kind == "policy_evaluated")
        .unwrap_or_else(|| unreachable!("policy event missing"));
    let approval_index = event_types
        .iter()
        .position(|kind| *kind == "approval_decided")
        .unwrap_or_else(|| unreachable!("approval event missing"));
    assert!(policy_index < approval_index);

    let denied_workspace =
        tempfile::tempdir().unwrap_or_else(|error| unreachable!("workspace: {error}"));
    let denied = run_process_approval_fixture(denied_workspace.path(), false);
    assert!(
        denied.status.success(),
        "denied process should remain a model-visible tool result: {}",
        String::from_utf8_lossy(&denied.stderr)
    );
    let denied: Value = serde_json::from_slice(&denied.stdout)
        .unwrap_or_else(|error| unreachable!("denied run JSON: {error}"));
    assert_eq!(denied["approvals"][0]["decision"], "deny");
}

fn run_process_approval_fixture(workspace: &Path, allow_run: bool) -> std::process::Output {
    let listener = TcpListener::bind("127.0.0.1:0")
        .unwrap_or_else(|error| unreachable!("mock provider: {error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| unreachable!("provider address: {error}"));
    let responses = [
        json!({
            "id": "process-turn",
            "choices": [{
                "message": {
                    "content": "",
                    "tool_calls": [{
                        "id": "process-1",
                        "type": "function",
                        "function": {
                            "name": "run_process",
                            "arguments": "{\"program\":\"cargo\",\"args\":[\"--version\"]}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 20, "completion_tokens": 8}
        }),
        json!({
            "id": "process-summary",
            "choices": [{
                "message": {"content": "The process request was handled by Pactrail policy."},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 24, "completion_tokens": 9}
        }),
    ];
    let server = thread::spawn(move || serve_responses(&listener, &responses));
    let mut command = Command::new(env!("CARGO_BIN_EXE_pactrail"));
    command.args([
        "--workspace",
        path_text(workspace),
        "run",
        "Report the Cargo version",
        "--provider",
        "open-ai-compatible",
        "--base-url",
        &format!("http://{address}/v1"),
        "--model",
        "mock-coder",
        "--no-stream",
        "--process-backend",
        "native",
        "--output",
        "json",
    ]);
    if allow_run {
        command.args(["--process-approval", "allow-run"]);
    }
    let output = command
        .output()
        .unwrap_or_else(|error| unreachable!("process approval run: {error}"));
    server
        .join()
        .unwrap_or_else(|_| unreachable!("provider thread panicked"));
    output
}

fn pactrail<const N: usize>(workspace: &Path, arguments: [&str; N]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .arg("--workspace")
        .arg(path_text(workspace))
        .args(arguments)
        .output()
        .unwrap_or_else(|error| unreachable!("pactrail command: {error}"))
}

fn assert_cross_workspace_run_commands_fail(bound_workspace: &Path, run_id: &str) {
    let foreign_workspace =
        tempfile::tempdir().unwrap_or_else(|error| unreachable!("foreign workspace: {error}"));
    let state = bound_workspace.join(".pactrail");
    for command in ["inspect", "diff", "apply", "discard"] {
        let rejected = Command::new(env!("CARGO_BIN_EXE_pactrail"))
            .args([
                "--workspace",
                path_text(foreign_workspace.path()),
                "--state-dir",
                path_text(&state),
                command,
                run_id,
                "--json",
            ])
            .output()
            .unwrap_or_else(|error| unreachable!("foreign {command}: {error}"));
        assert!(!rejected.status.success(), "foreign {command} was accepted");
        assert!(
            String::from_utf8_lossy(&rejected.stderr).contains("not the selected workspace"),
            "foreign {command} did not explain the workspace binding: {}",
            String::from_utf8_lossy(&rejected.stderr)
        );
    }
}

fn pactrail_with_state<const N: usize>(
    workspace: &Path,
    arguments: [&str; N],
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .arg("--workspace")
        .arg(path_text(workspace))
        .args(["--state-dir", "state"])
        .args(arguments)
        .output()
        .unwrap_or_else(|error| unreachable!("pactrail state command: {error}"))
}

fn serve_model(listener: &TcpListener) {
    let responses = [
        json!({
            "id": "turn-1",
            "choices": [{
                "message": {
                    "content": "",
                    "tool_calls": [{
                        "id": "write-1",
                        "type": "function",
                        "function": {
                            "name": "write_file",
                            "arguments": "{\"path\":\"README.md\",\"content\":\"# Created by Pactrail\\n\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 20, "completion_tokens": 8}
        }),
        json!({
            "id": "turn-2",
            "choices": [{
                "message": {"content": "Created README.md in the isolated transaction."},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 25, "completion_tokens": 7}
        }),
    ];
    serve_responses(listener, &responses);
}

fn serve_responses(listener: &TcpListener, responses: &[Value]) {
    for response in responses {
        let (mut stream, _address) = listener
            .accept()
            .unwrap_or_else(|error| unreachable!("provider accept: {error}"));
        read_request(&mut stream);
        write_response(&mut stream, response);
    }
}

fn write_response(stream: &mut TcpStream, response: &Value) {
    let body = serde_json::to_vec(response)
        .unwrap_or_else(|error| unreachable!("provider response: {error}"));
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .and_then(|()| stream.write_all(&body))
        .unwrap_or_else(|error| unreachable!("provider write: {error}"));
}

fn read_request(stream: &mut TcpStream) {
    drop(capture_request(stream));
}

fn capture_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let mut expected = None;
    loop {
        let count = stream
            .read(&mut buffer)
            .unwrap_or_else(|error| unreachable!("provider read: {error}"));
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if expected.is_none()
            && let Some(header_end) = find_bytes(&bytes, b"\r\n\r\n")
        {
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.strip_prefix("content-length: ")
                        .or_else(|| line.strip_prefix("Content-Length: "))
                })
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or_default();
            expected = Some(header_end + 4 + content_length);
        }
        if expected.is_some_and(|length| bytes.len() >= length) {
            break;
        }
    }
    assert!(bytes.starts_with(b"POST /v1/chat/completions HTTP/1.1"));
    bytes
}

fn tiny_png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(&13_u32.to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 2, 0, 0, 0]);
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&0_u32.to_be_bytes());
    bytes.extend_from_slice(b"IEND");
    bytes.extend_from_slice(&[0; 4]);
    bytes
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn path_text(path: &Path) -> &str {
    path.to_str()
        .unwrap_or_else(|| unreachable!("temporary path is not Unicode"))
}
