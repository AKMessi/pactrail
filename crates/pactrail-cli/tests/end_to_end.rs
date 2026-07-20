use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

use serde_json::{Value, json};

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

    let apply = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args([
            "--workspace",
            path_text(workspace.path()),
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
        std::fs::read_to_string(workspace.path().join("README.md")).ok(),
        Some("# Created by Pactrail\n".to_owned())
    );
    assert_eq!(
        std::fs::read_to_string(&review_path).ok(),
        Some(review_before)
    );
    let applied: Value = serde_json::from_slice(&apply.stdout)
        .unwrap_or_else(|error| unreachable!("apply JSON: {error}"));
    assert_eq!(applied["outcome"], "applied");

    let repeated = Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .args([
            "--workspace",
            path_text(workspace.path()),
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

    assert_applied_memory(workspace.path(), run_id);
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
    assert_eq!(tools.len(), 12);
    assert!(tools.iter().any(|tool| tool["name"] == "run_process"));
    let graph = tools
        .iter()
        .find(|tool| tool["name"] == "search_code_graph")
        .unwrap_or_else(|| unreachable!("repository evidence graph tool was not registered"));
    assert_eq!(graph["required_capability"], "file_read");
    assert_eq!(graph["annotations"]["read_only"], true);

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

fn pactrail<const N: usize>(workspace: &Path, arguments: [&str; N]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pactrail"))
        .arg("--workspace")
        .arg(path_text(workspace))
        .args(arguments)
        .output()
        .unwrap_or_else(|error| unreachable!("pactrail command: {error}"))
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
}

fn read_request(stream: &mut TcpStream) {
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
