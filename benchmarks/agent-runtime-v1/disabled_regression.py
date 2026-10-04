#!/usr/bin/env python3
"""Compare disabled-agent provider requests and governed effects against v2.0."""
import argparse
import hashlib
import http.server
import json
from pathlib import Path
import re
import subprocess
import tempfile
import threading


class Provider(http.server.BaseHTTPRequestHandler):
    requests = []

    def log_message(self, *_args):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        self.requests.append(request)
        if any(m["role"] == "tool" for m in request["messages"]):
            message, finish = {"content": "Updated; checks were not run."}, "stop"
        else:
            message, finish = {"content": "", "tool_calls": [{"id": "write", "type": "function", "function": {"name": "write_file", "arguments": json.dumps({"path": "value.txt", "content": "after"})}}]}, "tool_calls"
        body = json.dumps({"model": "disabled-regression", "choices": [{"message": message, "finish_reason": finish}], "usage": {"prompt_tokens": 100, "completion_tokens": 10}}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def run(baseline, current, output):
    output.mkdir(parents=True, exist_ok=False)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Provider)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    rows, normalized = [], []
    try:
        with tempfile.TemporaryDirectory(prefix="pactrail-disabled-regression-") as directory:
            for name, binary in [("v2.0", baseline), ("candidate", current)]:
                workspace = Path(directory) / name
                workspace.mkdir()
                (workspace / "value.txt").write_text("before")
                Provider.requests = []
                command = [str(binary), "--workspace", str(workspace), "run", "Change value.txt to after", "--provider", "open-ai-compatible", "--model", "disabled-regression", "--base-url", f"http://127.0.0.1:{server.server_port}/v1", "--api-key-env", "DISABLED_REGRESSION_NO_KEY", "--no-stream", "--output", "json"]
                completed = subprocess.run(command, capture_output=True, check=False)
                (output / (name + ".stdout")).write_bytes(completed.stdout)
                (output / (name + ".stderr")).write_bytes(completed.stderr)
                if completed.returncode:
                    raise ValueError("disabled single-agent run failed: " + name)
                result = json.loads(completed.stdout)
                receipt = json.loads(Path(result["receipt"]).read_text())
                (output / (name + "-receipt.json")).write_text(json.dumps(receipt, indent=2) + "\n")
                (output / (name + "-requests.json")).write_text(json.dumps(Provider.requests, indent=2) + "\n")
                canonical = re.sub(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", "RUN_UUID", json.dumps(Provider.requests, sort_keys=True))
                canonical = canonical.replace(str(workspace), "WORKSPACE")
                normalized.append(canonical)
                rows.append({"version": name, "source_unchanged": (workspace / "value.txt").read_text() == "before", "candidate_correct": (Path(result["receipt"]).parent / "workspace/value.txt").read_text() == "after", "outcome": receipt["outcome"], "requests": len(Provider.requests), "normalized_requests_sha256": hashlib.sha256(canonical.encode()).hexdigest()})
        report = {"cases": rows, "provider_requests_equal": normalized[0] == normalized[1], "scope": "identical scripted provider; ordinary single-agent defaults; dynamic UUID/workspace paths normalized"}
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        if not report["provider_requests_equal"] or any(not r["source_unchanged"] or not r["candidate_correct"] or r["outcome"] != "ready_to_apply" for r in rows):
            raise ValueError("disabled-agent regression differs; inspect retained requests")
    finally:
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--current", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    run(args.baseline.resolve(), args.current.resolve(), args.output.resolve())
