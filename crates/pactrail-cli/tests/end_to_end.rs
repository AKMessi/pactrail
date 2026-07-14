use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
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
    let applied: Value = serde_json::from_slice(&apply.stdout)
        .unwrap_or_else(|error| unreachable!("apply JSON: {error}"));
    assert_eq!(applied["outcome"], "applied");
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
    for response in responses {
        let (mut stream, _address) = listener
            .accept()
            .unwrap_or_else(|error| unreachable!("provider accept: {error}"));
        read_request(&mut stream);
        let body = serde_json::to_vec(&response)
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
