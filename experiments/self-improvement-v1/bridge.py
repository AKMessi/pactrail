"""Frozen container HTTP shim. It holds a scoped lease, never a provider key."""
import http.server
import json
import os
from pathlib import Path
import socket
import sys
import threading
from pactrail_lab.process import command
from pactrail_lab.archive import pack
from pactrail_lab.safe import canonical, decode, read, Refusal
from verifier import verify


class Forward(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def do_POST(self):
        length = int(self.headers.get("Content-Length", "-1"))
        if self.path != "/v1/chat/completions" or not 1 <= length <= 1048576 or self.headers.get("Transfer-Encoding"):
            self.send_error(400)
            return
        self.connection.settimeout(130)
        body = self.rfile.read(length)
        token = os.environ["PACTRAIL_LAB_LEASE"]
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as upstream:
            upstream.settimeout(130)
            upstream.connect("/gateway.sock")
            header = ("POST /v1/chat/completions HTTP/1.0\r\nHost: gateway\r\nContent-Type: application/json\r\n"
                      f"Authorization: Bearer {token}\r\nContent-Length: {len(body)}\r\n\r\n").encode()
            upstream.sendall(header + body)
            total = 0
            while data := upstream.recv(65536):
                total += len(data)
                if total > 8_454_144:
                    raise ValueError("gateway response exceeds limit")
                self.connection.sendall(data)


def main():
    request = decode(read("/request.json"))
    original_identity = dict(request["model_identity"])
    server = http.server.HTTPServer(("127.0.0.1", 0), Forward)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    request["model_identity"]["base_url"] = f"http://127.0.0.1:{server.server_port}/v1"
    request["model_identity"]["api_key_env"] = "PACTRAIL_LAB_LEASE"
    request["workspace"] = "/source"
    request["trial_directory"] = "/work"
    request["runtime_identity"] = {"binary_sha256": request["arm"]["binary_sha256"]}
    Path("/work/request.json").write_text(json.dumps(request))
    environment = {"PATH": os.environ["PATH"], "HOME": "/tmp", "PACTRAIL_BENCH_BINARY": "/harness",
                   "PACTRAIL_LAB_LEASE": os.environ["PACTRAIL_LAB_LEASE"], "PYTHONDONTWRITEBYTECODE": "1"}
    try:
        result = command(["python3", "/frozen/agent-runtime-v1/pactrail_cli.py", "/work/request.json"],
                         env=environment, timeout=request["limits"]["wall_seconds"], limit=1_048_576)
        Path("/work/adapter.stdout").write_bytes(result["stdout"])
        Path("/work/adapter.stderr").write_bytes(result["stderr"])
        Path("/work/execution.json").write_bytes(canonical({k: v for k, v in result.items() if k not in ("stdout", "stderr")}))
        verification = verify("/work")
        if result["exit_code"] or result["reason"]:
            verification["checks"]["ready_to_apply"] = False
        verification["model_identity"] = original_identity
        Path("/work/trusted-verification.json").write_bytes(canonical(verification))
        sys.stdout.buffer.write(pack("/work"))
        return 0
    finally:
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    try: sys.exit(main())
    except (Refusal, OSError, ValueError): sys.exit(65)
