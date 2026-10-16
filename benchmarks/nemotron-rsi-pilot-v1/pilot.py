#!/usr/bin/env python3
"""Frozen, free-only historical-task baseline through the real RSI lab."""
import argparse
from decimal import Decimal, InvalidOperation
import importlib.util
import json
import multiprocessing
import os
from pathlib import Path
import platform
import random
import shlex
import sys
import time
import urllib.error
import urllib.request
from unittest.mock import patch

REPO = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("groq_helpers", REPO / "benchmarks/groq-qwen-pilot-v1/pilot.py")
helpers = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helpers)
from pactrail_lab import budget
from pactrail_lab.archive import unpack
from pactrail_lab.execution import Execution, supervisor_inputs
from pactrail_lab.gateway import Gateway
from pactrail_lab.process import command
from pactrail_lab.safe import Refusal, canonical, digest, read
from pactrail_lab.snapshots import import_git, import_tree, materialize, revision
from pactrail_lab.store import Store

MODEL = "nvidia/nemotron-3.5-super-vl:free"
ENDPOINT = "https://openrouter.ai/api/v1/chat/completions"
TASKS = helpers.TASKS
MAX_BODY = 8_388_608
MAX_REQUESTS = 72
MAX_REPORTED_TOKENS = 2_000_000
PACING_SECONDS = 4


def credential(path):
    values = []
    for line in read(path).decode().splitlines():
        line = line.strip().removeprefix("export ")
        if line.startswith("OPENROUTER_API_KEY="):
            parts = shlex.split(line.split("=", 1)[1], comments=True)
            if len(parts) == 1: values.append(parts[0])
    if len(values) != 1 or not values[0] or any(c in values[0] for c in "\r\n"):
        raise Refusal("one literal OPENROUTER_API_KEY assignment is required")
    return values[0]


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_):
        raise Refusal("provider redirects refused")


def request(key, path, body=None, timeout=45):
    req = urllib.request.Request("https://openrouter.ai/api/v1/" + path, data=body,
        headers={"Authorization": "Bearer " + key, "Content-Type": "application/json",
                 "User-Agent": "pactrail-nemotron-rsi-pilot/1.0"})
    try:
        response = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect()).open(req, timeout=timeout)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        data = response.read(MAX_BODY + 1)
        headers = {k.lower(): v for k, v in response.headers.items()
                   if k.lower().startswith("x-ratelimit-") or k.lower() == "retry-after"}
        if len(data) > MAX_BODY or len(canonical(headers)) > 4096:
            raise Refusal("provider response exceeds bounds")
        return {"status": response.code, "headers": headers}, data.replace(key.encode(), b"[REDACTED]")


def provider_worker(key, body, sender):
    try:
        metadata, data = request(key, "chat/completions", body)
        sender.send_bytes(canonical(metadata))
        sender.send_bytes(data)
    except Exception:
        sender.send_bytes(canonical({"status": None, "headers": {}, "transport_error": True}))
    finally:
        sender.close()


def zero(value):
    try:
        number = Decimal(str(value))
        return number.is_finite() and number == 0 and not isinstance(value, bool)
    except InvalidOperation:
        return False


def check_catalog(entry):
    if entry.get("id") != MODEL:
        raise Refusal("exact free model unavailable; no substitution")
    pricing = entry.get("pricing", {})
    if not all(zero(pricing.get(k)) for k in ("prompt", "completion")) or (
            "request" in pricing and not zero(pricing["request"])):
        raise Refusal("model pricing is not zero; no paid inference authorized")
    if not {"tools", "reasoning", "temperature", "max_tokens"}.issubset(entry.get("supported_parameters", [])):
        raise Refusal("required model parameters are not advertised")


def wire_request(body):
    # Trusted routing restrictions only: original conversation/tools unchanged.
    return {**body, "provider": {"allow_fallbacks": False,
                                 "max_price": {"prompt": 0, "completion": 0}}}


class FreeGateway(Gateway):
    def submit(self, token, body):
        try:
            return super().submit(token, body)
        except Exception:
            with Store(self.root) as store, store.transaction():
                store.set("free-provider-blocked", True)
            raise

    def _provider(self, body):
        with Store(self.root) as store:
            if store.value("free-provider-blocked", False):
                raise Refusal("provider blocked; no replay, paid fallback or substitution")
            rows = budget.reservations(store).values()
            total = 0
            for row in rows:
                if row["state"] == "settled":
                    if row.get("usage") is None:
                        raise Refusal("unknown settled usage; stop further inference")
                    total += row["usage"]["input_tokens"] + row["usage"]["output_tokens"]
            if total + self.config["context_tokens"] + self.config["output_tokens"] > MAX_REPORTED_TOKENS:
                raise Refusal("campaign token admission ceiling reached")
            wait_until = store.value("next-provider-ns", 0)
        deadline = min(self.network_deadline_ns, time.time_ns() + 120 * 10**9)
        while time.time_ns() < wait_until:
            if wait_until >= deadline: raise Refusal("pacing exceeds invocation deadline")
            time.sleep(min(0.5, (wait_until - time.time_ns()) / 10**9))
        context = multiprocessing.get_context("spawn")
        receiver, sender = context.Pipe(duplex=False)
        wire = canonical(wire_request(body))
        child = context.Process(target=provider_worker, args=(self.key, wire, sender), daemon=True)
        started = time.monotonic()
        try:
            with Store(self.root) as store, store.transaction():
                store.set("next-provider-ns", time.time_ns() + PACING_SECONDS * 10**9)
                store.append("free-provider-wire-request", {"artifact": store.put(wire.replace(self.key.encode(), b"[REDACTED]"))})
            child.start()
            sender.close()
            if not receiver.poll(max(0, (deadline - time.time_ns()) / 10**9)):
                raise Refusal("provider total deadline reached")
            metadata = json.loads(receiver.recv_bytes(4096))
            with Store(self.root) as store, store.transaction():
                store.append("free-provider-response", metadata)
            if metadata.get("transport_error"):
                raise Refusal("provider transport failed; no retry")
            if not receiver.poll(max(0, (deadline - time.time_ns()) / 10**9)):
                raise Refusal("provider body deadline reached")
            data = receiver.recv_bytes(MAX_BODY)
            with Store(self.root) as store, store.transaction():
                store.append("free-provider-raw-response", {"status": metadata["status"], "artifact": store.put(data),
                             "wall_time_ms": round((time.monotonic() - started) * 1000)})
            if metadata["status"] != 200:
                raise Refusal("provider rejected request; stop all further inference")
            response = json.loads(data)
            if response.get("model") != MODEL or not isinstance(response.get("choices"), list) or not response["choices"]:
                raise Refusal("provider response model/protocol drift; stop")
            cost = (response.get("usage") or {}).get("cost")
            if cost is not None and not zero(cost):
                raise Refusal("nonzero or invalid reported inference cost; stop")
            return data
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


def account(key):
    meta, data = request(key, "key", timeout=20)
    if meta["status"] != 200: return {"http_status": meta["status"]}
    values = json.loads(data).get("data", {})
    return {"http_status": 200, **{k: values.get(k) for k in ("is_free_tier", "free_model_daily_requests")}}


def validations(output):
    hashes = {}
    for task in TASKS:
        for source in ("base", "gold"):
            root = output / "grader-validation" / task / source
            report = json.loads(read(root / "result.json"))
            targeted, regression = report["targeted"], report["regression"]
            valid = report.get("qualified") and not report.get("infrastructure_error") and not report.get("cleanup_error")
            valid = valid and targeted["coverage_complete"] and regression["coverage_complete"]
            valid = valid and (targeted["behavior_failed"] if source == "base" else
                              targeted["behavior_passed"] and regression["behavior_passed"])
            if not valid: raise Refusal("external bad/gold validation failed")
            for path in sorted(root.rglob("*")):
                if path.is_file(): hashes[str(path)] = digest(read(path))
    return hashes


def run(args):
    output, lab = args.output.resolve(), args.lab.resolve()
    if not output.is_dir() or (output / "protocol.json").exists():
        raise Refusal("fresh output with external grader validations required")
    validation = validations(output)
    key = credential(args.env_file.resolve())
    metadata, raw = request(key, "models", timeout=20)
    if metadata["status"] != 200: raise Refusal("model catalog unavailable")
    entry = next((m for m in json.loads(raw)["data"] if m["id"] == MODEL), {})
    check_catalog(entry)
    before = account(key)
    model = dict(id=MODEL, endpoint=ENDPOINT, api_key_env="OPENROUTER_API_KEY", context_tokens=131072,
                 output_tokens=4096, input_rate=0, output_rate=0, temperature=0, reasoning_effort="low")
    limits = dict(model_turns=12, wall_seconds=900, context_tokens=32768, output_tokens=4096, model_tokens=240000)
    with Store(output / "campaign", create=True) as store:
        inputs = supervisor_inputs()
        for directory in (REPO / "benchmarks/harness-lab-v1", Path(__file__).parent):
            for path in directory.glob("*.py"): inputs[str(path.resolve())] = digest(read(path))
        inputs[str(REPO / "benchmarks/groq-qwen-pilot-v1/pilot.py")] = digest(read(REPO / "benchmarks/groq-qwen-pilot-v1/pilot.py"))
        for path in (lab / "cohort-qualified-v1.json", lab / "prepared-expanded-v1/results.json", lab / "image-lock-expanded-v1.json"):
            inputs[str(path)] = digest(read(path))
        image = command(["docker", "image", "inspect", "--format", "{{.Id}}", "pactrail-lab-qualification:local"])
        image_id = image["stdout"].decode().strip()
        if image["exit_code"] or not image_id.startswith("sha256:"):
            raise Refusal("pinned local execution image unavailable")
        sha = command(["git", "rev-parse", "HEAD"], cwd=REPO)["stdout"].decode().strip()
        binary = read(args.binary.resolve(strict=True))
        with store.transaction():
            store.set("supervisor-inputs", supervisor_inputs())
            store.set("external-inputs", {})
            store.set("manifest", store.record({"image": image_id, "model": model,
                "limits": dict(requests=MAX_REQUESTS, cost_microusd=1, wall_seconds=7200)}))
            store.set("deadline-ns", time.time_ns() + 7200 * 10**9)
            harness = import_git(store, REPO, sha)
            parent = revision(store, harness, binary, {"agent_mode": "single"}, [])
        tasks = []
        for task in TASKS:
            preparation = lab / "prepared-expanded-v1" / task / "preparation.json"
            prep = json.loads(read(preparation))
            with store.transaction(): source = import_tree(store, prep["baseline"])
            goal = prep["goal"] + "\nFix the reported behavior with a focused production-code change. Do not edit tests or manifests. Commands are disabled: do not claim tests ran. External hidden tests will grade your isolated candidate."
            tasks.append({"id": task, "source": source, "goal": goal,
                          "source_tree_sha256": prep["source_tree_sha256"], "synthetic_commit": prep["synthetic_commit"]})
            for path in (preparation, preparation.with_name("grader.json")):
                inputs[str(path)] = digest(read(path))
        with store.transaction():
            for name, expected in inputs.items():
                if store.put(read(name)) != expected: raise Refusal("input changed while freezing")
        order = [(task["id"], repeat) for task in tasks for repeat in range(3)]
        random.Random(20261016).shuffle(order)
        protocol = {"schema_version": 1, "id": "nemotron-rsi-baseline-20261016-v1", "commit": sha,
            "binary_sha256": digest(binary), "image": image_id, "model": model, "provider_metadata": entry,
            "engine_limits": limits, "tasks": tasks, "order": order, "seed": 20261016, "repetitions": 3,
            "frozen_inputs": inputs, "validations": validation, "account_before": before,
            "environment": {"python": sys.version, "platform": platform.platform(), "cpu_count": os.cpu_count()},
            "provider_policy": {"exact_free_model": True, "allow_fallbacks": False, "max_price": {"prompt": 0, "completion": 0},
                                "max_requests": MAX_REQUESTS, "max_reported_tokens_plus_next_reservation": MAX_REPORTED_TOKENS,
                                "pacing_seconds": PACING_SECONDS, "unlimited_access_claimed": False},
            "permissions": {"process": "disabled", "write_paths": ["."]},
            "population": "Two public historical development issues; not held out or representative.",
            "success": "External targeted AND regression pass AND ready_to_apply, receipt_valid, trace_valid, source_isolation_valid.",
            "stopping": "No provider retries/substitution/paid fallback. Provider or infrastructure error stops all further requests. Retain all declared trials.",
            "analysis": "Per-task three-run counts, strict vs functional correctness, resource coverage, cache, latency and failure taxonomy. No causal improvement or broad superiority claim.",
            "limitations": ["Baseline only; no harness proposal/candidate comparison", "No independent held-out curation",
                "Gateway byte reservation 131072 differs from engine context target 32768", "Invoice and unlimited-access status unverified",
                "Per-run token cap is checked by engine after response; one invocation can overshoot", "No process tools inside agent; external grading only"]}
        (output / "protocol.json").write_bytes(canonical(protocol))
        (output / "protocol.sha256").write_text(digest(canonical(protocol)) + "\n")
        with store.transaction(): store.append("pilot-protocol-frozen", {"artifact": store.put(canonical(protocol))})
        print("PROTOCOL FROZEN", digest(canonical(protocol)), flush=True)
        old_key = os.environ.get("OPENROUTER_API_KEY")
        os.environ["OPENROUTER_API_KEY"] = key
        rows, stopped = [], False
        try:
            for task_id, repeat in order:
                trial = output / "trials" / f"{task_id}-r{repeat + 1}"
                trial.mkdir(parents=True)
                for path, expected in {**inputs, **validation}.items():
                    if digest(read(path)) != expected: raise Refusal("frozen input/evidence changed")
                task = next(t for t in tasks if t["id"] == task_id)
                row = {"task": task_id, "repeat": repeat, "status": "not-attempted", "strict_success": False}
                started = time.monotonic()
                if not stopped:
                    print("START", trial.name, flush=True)
                    try:
                        with patch("pactrail_lab.execution.Gateway", FreeGateway):
                            outcome = Execution(store).trial(parent, task["source"], task["goal"], limits, trial.name)
                        row.update(status=outcome["status"], checks=outcome["checks"], record=outcome["record"])
                        retained = store.load(outcome["record"])
                        unpack(store.get(retained["stdout"]), trial / "raw")
                        if outcome.get("candidate_source"):
                            materialize(store, outcome["candidate_source"], trial / "candidate")
                            targeted = helpers.external_grade(lab, task_id, "candidate", trial / "targeted", trial / "candidate", "targeted")
                            regression = helpers.external_grade(lab, task_id, "candidate", trial / "regression", trial / "candidate", "regression")
                            row.update(targeted=targeted["targeted"], regression=regression["regression"],
                                strict_success=bool(targeted["qualified"] and regression["qualified"] and len(outcome["checks"]) >= 4 and all(v is True for v in outcome["checks"].values())))
                        if store.value("free-provider-blocked", False): stopped = True
                        if not any(r["lease"] == trial.name for r in budget.reservations(store).values()) and not row["strict_success"]:
                            stopped = True
                    except (Refusal, OSError, ValueError) as error:
                        row.update(status="infrastructure-failed", error=str(error).replace(key, "[REDACTED]")[:2000])
                        stopped = True
                if row["status"] == "completed" and not all(row.get("checks", {}).values()): row["status"] = "harness-incomplete"
                row["wall_seconds"] = time.monotonic() - started
                with store.transaction(): store.append("pilot-trial-result", {"artifact": store.record(row)})
                rows.append(row)
                (output / "results.json").write_bytes(canonical(rows))
                (output / "usage.json").write_bytes(canonical(list(budget.reservations(store).values())))
                print("RESULT", trial.name, row["status"], "strict_success", row["strict_success"], flush=True)
            try: after = account(key)
            except (OSError, ValueError): after = {"http_status": None}
            with store.transaction(): store.append("pilot-account-after", after)
            (output / "account-after.json").write_bytes(canonical(after))
            (output / "events.json").write_bytes(canonical(store.events()))
            print("FINISHED", "strict_success", sum(r["strict_success"] for r in rows), "/", len(rows), flush=True)
        finally:
            if old_key is None: os.environ.pop("OPENROUTER_API_KEY", None)
            else: os.environ["OPENROUTER_API_KEY"] = old_key


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("lab", "output", "binary", "env-file"): parser.add_argument("--" + name, type=Path, required=True)
    run(parser.parse_args())
