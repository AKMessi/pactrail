#!/usr/bin/env python3
"""Admission/freeze gates around Pactrail's existing matched-trial runner.

No provider requests, repository preparation, or grader execution occur here.
Eligibility requires retained external base/gold validation, not dataset labels.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys


MAX_JSON_BYTES = 16 * 1024 * 1024
REPOSITORY = Path(__file__).resolve().parents[2]


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()


def sha256(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read_json(path):
    path = Path(path)
    with path.open("rb") as stream:
        raw = stream.read(MAX_JSON_BYTES + 1)
    if len(raw) > MAX_JSON_BYTES:
        raise ValueError("JSON exceeds admission bound")

    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate JSON key: " + key)
            result[key] = value
        return result

    value = json.loads(raw, object_pairs_hook=unique,
                       parse_constant=lambda value: (_ for _ in ()).throw(ValueError(value)))
    if not isinstance(value, dict):
        raise ValueError("JSON document must be an object")
    return value


def validate_catalogue(catalogue):
    if not isinstance(catalogue, dict) or type(catalogue.get("schema_version")) is not int or catalogue["schema_version"] != 1:
        raise ValueError("catalogue schema 1 required")
    tasks = catalogue.get("tasks")
    if not isinstance(tasks, list) or not 20 <= len(tasks) <= 50:
        raise ValueError("Harness Lab needs 20–50 distinct historical tasks")
    identities = set()
    for task in tasks:
        if not isinstance(task, dict):
            raise ValueError("task must be an object")
        identity = task.get("id")
        if not isinstance(identity, str) or not re.fullmatch(r"[a-z0-9][a-z0-9-]{0,63}", identity):
            raise ValueError("invalid task identity")
        if identity in identities:
            raise ValueError("duplicate task identity")
        identities.add(identity)
        if task.get("split") not in ("development", "confirmation"):
            raise ValueError("explicit task split required")
        if not re.fullmatch(r"[0-9a-f]{40}", task.get("base_commit", "")):
            raise ValueError("full pre-fix commit required")
        for key in ("repository", "language", "source", "grader_identity"):
            if not isinstance(task.get(key), str) or not task[key]:
                raise ValueError("missing task provenance: " + key)
        if not re.fullmatch(r"[0-9a-f]{64}", task["grader_identity"]):
            raise ValueError("grader identity must be SHA-256")
        if not re.fullmatch(r"[0-9a-f]{64}", task.get("goal_sha256", "")):
            raise ValueError("goal hash required")
    if {task["split"] for task in tasks} != {"development", "confirmation"}:
        raise ValueError("both development and confirmation sets required")
    partition = catalogue.get("partition_policy", "task_disjoint")
    if partition not in ("task_disjoint", "repository_disjoint"):
        raise ValueError("unknown partition policy")
    if partition == "repository_disjoint":
        development = {task["repository"] for task in tasks if task["split"] == "development"}
        confirmation = {task["repository"] for task in tasks if task["split"] == "confirmation"}
        if development & confirmation:
            raise ValueError("repository overlaps development and confirmation")
    return {task["id"]: task for task in tasks}


def validate_evidence(task, evidence, evidence_directory):
    """Require actual failing tests at base and passing tests after the gold fix.

    The curator is trusted; this is integrity binding, not cryptographic proof
    that arbitrary claimed logs are truthful. Review the external grader runner.
    """
    if not isinstance(evidence, dict) or type(evidence.get("schema_version")) is not int or evidence["schema_version"] != 1 or evidence.get("task") != task["id"]:
        raise ValueError("validation task/schema mismatch")
    if evidence.get("task_sha256") != hashlib.sha256(canonical(task)).hexdigest():
        raise ValueError("validation is for another task specification")
    if evidence.get("grader_identity") != task["grader_identity"]:
        raise ValueError("validation grader mismatch")
    for key in ("base_targeted", "gold_targeted", "gold_regression"):
        row = evidence.get(key, {})
        if not isinstance(row, dict):
            raise ValueError("validation record must be an object: " + key)
        count = row.get("tests_executed")
        failed = row.get("tests_failed")
        if type(count) is not int or count <= 0 or type(failed) is not int or not 0 <= failed <= count:
            raise ValueError("executed-test coverage required: " + key)
        if row.get("timed_out") is not False or row.get("infrastructure_error") is not False:
            raise ValueError("grader setup/timeout cannot qualify: " + key)
        if type(row.get("exit_code")) is not int:
            raise ValueError("integer grader exit code required")
        if key == "base_targeted":
            if failed == 0 or row["exit_code"] == 0:
                raise ValueError("base must fail an executed behavioral test")
        elif failed != 0 or row["exit_code"] != 0:
            raise ValueError("gold must pass: " + key)
        log = row.get("log")
        if not isinstance(log, str) or Path(log).is_absolute() or ".." in Path(log).parts:
            raise ValueError("log must be validation-local")
        root = Path(evidence_directory).resolve()
        path = (root / log).resolve(strict=True)
        path.relative_to(root)
        if sha256(path) != row.get("log_sha256"):
            raise ValueError("validation log digest changed")


def load_matched_runner():
    path = REPOSITORY / "benchmarks/agent-runtime-v1/run.py"
    spec = importlib.util.spec_from_file_location("pactrail_matched_runner", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def freeze(catalogue_path, protocol_path, validation_directory, output):
    """Exclusive creation; refused eligibility never emits a scored protocol."""
    catalogue = read_json(catalogue_path)
    tasks = validate_catalogue(catalogue)
    protocol = read_json(protocol_path)
    if any(not isinstance(protocol.get(key), dict) for key in ("model_identity", "permissions", "limits")):
        raise ValueError("protocol model, permissions and limits must be objects")
    for key in ("arms", "tasks"):
        if not isinstance(protocol.get(key), list) or any(not isinstance(item, dict) for item in protocol[key]):
            raise ValueError("protocol arms/tasks must contain objects")
    load_matched_runner().validate(protocol)
    split = protocol.get("lab_split")
    if split not in ("development", "confirmation"):
        raise ValueError("protocol must declare lab_split")
    expected = {identity for identity, task in tasks.items() if task["split"] == split}
    if {task["id"] for task in protocol["tasks"]} != expected:
        raise ValueError("protocol must include every declared task in its split")
    bound = {}
    identity = protocol.get("runtime_identity", {})
    binary = identity.get("binary")
    if not binary or sha256(binary) != identity.get("binary_sha256"):
        raise ValueError("explicit hash-bound runtime binary required")
    bound[str(Path(binary).resolve())] = sha256(binary)
    if not re.fullmatch(r"[0-9a-f]{40}", identity.get("commit", "")):
        raise ValueError("runtime commit required")
    for arm in protocol["arms"]:
        for index, argument in enumerate(arm["adapter"]):
            path = Path(shutil.which(argument) or argument) if index == 0 else Path(argument)
            if path.is_file():
                bound[str(path.resolve())] = sha256(path)
    for entry in protocol["tasks"]:
        task = tasks[entry["id"]]
        if entry.get("source_base_commit") != task["base_commit"]:
            raise ValueError("prepared task source identity mismatch")
        if hashlib.sha256(entry["goal"].encode()).hexdigest() != task["goal_sha256"]:
            raise ValueError("task goal changed")
        repository = Path(entry["repository"]).resolve(strict=True)
        git_env = {"PATH": os.environ["PATH"], "GIT_CONFIG_NOSYSTEM": "1",
                   "GIT_CONFIG_GLOBAL": os.devnull}
        def git(*args):
            return subprocess.check_output(["git", "-C", str(repository), *args],
                                           env=git_env, timeout=30, text=True).strip()
        if git("rev-parse", "HEAD") != entry["commit"] or git("rev-list", "--all", "--count") != "1":
            raise ValueError("prepared source must have exactly one synthetic baseline commit")
        if git("remote") or git("status", "--porcelain", "--untracked-files=all"):
            raise ValueError("prepared source must be clean and have no remote")
        path = Path(validation_directory) / (task["id"] + ".json")
        evidence = read_json(path)
        validate_evidence(task, evidence, path.parent)
        bound[str(path.resolve())] = sha256(path)
        for key in ("base_targeted", "gold_targeted", "gold_regression"):
            log = (path.parent / evidence[key]["log"]).resolve(strict=True)
            bound[str(log)] = sha256(log)
        for key in ("targeted", "regression"):
            # Require explicit grader source identities, not just an argv label.
            inputs = entry.get(key + "_inputs")
            if not isinstance(inputs, dict) or not inputs:
                raise ValueError("grader source hashes required")
            for name, expected_hash in inputs.items():
                if sha256(name) != expected_hash:
                    raise ValueError("grader source changed: " + name)
                bound[str(Path(name).resolve())] = expected_hash
    for path in (catalogue_path, protocol_path, Path(__file__),
                 REPOSITORY / "benchmarks/agent-runtime-v1/run.py"):
        bound[str(Path(path).resolve())] = sha256(path)
    protocol["frozen_inputs"] = {**protocol.get("frozen_inputs", {}), **bound}
    payload = json.dumps(protocol, indent=2, allow_nan=False) + "\n"
    output = Path(output)
    # The caller creates the evidence directory; avoid mutation before all gates.
    with output.open("x", encoding="utf-8") as stream:
        stream.write(payload)
    return sha256(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    check = sub.add_parser("check-catalogue")
    check.add_argument("catalogue", type=Path)
    frozen = sub.add_parser("freeze")
    frozen.add_argument("catalogue", type=Path)
    frozen.add_argument("protocol", type=Path)
    frozen.add_argument("validation", type=Path)
    frozen.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "check-catalogue":
            print(json.dumps({"tasks": len(validate_catalogue(read_json(args.catalogue))),
                              "eligible_for_scoring": False}))
        else:
            print(freeze(args.catalogue, args.protocol, args.validation, args.output))
    except (ValueError, OSError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
