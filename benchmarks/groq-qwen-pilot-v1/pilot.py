#!/usr/bin/env python3
"""Free-plan diagnostic using the existing isolated runtime and external graders.

This is a baseline qualification experiment, not an improvement comparison.
The operator must confirm Free-plan membership; rate headers cannot prove billing.
"""
import argparse
import hashlib
import json
import multiprocessing
import os
from pathlib import Path
import random
import shlex
import sys
import time
import urllib.error
import urllib.request
from unittest.mock import patch

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "experiments/self-improvement-v1"))
from pactrail_lab import budget
from pactrail_lab.archive import unpack
from pactrail_lab.execution import Execution, supervisor_inputs
from pactrail_lab.gateway import Gateway
from pactrail_lab.process import command
from pactrail_lab.safe import Refusal, canonical, digest, read
from pactrail_lab.snapshots import import_git, import_tree, materialize, revision
from pactrail_lab.store import Store

MODEL = "qwen/qwen3.8-27b"
ENDPOINT = "https://api.groq.com/openai/v1/chat/completions"
TASKS = ("psf-requests-6028", "vuejs-core-11915")
MAX_BODY = 8_388_608


def credential(path):
    values = []
    for line in read(path).decode().splitlines():
        line = line.strip().removeprefix("export ")
        if line.startswith("GROQ_API_KEY="):
            words = shlex.split(line.split("=", 1)[1], comments=True)
            if len(words) == 1:
                values.append(words[0])
    if len(values) != 1 or not values[0] or any(c in values[0] for c in "\r\n"):
        raise Refusal("one valid GROQ_API_KEY assignment required; no shell evaluation")
    return values[0]


def provider_worker(key, body, sender):
    """Only allowlisted public response headers and bounded bytes cross back."""
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, *_):
            raise Refusal("redirect refused")
    try:
        request = urllib.request.Request(ENDPOINT, data=body, headers={
            "Authorization": "Bearer " + key, "Content-Type": "application/json",
            "User-Agent": "pactrail-free-pilot/1.0"})
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect)
        try:
            response = opener.open(request, timeout=45)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            data = response.read(MAX_BODY + 1)
            headers = {k.lower(): v for k, v in response.headers.items()
                       if k.lower().startswith("x-ratelimit-") or k.lower() == "retry-after"}
            if len(data) > MAX_BODY or len(canonical(headers)) > 4096:
                raise Refusal("provider response exceeds bound")
            metadata = canonical({"status": response.code, "headers": headers})
            sender.send_bytes(metadata)
            sender.send_bytes(data.replace(key.encode(), b"[REDACTED]"))
    except Exception:
        sender.send_bytes(canonical({"status": None, "headers": {}, "transport_error": True}))
    finally:
        sender.close()


class FreeGateway(Gateway):
    def _provider(self, body):
        # Pactrail's compatible adapter emits OpenRouter's reasoning object.
        # Groq requires the equivalent top-level field; freeze and retain both.
        wire = dict(body)
        if "reasoning" in wire:
            wire["reasoning_effort"] = wire.pop("reasoning")["effort"]
        with Store(self.root) as store:
            if store.value("free-provider-blocked", False):
                raise Refusal("provider previously failed; no automatic retry/substitution")
            rows = budget.reservations(store)
            consumed = 61  # Separate, unscored connectivity probe retained in protocol.
            for row in rows.values():
                if row["state"] == "settled":
                    usage = row.get("usage")
                    if usage is None:
                        raise Refusal("unknown usage; no more free-quota requests admitted")
                    consumed += usage["input_tokens"] + usage["output_tokens"]
            if consumed + self.config["context_tokens"] + self.config["output_tokens"] > 180_000:
                raise Refusal("conservative daily pilot token allowance exhausted")
            wait_until = store.value("next-provider-ns", 0)
        deadline = min(self.network_deadline_ns, time.time_ns() + 120 * 10**9)
        while time.time_ns() < wait_until:
            if wait_until >= deadline:
                raise Refusal("rate pacing would exceed invocation deadline")
            time.sleep(min(1, (wait_until - time.time_ns()) / 10**9))
        context = multiprocessing.get_context("spawn")
        receiver, sender = context.Pipe(duplex=False)
        child = context.Process(target=provider_worker, args=(self.key, canonical(wire), sender), daemon=True)
        try:
            with Store(self.root) as store, store.transaction():
                store.set("next-provider-ns", time.time_ns() + 65 * 10**9)
                store.append("free-provider-wire-request", {"artifact": store.put(canonical(wire).replace(self.key.encode(), b"[REDACTED]"))})
            child.start()
            sender.close()
            if not receiver.poll(max(0, (deadline - time.time_ns()) / 10**9)):
                raise Refusal("provider total deadline reached")
            metadata = json.loads(receiver.recv_bytes(4096))
            with Store(self.root) as store, store.transaction():
                store.append("free-provider-response", metadata)
                headers = metadata["headers"]
                if (metadata["status"] != 200 or headers.get("x-ratelimit-limit-requests") != "1000"
                        or headers.get("x-ratelimit-limit-tokens") != "8000"):
                    store.set("free-provider-blocked", True)
            if (metadata["status"] != 200 or headers.get("x-ratelimit-limit-requests") != "1000"
                    or headers.get("x-ratelimit-limit-tokens") != "8000"):
                raise Refusal("provider failed or expected free-tier limits changed")
            if not receiver.poll(max(0, (deadline - time.time_ns()) / 10**9)):
                raise Refusal("provider body deadline reached")
            return receiver.recv_bytes(MAX_BODY)
        except Exception:
            with Store(self.root) as store, store.transaction():
                store.set("free-provider-blocked", True)
            raise
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


def external_grade(lab, task, source, output, candidate=None, phase=None):
    argv = [sys.executable, str(REPO / "benchmarks/harness-lab-v1/container_grade.py"),
            "--catalogue", str(lab / "cohort-qualified-v1.json"),
            "--prepared", str(lab / "prepared-expanded-v1/results.json"),
            "--images", str(lab / "image-lock-expanded-v1.json"),
            "--parser-python", "/home/akmessi/.cache/pactrail-harness-lab-swebench/bin/python",
            "--task", task, "--source", source, "--timeout", "300", "--output", str(output)]
    if candidate is not None:
        argv += ["--candidate", str(candidate), "--phase", phase]
    result = command(argv, timeout=360, limit=1_048_576)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.with_suffix(".stdout").write_bytes(result["stdout"])
    output.with_suffix(".stderr").write_bytes(result["stderr"])
    report = json.loads(read(output / "result.json"))
    if result["reason"] or report.get("infrastructure_error") or report.get("cleanup_error"):
        raise Refusal("external grader infrastructure failure; evidence retained")
    return report


def run(args):
    if not args.free_plan_confirmed:
        raise Refusal("operator confirmation of this key's Free-plan organization is required")
    output, lab = args.output.resolve(), args.lab.resolve()
    if not output.is_dir() or (output / "protocol.json").exists():
        raise Refusal("fresh validated output directory required; never overwrite a scored protocol")
    validations = {}
    for task in TASKS:
        for source in ("base", "gold"):
            report_path = output / "grader-validation" / task / source / "result.json"
            report = json.loads(read(report_path))
            if not report.get("qualified") or report.get("infrastructure_error") or report.get("cleanup_error"):
                raise Refusal("bad/gold grader not validated")
            targeted, regression = report["targeted"], report["regression"]
            if not targeted["coverage_complete"] or not regression["coverage_complete"]:
                raise Refusal("incomplete grader coverage")
            if source == "base" and not targeted["behavior_failed"]:
                raise Refusal("base did not fail executed targeted tests")
            if source == "gold" and not (targeted["behavior_passed"] and regression["behavior_passed"]):
                raise Refusal("gold did not pass targeted and regression tests")
            for path in sorted(report_path.parent.rglob("*")):
                if path.is_file(): validations[str(path)] = digest(read(path))
    key = credential(args.env_file.resolve())
    request = urllib.request.Request("https://api.groq.com/openai/v1/models",
        headers={"Authorization": "Bearer " + key, "User-Agent": "pactrail-free-pilot/1.0"})
    with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request, timeout=20) as response:
        metadata = next(m for m in json.loads(response.read(1_048_576))["data"] if m["id"] == MODEL)
    if metadata.get("active") is not True:
        raise Refusal("exact model is inactive; no fallback")
    model = dict(id=MODEL, endpoint=ENDPOINT, api_key_env="GROQ_API_KEY", context_tokens=65536,
                 output_tokens=2048, input_rate=0, output_rate=0, temperature=0, reasoning_effort="low")
    limits = dict(model_turns=6, wall_seconds=900, context_tokens=16384,
                  output_tokens=2048, model_tokens=80000)
    with Store(output / "campaign", create=True) as store:
        inputs = supervisor_inputs()
        inputs[str(Path(__file__).resolve())] = digest(read(__file__))
        for path in (REPO / "benchmarks/harness-lab-v1").glob("*.py"):
            inputs[str(path)] = digest(read(path))
        for path in (lab / "cohort-qualified-v1.json", lab / "prepared-expanded-v1/results.json",
                     lab / "image-lock-expanded-v1.json"):
            inputs[str(path)] = digest(read(path))
        code_inputs = supervisor_inputs()
        with store.transaction():
            for name, expected in inputs.items():
                if store.put(read(name)) != expected: raise Refusal("input changed")
        image = command(["docker", "image", "inspect", "--format", "{{.Id}}", "pactrail-lab-qualification:local"])
        image_id = image["stdout"].decode().strip()
        if image["exit_code"] or not image_id.startswith("sha256:"):
            raise Refusal("preinstalled pinned OCI image unavailable")
        with store.transaction():
            store.set("supervisor-inputs", code_inputs)
            store.set("external-inputs", {})
            store.set("manifest", store.record({"image": image_id, "model": model,
                "limits": dict(requests=24, cost_microusd=1, wall_seconds=3600)}))
            store.set("deadline-ns", time.time_ns() + 3600 * 10**9)
        sha = command(["git", "-C", str(REPO), "rev-parse", "HEAD"])["stdout"].decode().strip()
        binary = read(args.binary.resolve(strict=True))
        with store.transaction():
            harness = import_git(store, REPO, sha)
            parent = revision(store, harness, binary, {"agent_mode": "single"}, [])
        tasks = []
        for task in TASKS:
            prep = json.loads(read(lab / "prepared-expanded-v1" / task / "preparation.json"))
            with store.transaction():
                source = import_tree(store, prep["baseline"])
            goal = prep["goal"] + "\nFix the reported behavior with a focused production-code change. Do not edit tests or manifests. Commands are disabled: do not claim tests ran. External hidden tests will grade your isolated candidate."
            tasks.append({"id": task, "source": source, "goal": goal,
                          "source_tree_sha256": prep["source_tree_sha256"], "synthetic_commit": prep["synthetic_commit"]})
            inputs[str(lab / "prepared-expanded-v1" / task / "grader.json")] = digest(read(lab / "prepared-expanded-v1" / task / "grader.json"))
        order = [(task["id"], repeat) for task in tasks for repeat in range(2)]
        random.Random(20261012).shuffle(order)
        protocol = {"schema_version": 1, "commit": sha, "binary_sha256": digest(binary),
            "image": image_id, "model": model, "provider_metadata": metadata, "engine_limits": limits, "tasks": tasks, "order": order,
            "seed": 20261012, "repetitions": 2, "frozen_inputs": inputs, "validations": validations,
            "free_plan": {"operator_confirmed": True, "expected_rpd": 1000, "expected_tpm": 8000,
                          "max_requests": 24, "max_reported_tokens_plus_next_reservation": 180000,
                          "pacing_seconds": 65, "probe_tokens": 61},
            "permissions": {"process": "disabled", "write_paths": ["."]},
            "population": "Two previously developed public historical defects; diagnostic only, not held out.",
            "success": "Targeted and regression tests pass AND ready-to-apply/receipt/trace/source checks pass.",
            "stopping": "No retries, substitution or paid fallback. Any provider/infrastructure failure stops remaining calls; all four declared trials retained.",
            "analysis": "Per-task pass counts, physical usage and observable failure categories; no superiority/significance claim.",
            "limitations": ["No candidate harness improvement or causal RSI comparison", "Small quota-bound sample",
                "Free status is operator-attested, not cryptographically verified", "Cache/invoice metrics absent unless reported",
                "Strict failure may leave an ungraded partial candidate", "Gateway byte reservation is 65536; engine context target is 16384"]}
        (output / "protocol.json").write_bytes(canonical(protocol))
        (output / "protocol.sha256").write_text(digest(canonical(protocol)) + "\n")
        os.environ["GROQ_API_KEY"] = key
        rows, stopped = [], False
        try:
            for task_id, repeat in order:
                trial = output / "trials" / f"{task_id}-r{repeat + 1}"
                trial.mkdir(parents=True)
                task = next(t for t in tasks if t["id"] == task_id)
                for path, expected in inputs.items():
                    if digest(read(path)) != expected: raise Refusal("frozen input changed")
                for path, expected in validations.items():
                    if digest(read(path)) != expected: raise Refusal("validation evidence changed")
                row = {"task": task_id, "repeat": repeat, "status": "not-attempted", "strict_success": False}
                started = time.monotonic()
                if not stopped:
                    try:
                        with patch("pactrail_lab.execution.Gateway", FreeGateway):
                            outcome = Execution(store).trial(parent, task["source"], task["goal"], limits, trial.name)
                        row.update(status=outcome["status"], checks=outcome["checks"], record=outcome["record"])
                        retained = store.load(outcome["record"])
                        if outcome.get("candidate_source"):
                            materialize(store, outcome["candidate_source"], trial / "candidate")
                            unpack(store.get(retained["stdout"]), trial / "raw")
                            targeted = external_grade(lab, task_id, "candidate", trial / "targeted", trial / "candidate", "targeted")
                            regression = external_grade(lab, task_id, "candidate", trial / "regression", trial / "candidate", "regression")
                            row.update(targeted=targeted["targeted"], regression=regression["regression"],
                                strict_success=targeted["qualified"] and regression["qualified"] and
                                len(outcome["checks"]) >= 4 and all(v is True for v in outcome["checks"].values()))
                        else:
                            unpack(store.get(retained["stdout"]), trial / "raw")
                        if store.value("free-provider-blocked", False) or (
                                not any(r["lease"] == trial.name for r in budget.reservations(store).values())
                                and not row["strict_success"]): stopped = True
                    except (Refusal, OSError, ValueError) as error:
                        row.update(status="infrastructure-failed", error=str(error).replace(key, "[REDACTED]")[:2000])
                        stopped = True
                row["wall_seconds"] = time.monotonic() - started
                if row["status"] == "completed" and not all(row.get("checks", {}).values()):
                    row["status"] = "harness-incomplete"
                rows.append(row)
                (output / "results.json").write_bytes(canonical(rows))
                print(task_id, repeat + 1, row["status"], "strict_success", row["strict_success"], flush=True)
            requests = budget.reservations(store)
            (output / "usage.json").write_bytes(canonical(list(requests.values())))
            (output / "events.json").write_bytes(canonical(store.events()))
            (output / "summary.json").write_bytes(canonical({"declared": len(rows), "strict_success": sum(r["strict_success"] for r in rows),
                "physical_requests": len(requests), "usage_coverage": sum(r.get("usage") is not None for r in requests.values()),
                "input_tokens": sum(r["usage"]["input_tokens"] for r in requests.values() if r.get("usage")),
                "output_tokens": sum(r["usage"]["output_tokens"] for r in requests.values() if r.get("usage")),
                "free_plan_operator_attested": True, "invoice_verified": False,
                "self_improvement_evidence": False, "stopped": stopped}))
        finally:
            os.environ.pop("GROQ_API_KEY", None)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lab", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--env-file", required=True, type=Path)
    parser.add_argument("--free-plan-confirmed", action="store_true")
    run(parser.parse_args())
