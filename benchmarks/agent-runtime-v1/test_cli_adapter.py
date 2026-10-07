"""Protocol smoke test, not an inference or task-performance benchmark."""
import http.server
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest

spec = importlib.util.spec_from_file_location("runner", Path(__file__).with_name("run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        roles = ("localizer", "solver", "critic")
        messages = request["messages"]
        specialist = any(f"Pactrail agent: {role}." in str(message.get("content", "")) for role in roles for message in messages)
        if specialist:
            message, finish = {"content": "Fixture advice, not evidence."}, "stop"
        elif any(message["role"] == "tool" for message in messages):
            message, finish = {"content": "Updated the fixture; tests not run."}, "stop"
        else:
            message = {"content": "", "tool_calls": [{"id": "write-fixture", "type": "function",
                "function": {"name": "write_file", "arguments": json.dumps({"path": "value.txt", "content": "after"})}}]}
            finish = "tool_calls"
        body = json.dumps({"id": "fixture-only", "choices": [{"message": message, "finish_reason": finish}],
                           "usage": {"prompt_tokens": 100, "completion_tokens": 10}}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


@unittest.skipUnless(os.environ.get("PACTRAIL_BENCH_BINARY"), "set PACTRAIL_BENCH_BINARY for real CLI smoke")
class CliAdapterTests(unittest.TestCase):
    def test_single_text_and_explicitly_unsupported_latent(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.mkdir()
            (source / "value.txt").write_text("before")
            subprocess.run(["git", "init", str(source)], capture_output=True, check=True)
            subprocess.run(["git", "-C", str(source), "add", "."], check=True)
            subprocess.run(["git", "-C", str(source), "-c", "user.name=Fixture", "-c", "user.email=fixture@invalid", "commit", "-m", "fixture"], capture_output=True, check=True)
            commit = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
            server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Provider)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                grader = [sys.executable, "-c", "from pathlib import Path; assert Path('value.txt').read_text() == 'after'"]
                protocol = {"schema_version": 1, "seed": 42, "repetitions": 1, "source_policy": "sealed",
                    "model_identity": {"provider": "open-ai-compatible", "model": "fixture-only", "base_url": f"http://127.0.0.1:{server.server_port}/v1", "api_key_env": "BENCH_FIXTURE_NO_KEY"},
                    "limits": {"model_turns": 24, "wall_seconds": 20, "output_tokens": 1024, "context_tokens": 32768, "model_tokens": 786432},
                    "permissions": {"process": "disabled", "write_paths": ["."]}, "normalization": "equal declared ceilings",
                    "tasks": [{"id": "fixture", "repository": str(source), "commit": commit, "goal": "Change value.txt to after.", "targeted": grader, "regression": grader}],
                    "arms": [{"id": mode, "mode": mode, "adapter": [sys.executable, str(Path(__file__).with_name("pactrail_cli.py").resolve())]} for mode in ("single", "text", "latent")]}
                path = root / "protocol.json"
                runner.write(path, protocol)
                rows = {row["arm"]: row for row in runner.run(path, root / "results")}
                self.assertTrue(rows["single"]["task_success"], rows["single"])
                self.assertTrue(rows["text"]["task_success"], rows["text"])
                for mode in ("single", "text"):
                    self.assertTrue(rows[mode]["checks"]["source_isolation_valid"])
                    provenance = rows[mode]["provenance"]
                    self.assertEqual(provenance["source_tree_before_sha256"], provenance["source_tree_after_sha256"])
                    self.assertIsNotNone(provenance["source_tree_before_sha256"])
                self.assertEqual(rows["single"]["metrics"]["model_turns"], 2)
                self.assertEqual(rows["text"]["metrics"]["model_turns"], 5)
                self.assertIsNone(rows["text"]["metrics"]["inter_agent_text_tokens"])
                self.assertEqual(rows["latent"]["status"], "unsupported")
                self.assertEqual((source / "value.txt").read_text(), "before")
            finally:
                server.shutdown()
                server.server_close()
                thread.join()


if __name__ == "__main__":
    unittest.main()
