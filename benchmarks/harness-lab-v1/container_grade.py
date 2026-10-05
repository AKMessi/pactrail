#!/usr/bin/env python3
"""Validate one prepared base/gold tree using an already-pulled immutable image.

Trusted operator-side grading, never an agent tool. No host mounts, networking,
provider environment, implicit pulls or gold exposure to the agent source tree.
Each invocation creates fresh exclusive evidence and removes its container.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
import uuid

from grading import summarize
from lab import canonical, read_json, sha256, validate_catalogue
from prepare import tree_identity
from candidate_patch import build as build_candidate_patch, identity as candidate_identity

MAX_LOG_BYTES = 8 * 1024 * 1024


def bounded_list(path):
    with Path(path).open("rb") as stream:
        raw = stream.read(16 * 1024 * 1024 + 1)
    if len(raw) > 16 * 1024 * 1024:
        raise ValueError("ledger exceeds bound")
    rows = json.loads(raw)
    if not isinstance(rows, list) or not 1 <= len(rows) <= 50:
        raise ValueError("bounded case ledger required")
    if any(not isinstance(row, dict) for row in rows):
        raise ValueError("case ledger records must be objects")
    return rows


def control(argv, env, *, timeout=60):
    # Bound untrusted responses before reading into memory, not afterwards.
    with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as error:
        process = subprocess.Popen(argv, env=env, stdout=output, stderr=error)
        deadline = time.monotonic() + timeout
        try:
            while process.poll() is None:
                if os.fstat(output.fileno()).st_size + os.fstat(error.fileno()).st_size > MAX_LOG_BYTES:
                    raise ValueError("control response exceeds bound")
                if time.monotonic() >= deadline:
                    raise subprocess.TimeoutExpired(argv, timeout)
                time.sleep(0.05)
            if os.fstat(output.fileno()).st_size + os.fstat(error.fileno()).st_size > MAX_LOG_BYTES:
                raise ValueError("control response exceeds bound")
            error.seek(0)
            if process.returncode:
                detail = error.read(2000).decode("utf-8", errors="replace")
                raise RuntimeError(f"command failed with exit {process.returncode}: {argv[0]}: {detail}")
            output.seek(0)
            return output.read(MAX_LOG_BYTES)
        finally:
            if process.poll() is None:
                process.kill()
            process.wait()


def execute(args):
    tasks = validate_catalogue(read_json(args.catalogue))
    task = tasks[args.task]
    prepared = [row for row in bounded_list(args.prepared) if row.get("id") == args.task]
    images = [row for row in bounded_list(args.images) if row.get("task") == args.task]
    if len(prepared) != 1 or len(images) != 1:
        raise ValueError("one preparation and one image identity required")
    prepared, image = prepared[0], images[0]
    if prepared["status"] != "prepared_not_validated" or image["status"] != "manifest_locked_not_executed":
        raise ValueError("case is not prepared with a locked image")
    import hashlib
    if prepared["task_sha256"] != hashlib.sha256(canonical(task)).hexdigest():
        raise ValueError("preparation is for another task")
    immutable = image["immutable_image"]
    if not re.fullmatch(r"swebench/[a-z0-9_.-]+@sha256:[0-9a-f]{64}", immutable):
        raise ValueError("immutable declared public image required")
    if immutable.rsplit("@", 1)[1] != image["image_digest"] or image["image_tag"] != prepared["image_tag"]:
        raise ValueError("image identity mismatch")
    baseline = Path(prepared["baseline"]).resolve(strict=True)
    source = (Path(args.candidate) if args.source == "candidate" else
              Path(prepared["baseline" if args.source == "base" else "gold"])).resolve(strict=True)
    expected = (candidate_identity(source) if args.source == "candidate" else
                prepared["source_tree_sha256" if args.source == "base" else "gold_tree_sha256"])
    actual = candidate_identity(source) if args.source == "candidate" else tree_identity(source)
    if actual != expected:
        raise ValueError("prepared source tree changed")
    grader_path = baseline.parent / "grader.json"
    grader = read_json(grader_path)
    if hashlib.sha256(canonical(grader)).hexdigest() != task["grader_identity"]:
        raise ValueError("grader identity changed")
    env = {key: os.environ[key] for key in ("PATH", "HOME", "LANG") if key in os.environ}
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
    docker = ["docker", "--host", "unix:///var/run/docker.sock"]
    inspected = json.loads(control(docker + ["image", "inspect", immutable], env))[0]
    if inspected.get("Os") != "linux" or inspected.get("Architecture") != "amd64" or immutable not in inspected.get("RepoDigests", []):
        raise ValueError("installed image does not match its immutable platform identity")
    # Validation uses the identity-bound reference patch, including added files
    # that a plain git diff of an unindexed gold tree would silently omit.
    reference = baseline.parent / "reference.patch"
    if sha256(reference) != task["reference_patch_sha256"]:
        raise ValueError("reference fix changed")
    patch = (build_candidate_patch(baseline, source, control) if args.source == "candidate" else
             reference.read_bytes() if args.source == "gold" else b"")
    args.output.mkdir(parents=True, exist_ok=False)
    (args.output / "candidate.patch").write_bytes(patch)
    (args.output / "eval.sh").write_text(grader["eval_script"], encoding="utf-8")
    container = None
    report = {"task": args.task, "source": args.source, "scored": False,
              "source_tree_sha256": expected, "image": immutable,
              "image_id": inspected["Id"], "grader_identity": task["grader_identity"],
              "patch_sha256": sha256(args.output / "candidate.patch")}
    started = time.monotonic()
    try:
        # Name before admission so cleanup can address an uncertain create
        # response without relying on a returned container ID.
        container = "pactrail-grade-" + uuid.uuid4().hex
        report["container_name"] = container
        control(docker + ["create", "--name", container, "--label", "pactrail.harness-lab.task=" + args.task,
                            "--pull", "never", "--network", "none",
                            "--memory", "6g", "--cpus", "2", "--pids-limit", "512",
                            "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
                            "--entrypoint", "/bin/bash", immutable, "-c", "sleep 3600"], env)
        control(docker + ["start", container], env)
        head = control(docker + ["exec", container, "git", "-C", "/testbed", "rev-parse", "HEAD"], env).decode().strip()
        report["image_checkout_commit"] = head
        # Upstream images may preserve an environment-setup commit or dependency
        # installation edits. Restore actual task source without deleting the
        # already installed untracked dependencies or fetching any history.
        control(docker + ["exec", container, "git", "-C", "/testbed", "cat-file", "-e",
                         task["base_commit"] + "^{commit}"], env)
        control(docker + ["exec", container, "git", "-C", "/testbed", "reset", "--hard", task["base_commit"]], env)
        head = control(docker + ["exec", container, "git", "-C", "/testbed", "rev-parse", "HEAD"], env).decode().strip()
        if head != task["base_commit"]:
            raise ValueError("grading image source commit mismatch")
        try:
            control(docker + ["exec", container, "git", "-C", "/testbed", "diff", "--quiet", "HEAD"], env)
        except RuntimeError as error:
            raise ValueError("grading source remains modified after reset") from error
        report["grading_source_commit"] = head
        for filename in ("candidate.patch", "eval.sh"):
            control(docker + ["cp", str(args.output / filename), container + ":/tmp/" + filename], env)
        if patch:
            control(docker + ["exec", "-w", "/testbed", container, "git", "apply", "--check", "/tmp/candidate.patch"], env)
            control(docker + ["exec", "-w", "/testbed", container, "git", "apply", "/tmp/candidate.patch"], env)
        log = args.output / "test.log"
        timed_out, oversized = False, False
        with log.open("wb") as output:
            process = subprocess.Popen(docker + ["exec", container, "bash", "/tmp/eval.sh"],
                                       env=env, stdout=output, stderr=subprocess.STDOUT)
            deadline = time.monotonic() + args.timeout
            while process.poll() is None:
                timed_out = time.monotonic() >= deadline
                oversized = log.stat().st_size > MAX_LOG_BYTES
                if timed_out or oversized:
                    process.kill()
                    break
                time.sleep(0.05)
            shell_exit = process.wait()
        report.update(shell_exit_code=shell_exit, timed_out=timed_out, output_limit=oversized,
                      log_sha256=sha256(log))
        if timed_out or oversized:
            raise ValueError("grader exceeded time or output bound")
        parsed = control([str(args.parser_python), str(Path(__file__).with_name("parse_upstream.py")),
                          str(grader_path), str(log), task["upstream_instance_id"]], env)
        (args.output / "parsed.json").write_bytes(parsed)
        statuses = json.loads(parsed)["statuses"]
        targeted = summarize(grader["FAIL_TO_PASS"], statuses)
        regression = summarize(grader["PASS_TO_PASS"], statuses)
        report.update(targeted=targeted, regression=regression,
                      qualified=(targeted["behavior_failed"] if args.source == "base" else
                                 targeted["behavior_passed"] if args.source == "candidate" and args.phase == "targeted" else
                                 regression["behavior_passed"] if args.source == "candidate" and args.phase == "regression" else
                                 targeted["behavior_passed"] and regression["behavior_passed"]))
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError, KeyError) as error:
        report.update(qualified=False, infrastructure_error=str(error))
    finally:
        if container:
            try:
                control(docker + ["rm", "--force", container], env)
            except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
                report.update(qualified=False, cleanup_error=str(error), retained_container=container)
        report["wall_seconds"] = time.monotonic() - started
        (args.output / "result.json").write_text(json.dumps(report, indent=2, allow_nan=False) + "\n")
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for field in ("catalogue", "prepared", "images", "parser-python", "output"):
        parser.add_argument("--" + field, type=Path, required=True)
    parser.add_argument("--task", required=True)
    parser.add_argument("--source", choices=("base", "gold", "candidate"), required=True)
    parser.add_argument("--candidate", type=Path)
    parser.add_argument("--phase", choices=("targeted", "regression"))
    parser.add_argument("--timeout", type=int, default=600)
    args = parser.parse_args()
    if not 1 <= args.timeout <= 3600:
        parser.error("timeout must be 1–3600 seconds")
    if (args.source == "candidate") != (args.candidate is not None and args.phase is not None):
        parser.error("candidate grading requires --candidate and --phase")
    result = execute(args)
    print(json.dumps(result, allow_nan=False))
    raise SystemExit(0 if result.get("qualified") else 1)
