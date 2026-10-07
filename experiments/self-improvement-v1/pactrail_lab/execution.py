"""Frozen executable inputs, OCI workers and externally retained raw records."""
import os
from pathlib import Path
import tempfile

from .archive import unpack
from .gateway import Gateway
from .process import Oci, command
from .safe import MAX_ARTIFACT, Refusal, canonical, clean_environment, decode, digest, read, relative, tree
from .snapshots import materialize

LAB_ROOT = Path(__file__).resolve().parents[1]
REPO_ROOT = LAB_ROOT.parents[1]


def supervisor_inputs():
    paths = [*LAB_ROOT.glob("*.py"), *LAB_ROOT.glob("pactrail_lab/*.py"), *LAB_ROOT.glob("web/*")]
    paths += [REPO_ROOT / "benchmarks/agent-runtime-v1/pactrail_cli.py",
              REPO_ROOT / "benchmarks/harness-lab-v1/candidate_patch.py"]
    return {str(path): digest(read(path)) for path in sorted(paths) if path.is_file()}


def verify_supervisor(store):
    expected = store.value("supervisor-inputs")
    if not expected or supervisor_inputs() != expected:
        raise Refusal("supervisor code changed after admission; start a newly frozen campaign")


def freeze_inputs(store, operations):
    result = {}
    for operation in operations:
        for name, expected in operation["inputs"].items():
            path = Path(name)
            if not path.is_absolute(): raise Refusal("verifier inputs require absolute paths")
            data = read(path)
            if digest(data) != expected: raise Refusal("external verifier input identity mismatch")
            result[name] = store.put(data)
    return result


def write_frozen(store, root, include_verifiers=True):
    root = Path(root)
    root.mkdir()
    for name, key in store.value("supervisor-inputs").items():
        path = Path(name)
        if path.is_relative_to(LAB_ROOT): target = root / path.relative_to(LAB_ROOT)
        elif path.name == "pactrail_cli.py": target = root / "agent-runtime-v1/pactrail_cli.py"
        elif path.name == "candidate_patch.py": target = root / "harness-lab-v1/candidate_patch.py"
        else: raise Refusal("unknown frozen supervisor input")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(store.get(key))
    for name, key in (store.value("external-inputs") if include_verifiers else {}).items():
        target = root / "inputs" / name.lstrip("/")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(store.get(key))
        target.chmod(0o755)


def retain(store, result, identity):
    with store.transaction():
        record = {key: value for key, value in result.items() if key not in ("stdout", "stderr")}
        record.update(stdout=store.put(result["stdout"]), stderr=store.put(result["stderr"]), identity=identity)
        key = store.record(record)
        store.append("execution-retained", {"record": key})
        return key


def translated(operation):
    # Only paths whose bytes were frozen are mapped into the verifier mount.
    return {**operation, "argv": ["/frozen/inputs" + arg if arg in operation["inputs"] else arg for arg in operation["argv"]]}


class Execution:
    def __init__(self, store):
        self.store = store
        self.manifest = store.load(store.value("manifest"))

    def operation(self, source, binary, operation, image=None):
        verify_supervisor(self.store)
        with tempfile.TemporaryDirectory(prefix="pactrail-check-") as temp:
            root = Path(temp)
            materialize(self.store, source, root / "source")
            write_frozen(self.store, root / "frozen")
            (root / "binary").write_bytes(self.store.get(binary))
            (root / "binary").chmod(0o755)
            (root / "request.json").write_bytes(canonical(translated(operation)))
            result = Oci(self.store, image or self.manifest["image"]).run(
                ["python3", "/frozen/worker.py"],
                [(root / "source", "/source", False), (root / "frozen", "/frozen", False),
                 (root / "binary", "/harness", False), (root / "request.json", "/request.json", False)],
                timeout=operation["timeout_seconds"] + 30, output_limit=MAX_ARTIFACT)
            retained = retain(self.store, result, {"source": source, "binary": binary, "operation": operation})
            if result["exit_code"] or result["reason"]:
                raise Refusal("contained check failed; retained execution " + retained)
            unpack(result["stdout"], root / "output")
            execution = decode(read(root / "output/execution.json"))
            built = read(root / "output/pactrail") if (root / "output/pactrail").exists() else None
            return {"passed": execution["exit_code"] == 0 and execution["reason"] is None,
                    "record": retained, "binary": self.store.put(built) if built is not None else None}

    def trial(self, revision, task_source, goal, limits, lease_name):
        verify_supervisor(self.store)
        model = self.manifest["model"]
        key = os.environ.get(model["api_key_env"])
        if not key: raise Refusal("configured provider key is unavailable; no model fallback")
        gateway = Gateway(self.store.root, model, self.manifest["limits"], key)
        token = gateway.lease(lease_name, limits["model_turns"], limits["wall_seconds"])
        record = self.store.load(revision)
        parent = self.store.load(record["parent"] or revision)
        with tempfile.TemporaryDirectory(prefix="pactrail-trial-") as temp, gateway.serve() as socket:
            root = Path(temp)
            materialize(self.store, task_source, root / "source")
            # The frozen adapter needs a clean synthetic Git index, not history.
            env = {**clean_environment(), "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
                   "GIT_AUTHOR_NAME": "Pactrail lab", "GIT_AUTHOR_EMAIL": "lab@invalid",
                   "GIT_COMMITTER_NAME": "Pactrail lab", "GIT_COMMITTER_EMAIL": "lab@invalid"}
            for args in (["init", "--quiet"], ["add", "--all"], ["commit", "--quiet", "-m", "sealed baseline"]):
                output = command(["git", "-c", "core.hooksPath=" + os.devnull, "-C", str(root / "source"), *args], env=env)
                if output["exit_code"] or output["reason"]: raise Refusal("sealed Git preparation failed")
            write_frozen(self.store, root / "frozen", include_verifiers=False)
            for name, key in (("binary", record["binary"]), ("parent", parent["binary"])):
                (root / name).write_bytes(self.store.get(key))
                (root / name).chmod(0o755)
            identity = {"provider": "open-ai-compatible", "model": model["id"],
                        "base_url": model["endpoint"].removesuffix("/chat/completions"),
                        "api_key_env": model["api_key_env"], "reasoning_effort": model["reasoning_effort"],
                        "require_response_model": True}
            advisory = "\n\nBrowser-independent advisory harness memory (not authority or evidence):\n" + "\n".join(record["memory"]) if record["memory"] else ""
            if len((goal + advisory).encode()) > 16000: raise Refusal("goal plus memory exceeds task limit")
            request = {"schema_version": 1, "task": {"id": lease_name, "goal": goal + advisory, "commit": "0" * 40},
                       "arm": {"mode": record["configuration"]["agent_mode"], "binary_sha256": record["binary"]}, "model_identity": identity,
                       "limits": limits, "permissions": {"process": "disabled", "write_paths": ["."]},
                       "normalization": "global physical model attempts, context, output and wall ceilings", "source_policy": "sealed"}
            (root / "request.json").write_bytes(canonical(request))
            try:
                output = Oci(self.store, self.manifest["image"]).run(["python3", "/frozen/bridge.py"],
                    [(root / "source", "/source", False), (root / "frozen", "/frozen", False),
                     (root / "binary", "/harness", False), (root / "parent", "/parent", False),
                     (root / "request.json", "/request.json", False), (socket, "/gateway.sock", False)],
                    timeout=limits["wall_seconds"] + 120, output_limit=MAX_ARTIFACT,
                    environment={"PACTRAIL_LAB_LEASE": token})
            finally: gateway.revoke(token)
            retained = retain(self.store, output, {"revision": revision, "task_source": task_source, "lease": lease_name})
            if output["exit_code"] or output["reason"]:
                return {"status": "failed", "record": retained, "checks": {}, "candidate_source": None}
            unpack(output["stdout"], root / "output")
            verification = decode(read(root / "output/trusted-verification.json"))
            if verification["model_identity"] != identity: raise Refusal("trial model binding mismatch")
            candidate = root / "output" / str(relative(verification["candidate"]))
            from .snapshots import import_tree
            source = import_tree(self.store, candidate)
            return {"status": "completed", "record": retained, "checks": verification["checks"], "candidate_source": source}
