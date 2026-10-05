#!/usr/bin/env python3
"""POSIX matched trials. Trusted adapters/graders are invoked without a shell."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import random
import re
import shutil
import signal
import subprocess
import time

METRICS = (
    "model_attempts", "model_turns", "input_tokens", "output_tokens",
    "cached_input_tokens", "inter_agent_text_tokens", "tool_calls",
    "model_time_ms", "inference_time_ms", "latent_logical_bytes",
    "latent_stored_bytes", "latent_messages", "communication_rounds",
    "peak_memory_bytes", "cost_microusd", "latent_import_bytes", "latent_export_bytes",
    "cache_creation_input_tokens", "uncached_input_tokens", "read_calls", "write_calls",
    "process_calls", "communication_messages", "communication_bytes", "files_changed",
    "lines_added", "lines_removed", "provider_errors", "timeouts", "recovery_events",
)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n")


def bounded_json(path):
    if path.stat().st_size > 1_048_576:
        raise ValueError(f"oversized JSON: {path}")
    return json.loads(path.read_text(), parse_constant=lambda x: (_ for _ in ()).throw(ValueError(x)))


def valid_id(value):
    return isinstance(value, str) and re.fullmatch(r"[a-z0-9][a-z0-9-]{0,63}", value)


def command(argv, cwd, output, timeout, env=None):
    if not argv or not all(isinstance(x, str) and x for x in argv):
        raise ValueError("commands must be non-empty argv arrays")
    started = time.monotonic()
    with (output / "stdout.txt").open("wb") as stdout, (output / "stderr.txt").open("wb") as stderr:
        child = subprocess.Popen(argv, cwd=cwd, stdout=stdout, stderr=stderr,
                                 env=env, start_new_session=True)
        timed_out = False
        try:
            code = child.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(child.pid, signal.SIGTERM)
            try:
                code = child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                code = child.wait()
        # An adapter must not leave background inference or commands running.
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    return {"exit_code": code, "timed_out": timed_out,
            "wall_time_ms": round((time.monotonic() - started) * 1000)}


def metrics(result):
    raw = result.get("metrics", {})
    if not isinstance(raw, dict) or set(raw) - set(METRICS):
        raise ValueError("unknown metrics")
    normalized = {}
    for key in METRICS:
        value = raw.get(key)
        if value is not None and (type(value) is not int or value < 0):
            raise ValueError(f"invalid metric: {key}")
        normalized[key] = value
    return normalized


def strict_completion(functional_success, checks):
    """Keep functional behavior distinct from Pactrail completion assurance.

    Missing assurance is unknown. This Pactrail-specific result must not become
    a universal harness score for adapters that cannot report these checks.
    """
    if not functional_success:
        return False
    if checks is None:
        return None
    if not isinstance(checks, dict):
        raise ValueError("assurance checks must be an object")
    values = [checks.get(key) for key in ("receipt_valid", "trace_valid", "source_isolation_valid", "ready_to_apply")]
    if any(value is not None and type(value) is not bool for value in values):
        raise ValueError("assurance checks must be booleans or unknown")
    if any(value is False for value in values):
        return False
    return True if all(value is True for value in values) else None


def validate(protocol):
    if protocol.get("schema_version") != 1 or not protocol.get("model_identity"):
        raise ValueError("schema 1 and pinned model identity required")
    if not protocol.get("permissions") or not protocol.get("normalization"):
        raise ValueError("explicit shared permissions and resource normalization required")
    if type(protocol.get("seed")) is not int or not 1 <= protocol.get("repetitions", 0) <= 100:
        raise ValueError("seed and bounded repetitions required")
    if protocol.get("source_policy", "historical") not in ("historical", "sealed"):
        raise ValueError("unknown source policy")
    limits = protocol.get("limits", {})
    for key, high in [("model_turns", 200), ("wall_seconds", 3600), ("output_tokens", 131072), ("context_tokens", 1048576), ("model_tokens", 1000000000)]:
        if type(limits.get(key)) is not int or not 1 <= limits[key] <= high:
            raise ValueError(f"invalid common limit: {key}")
    for collection in (protocol.get("arms", []), protocol.get("tasks", [])):
        if not collection or len(collection) > 100 or len({x["id"] for x in collection}) != len(collection):
            raise ValueError("empty, duplicate or oversized arms/tasks")
        if any(not valid_id(x.get("id")) for x in collection):
            raise ValueError("invalid trial identity")
    if limits["output_tokens"] >= limits["context_tokens"]:
        raise ValueError("output tokens must be below context capacity")
    if len(protocol["arms"]) * len(protocol["tasks"]) * protocol["repetitions"] > 10000:
        raise ValueError("trial count exceeds explicit runner ceiling")
    for arm in protocol["arms"]:
        if arm["mode"] not in ("single", "text", "latent"):
            raise ValueError("unsupported communication mode")
        intervention = arm.get("intervention", {"kind": "correct"})
        if intervention.get("kind") not in ("correct", "none", "zero", "random", "shuffled", "wrong_task"):
            raise ValueError("unsupported intervention")
        if arm["mode"] != "latent" and intervention["kind"] != "correct":
            raise ValueError("interventions require latent mode")
        if not isinstance(arm.get("adapter"), list) or not arm["adapter"]:
            raise ValueError("explicit adapter argv required")
    for task in protocol["tasks"]:
        if not re.fullmatch(r"[0-9a-f]{40}", task.get("commit", "")) or not task.get("goal"):
            raise ValueError("full Git commit and goal required")
        for key in ("targeted", "regression"):
            if not isinstance(task.get(key), list) or not task[key]:
                raise ValueError("external targeted and regression grader argv required")


def seal_workspace(workspace, expected_commit, env):
    """Remove clone metadata before an agent can observe a sealed source.

    A local clone normally adds origin, including an operator-side filesystem
    path. Sealed tasks expose neither that path nor additional reachable history.
    """
    git = ["git", "-c", "core.hooksPath=" + os.devnull, "-C", str(workspace)]
    def output(*args):
        return subprocess.check_output(git + list(args), env=env, timeout=30, text=True).strip()
    if output("rev-parse", "HEAD") != expected_commit or output("rev-list", "--all", "--count") != "1":
        raise ValueError("sealed workspace must contain only its synthetic baseline commit")
    for remote in output("remote").splitlines():
        subprocess.run(git + ["remote", "remove", remote], env=env, check=True, timeout=30,
                       stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if output("remote") or output("status", "--porcelain", "--untracked-files=all"):
        raise ValueError("sealed workspace must be clean with no remote")


def run(protocol_path, output):
    if os.name != "posix":
        raise ValueError("this runner requires POSIX process-group cancellation")
    protocol = bounded_json(protocol_path)
    validate(protocol)
    for path, expected in protocol.get("frozen_inputs", {}).items():
        if digest(Path(path)) != expected:
            raise ValueError("frozen experiment input changed: " + path)
    output.mkdir(parents=True, exist_ok=False)
    write(output / "protocol.json", protocol)
    programs = [a["adapter"] for a in protocol["arms"]] + [t[k] for t in protocol["tasks"] for k in ("targeted", "regression")]
    pinned = {}
    for argv in programs:
        for index, argument in enumerate(argv):
            path = Path(shutil.which(argument) or argument) if index == 0 else Path(argument)
            if path.is_file():
                pinned[str(path.resolve())] = digest(path)
    write(output / "provenance.json", {"protocol_sha256": digest(protocol_path),
          "runner_sha256": digest(Path(__file__)), "program_sha256": pinned, "schema_version": 1})
    trials = [(task, arm, repeat) for task in protocol["tasks"] for arm in protocol["arms"]
              for repeat in range(protocol["repetitions"])]
    random.Random(protocol["seed"]).shuffle(trials)
    write(output / "order.json", [[t["id"], a["id"], r] for t, a, r in trials])
    results = []
    for task, arm, repeat in trials:
        root = output / f'{task["id"]}--{arm["id"]}--{repeat}'
        root.mkdir()
        row = {"task": task["id"], "arm": arm["id"], "repeat": repeat,
               "execution": None, "status": "failed", "metrics": {x: None for x in METRICS},
               "targeted_passed": None, "regression_passed": None, "task_success": False,
               "strict_completion": False}
        try:
            workspace = root / "workspace"
            # Detached, local clone: no network fetch and no caller working-tree writes.
            git_env = {**os.environ, "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull, "GIT_LFS_SKIP_SMUDGE": "1"}
            clone = subprocess.run(["git", "-c", "core.hooksPath=" + os.devnull, "clone", "--no-hardlinks", "--no-checkout", "--",
                                    str(Path(task["repository"]).resolve()), str(workspace)],
                                   capture_output=True, check=False, timeout=120, env=git_env)
            (root / "clone.stderr").write_bytes(clone.stderr)
            clone.check_returncode()
            subprocess.run(["git", "-c", "core.hooksPath=" + os.devnull, "-C", str(workspace), "checkout", "--detach", task["commit"]],
                           capture_output=True, check=True, timeout=120, env=git_env)
            actual = subprocess.check_output(["git", "-C", str(workspace), "rev-parse", "HEAD"], text=True).strip()
            if actual != task["commit"]:
                raise ValueError("source commit mismatch")
            if protocol.get("source_policy") == "sealed":
                seal_workspace(workspace, task["commit"], git_env)
            request = {"schema_version": 1, "task": {key: task[key] for key in ("id", "goal", "commit")}, "arm": arm,
                       "model_identity": protocol["model_identity"], "limits": protocol["limits"],
                       "workspace": str(workspace), "trial_directory": str(root), "repeat": repeat,
                       "permissions": protocol["permissions"], "normalization": protocol["normalization"],
                       "runtime_identity": protocol.get("runtime_identity")}
            write(root / "request.json", request)
            execution = command(arm["adapter"] + [str(root / "request.json")], workspace, root,
                                protocol["limits"]["wall_seconds"] + 30)
            row["execution"] = execution
            if execution["exit_code"] == 78:
                row["status"] = "unsupported"
            elif execution["exit_code"] != 0 or execution["timed_out"]:
                # Failed trials count, and their available usage must not disappear.
                if (root / "result.json").exists():
                    response = bounded_json(root / "result.json")
                    if response.get("schema_version") != 1 or response.get("model_identity") != protocol["model_identity"]:
                        raise ValueError("failed adapter result identity mismatch")
                    row["metrics"] = metrics(response)
                    row["checks"] = response.get("checks")
                    row["usage_coverage"] = response.get("usage_coverage")
                    row["provenance"] = response.get("provenance")
            elif not execution["timed_out"] and execution["exit_code"] == 0:
                response = bounded_json(root / "result.json")
                if response.get("schema_version") != 1 or response.get("model_identity") != protocol["model_identity"]:
                    raise ValueError("adapter result identity mismatch")
                candidate = Path(response["candidate"]).resolve(strict=True)
                candidate.relative_to(root)
                if not candidate.is_dir():
                    raise ValueError("candidate must be a trial-local directory")
                row["metrics"] = metrics(response)
                row["usage_coverage"] = response.get("usage_coverage")
                row["provenance"] = response.get("provenance")
                row["checks"] = response.get("checks")
                # Separate grading copy; build artifacts cannot mutate the scored candidate.
                for path in candidate.rglob("*"):
                    if path.is_symlink():
                        path.resolve(strict=True).relative_to(candidate)
                clean_env = {key: os.environ[key] for key in ("PATH", "HOME", "LANG", "TMPDIR") if key in os.environ}
                for key in ("targeted", "regression"):
                    # A grader may install hidden tests or generate outputs.
                    # Each phase starts from the unchanged candidate so those
                    # writes cannot contaminate another phase's policy checks.
                    grading = root / "grading" / key
                    grading.parent.mkdir(exist_ok=True)
                    shutil.copytree(candidate, grading, ignore=shutil.ignore_patterns(".git", ".pactrail"), symlinks=True)
                    logs = root / key
                    logs.mkdir()
                    graded = command(task[key], grading, logs, task.get("grader_timeout_seconds", 120), clean_env)
                    write(logs / "execution.json", graded)
                    row[key + "_passed"] = graded["exit_code"] == 0 and not graded["timed_out"]
                row["status"] = "scored"
                row["task_success"] = row["targeted_passed"] and row["regression_passed"]
                row["strict_completion"] = strict_completion(row["task_success"], row["checks"])
        except (ValueError, OSError, subprocess.SubprocessError, KeyError, TypeError) as error:
            row["status"] = "invalid"
            row["error"] = f"{type(error).__name__}: {error}"
            row["task_success"] = False
        write(root / "trial.json", row)
        results.append(row)
        write(output / "results.json", results)
        hashes = {str(p.relative_to(root)): digest(p) for p in root.rglob("*")
                  if p.is_file() and not p.is_symlink() and "workspace" not in p.relative_to(root).parts
                  and "grading" not in p.relative_to(root).parts and p.name != "sha256.json"}
        write(root / "sha256.json", hashes)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("protocol", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    run(args.protocol.resolve(), args.output.resolve())


if __name__ == "__main__":
    main()
