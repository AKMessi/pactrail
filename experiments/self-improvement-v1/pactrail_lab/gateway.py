"""Text-only provider gateway outside candidate authority and credentials."""
import contextlib
import http.server
import multiprocessing
import os
from pathlib import Path
import secrets
import sqlite3
import socketserver
import tempfile
import threading
import time
import urllib.error
import urllib.request
import uuid

from . import budget
from .safe import MAX_JSON, Refusal, canonical, decode, integer, text
from .store import Store


def _provider_bytes(endpoint, key, encoded, sender):
    """Trusted host worker; a stalled DNS/TLS/body read can be terminated.

    Only bounded bytes cross back. Exceptions never serialize credentials,
    headers or upstream URLs into the campaign or child process diagnostics.
    """
    try:
        request = urllib.request.Request(endpoint, data=encoded, method="POST",
            headers={"Content-Type": "application/json", "Authorization": "Bearer " + key})
        class NoRedirect(urllib.request.HTTPRedirectHandler):
            def redirect_request(self, *_):
                raise Refusal("provider redirect refused")
        with urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect).open(request, timeout=120) as response:
            data = response.read(8_388_609)
        sender.send_bytes(b"S" + data if len(data) <= 8_388_608 else b"E")
    except Exception:
        sender.send_bytes(b"E")
    finally:
        sender.close()


def validate_request(body, config):
    allowed = {"model", "messages", "tools", "tool_choice", "max_tokens", "max_completion_tokens",
               "temperature", "stream", "reasoning_effort", "parallel_tool_calls", "response_format"}
    if not isinstance(body, dict) or set(body) - allowed or body.get("model") != config["id"]:
        raise Refusal("provider model/fields differ from frozen capability profile")
    if body.get("stream", False) is not False:
        raise Refusal("v1 gateway admits buffered requests only")
    if body.get("temperature", 0) != 0:
        raise Refusal("temperature differs from frozen configuration")
    if body.get("reasoning_effort") != config["reasoning_effort"]:
        raise Refusal("reasoning setting differs from frozen configuration")
    if "max_tokens" in body and "max_completion_tokens" in body:
        raise Refusal("ambiguous output ceiling")
    output = body.get("max_tokens", body.get("max_completion_tokens"))
    integer(output, 1, config["output_tokens"], "request output ceiling")
    messages = body.get("messages")
    if not isinstance(messages, list) or not 1 <= len(messages) <= 1024:
        raise Refusal("messages are missing or exceed bounds")
    for message in messages:
        if not isinstance(message, dict) or message.get("role") not in ("system", "user", "assistant", "tool"):
            raise Refusal("invalid message role")
        if set(message) - {"role", "content", "name", "tool_call_id", "tool_calls"}:
            raise Refusal("unsupported message fields; v1 admits text and function calls only")
        content = message.get("content")
        if content is not None and not isinstance(content, str):
            raise Refusal("v1 admits text only; multimodal billing is unsupported")
    # Byte-level upper bound for declared byte-tokenized text providers, with
    # overhead margin. This is intentionally pessimistic, not a usage report.
    if len(canonical(body)) + 64 * len(messages) > config["context_tokens"] - config["output_tokens"]:
        raise Refusal("request exceeds conservative text context reservation")
    if len(canonical(body)) > MAX_JSON:
        raise Refusal("oversized provider request")
    return body


class Gateway:
    def __init__(self, root, config, limits, api_key, transport=None):
        self.root, self.config, self.limits = root, config, limits
        if not isinstance(api_key, str) or not api_key or len(api_key) > 8192 or "\n" in api_key or "\r" in api_key:
            raise Refusal("provider credential is unavailable or invalid")
        self.key, self.transport = api_key, transport or self._provider
        self.leases, self.lock = {}, threading.Lock()

    def lease(self, name, requests, seconds):
        integer(requests, 1, self.limits["requests"], "lease request count")
        integer(seconds, 1, self.limits["wall_seconds"], "lease seconds")
        token = secrets.token_urlsafe(32)
        self.leases[token] = {"id": name, "requests": requests, "deadline_ns": time.time_ns() + seconds * 10**9}
        return token

    def revoke(self, token):
        self.leases.pop(token, None)

    def _provider(self, body):
        deadline = min(time.time_ns() + 120 * 10**9, self.network_deadline_ns)
        remaining = (deadline - time.time_ns()) / 10**9
        if remaining <= 0: raise Refusal("provider deadline exhausted")
        context = multiprocessing.get_context("spawn")
        receiver, sender = context.Pipe(duplex=False)
        child = context.Process(target=_provider_bytes,
            args=(self.config["endpoint"], self.key, canonical(body), sender), daemon=True)
        try:
            child.start()
            sender.close()
            remaining = max(0, (deadline - time.time_ns()) / 10**9)
            if not receiver.poll(remaining): raise Refusal("provider total deadline reached")
            frame = receiver.recv_bytes(8_388_609)
            if not frame.startswith(b"S"): raise Refusal("provider transport failed")
            return frame[1:]
        finally:
            sender.close()
            receiver.close()
            if child.pid is not None:
                if child.is_alive(): child.terminate()
                child.join(timeout=1)
                if child.is_alive():
                    child.kill()
                    child.join(timeout=1)
                child.close()

    def submit(self, token, body):
        with self.lock:
            if token not in self.leases:
                raise Refusal("unknown/revoked model lease")
            lease = self.leases[token]
            validate_request(body, self.config)
            body = {**body, "temperature": 0, "stream": False}
            request_id = "request-" + uuid.uuid4().hex
            with Store(self.root) as store:
                budget.reserve(store, request_id, body, self.config, self.limits, lease)
                request_key = store.put(canonical(body).replace(self.key.encode(), b"[redacted]"))
                with store.transaction(): store.append("model-request", {"id": request_id, "artifact": request_key})
                try:
                    self.network_deadline_ns = min(lease["deadline_ns"], store.value("deadline-ns", lease["deadline_ns"]))
                    data = self.transport(body)
                    result = decode(data, 8_388_608)
                    if not isinstance(result, dict) or result.get("model") != self.config["id"] or not isinstance(result.get("choices"), list) or not result["choices"]:
                        raise Refusal("provider response identity/protocol differs from frozen model")
                    usage = result.get("usage") or {}
                    observed = None
                    if "prompt_tokens" in usage and "completion_tokens" in usage:
                        observed = {"input_tokens": usage["prompt_tokens"], "output_tokens": usage["completion_tokens"]}
                        integer(observed["input_tokens"], 0, self.config["context_tokens"], "reported input")
                        integer(observed["output_tokens"], 0, self.config["output_tokens"], "reported output")
                        cached = (usage.get("prompt_tokens_details") or {}).get("cached_tokens")
                        if cached is not None:
                            integer(cached, 0, observed["input_tokens"], "reported cache usage")
                        observed["cached_input_tokens"] = cached
                    retained = store.put(data.replace(self.key.encode(), b"[redacted]"))
                    budget.settle(store, request_id, retained, observed)
                    return data.replace(self.key.encode(), b"[redacted]")
                except Exception:
                    # Never log upstream headers, credentials, or exception URLs.
                    with store.transaction(): store.append("model-failed", {"id": request_id, "reservation_retained": True})
                    raise Refusal("provider request failed; reservation retained, no fallback/replay") from None

    @contextlib.contextmanager
    def serve(self):
        gateway = self
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_): pass
            def do_POST(self):
                try:
                    if self.path != "/v1/chat/completions" or self.headers.get("Transfer-Encoding"):
                        raise Refusal("unsupported gateway endpoint/transfer encoding")
                    length = int(self.headers.get("Content-Length", "-1"))
                    integer(length, 1, MAX_JSON, "request bytes")
                    self.connection.settimeout(130)
                    token = self.headers.get("Authorization", "").removeprefix("Bearer ")
                    data = gateway.submit(token, decode(self.rfile.read(length)))
                    code = 200
                except (Refusal, ValueError, OSError, sqlite3.Error):
                    data, code = canonical({"error": {"message": "model gateway refused request; inspect supervisor journal"}}), 403
                self.send_response(code)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)
        class Server(socketserver.UnixStreamServer):
            def get_request(self):
                connection, address = super().get_request()
                connection.settimeout(130)
                return connection, address
        with tempfile.TemporaryDirectory(prefix="pactrail-model-") as directory:
            path = Path(directory) / "gateway.sock"
            server = Server(str(path), Handler)
            path.chmod(0o600)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try: yield path
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=5)
