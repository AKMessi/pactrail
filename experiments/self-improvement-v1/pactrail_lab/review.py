"""Loopback human controls: exact head/revision binding and origin checks."""
import difflib
import http.server
from pathlib import Path
import secrets
from urllib.parse import parse_qs, urlsplit

from .campaign import Campaign, status
from .execution import LAB_ROOT, verify_supervisor
from .safe import Refusal, canonical, decode, hash_id, integer, read
from .snapshots import diff
from .store import Store


def authorize(host, origin, cookie, expected_host, session, *, mutation=False):
    if host != expected_host:
        raise Refusal("unexpected Host; loopback binding is required")
    if origin is not None and origin != "http://" + expected_host:
        raise Refusal("cross-origin review request refused")
    if mutation and origin != "http://" + expected_host:
        raise Refusal("review mutation requires an exact loopback Origin")
    from http.cookies import SimpleCookie
    cookies = SimpleCookie()
    try: cookies.load(cookie or "")
    except Exception: raise Refusal("invalid review session") from None
    token = cookies.get("pactrail_lab")
    if token is None or not secrets.compare_digest(token.value, session):
        raise Refusal("review session is unavailable")


def change_view(store, key):
    hash_id(key)
    if key not in store.value("lineage"):
        raise Refusal("revision is not in this campaign")
    after = store.load(key)
    if after["parent"] is None: return {"changes": [], "patch": "Baseline revision"}
    before = store.load(after["parent"])
    changes = diff(store, before["source"], after["source"])
    patches = []
    total = 0
    for change in changes:
        a, b = change["before"], change["after"]
        if (a and a["kind"] == "directory") or (b and b["kind"] == "directory"): continue
        first = store.get(a["digest"]) if a else b""
        second = store.get(b["digest"]) if b else b""
        if len(first) + len(second) > 262144 or b"\0" in first + second:
            patches.append(change["path"] + ": binary or large diff; inspect exported source\n")
        else:
            patches.extend(difflib.unified_diff(first.decode("utf-8", "replace").splitlines(True),
                                               second.decode("utf-8", "replace").splitlines(True),
                                               "before/" + change["path"], "after/" + change["path"]))
        total = sum(len(p) for p in patches)
        if total > 524288:
            patches.append("Review preview truncated at 512 KiB; export the full revision.\n")
            break
    authority_paths = [row["path"] for row in changes if any(word in row["path"] for word in
        ("pactrail-core", "pactrail-workspace", "pactrail-store", "policy", "receipt", "recovery", "verification"))]
    return {"changes": changes, "patch": "".join(patches), "authority_paths": authority_paths,
            "configuration": after["configuration"], "memory": after["memory"],
            "proposal": store.load(after["proposal"]) if after["proposal"] else None}


def serve(root, port):
    integer(port, 1024, 65535, "review port")
    session = secrets.token_urlsafe(32)
    host = "127.0.0.1:" + str(port)
    static = {"/": ("index.html", "text/html; charset=utf-8"),
              "/app.js": ("app.js", "text/javascript; charset=utf-8"),
              "/app.css": ("app.css", "text/css; charset=utf-8")}
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_): pass
        def send(self, code, data, mime="application/json", cookie=False):
            self.send_response(code)
            self.send_header("Content-Type", mime)
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Security-Policy", "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.send_header("Referrer-Policy", "no-referrer")
            if cookie: self.send_header("Set-Cookie", "pactrail_lab=" + session + "; HttpOnly; SameSite=Strict; Path=/")
            self.end_headers()
            self.wfile.write(data)
        def do_GET(self):
            try:
                self.connection.settimeout(10)
                path = urlsplit(self.path)
                if self.headers.get("Host") != host or self.headers.get("Sec-Fetch-Site") == "cross-site":
                    raise Refusal("cross-site or unexpected-host review request")
                with Store(root) as store:
                    verify_supervisor(store)
                    if path.path in static and not path.query:
                        name, mime = static[path.path]
                        self.send(200, read(LAB_ROOT / "web" / name), mime, cookie=path.path == "/")
                        return
                    authorize(self.headers.get("Host"), self.headers.get("Origin"), self.headers.get("Cookie"), host, session)
                    if path.path == "/api/status": result = status(store)
                    elif path.path == "/api/change":
                        result = change_view(store, parse_qs(path.query, strict_parsing=True)["revision"][0])
                    elif path.path == "/api/record":
                        result = store.load(hash_id(parse_qs(path.query, strict_parsing=True)["digest"][0]))
                    else: raise Refusal("unknown review endpoint")
                self.send(200, canonical(result))
            except (Refusal, OSError, KeyError, ValueError):
                self.send(403, canonical({"error": "Review request refused; inspect campaign integrity and session."}))
        def do_POST(self):
            try:
                self.connection.settimeout(10)
                authorize(self.headers.get("Host"), self.headers.get("Origin"), self.headers.get("Cookie"), host, session, mutation=True)
                if self.headers.get("Transfer-Encoding") or self.headers.get("Content-Type") != "application/json":
                    raise Refusal("invalid review request encoding")
                size = integer(int(self.headers.get("Content-Length", "-1")), 1, 16384, "review request length")
                body = decode(self.rfile.read(size))
                expected = {"command_id", "head", "revision"} if self.path == "/api/undo" else {"command_id", "head", "verdict", "acknowledgment"}
                if set(body) != expected: raise Refusal("unknown or missing review fields")
                with Store(root) as store:
                    campaign = Campaign(store)
                    if self.path == "/api/approve":
                        result = campaign.approve(body["command_id"], body["head"], body["verdict"], body["acknowledgment"])
                    elif self.path == "/api/undo": result = campaign.undo(body["command_id"], body["head"], body["revision"])
                    else: raise Refusal("unknown mutation")
                self.send(200, canonical(result))
            except (Refusal, OSError, KeyError, TypeError, ValueError) as error:
                self.send(409, canonical({"error": str(error)}))
    # A serial server ensures this review surface cannot interleave operations.
    server = http.server.HTTPServer(("127.0.0.1", port), Handler)
    print("Pactrail lab review: http://" + host + " (approval selects the lab harness only)", flush=True)
    try: server.serve_forever()
    except KeyboardInterrupt: pass
    finally: server.server_close()
