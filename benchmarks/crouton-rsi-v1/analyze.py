#!/usr/bin/env python3
"""Extend the shared journal audit with descriptive failure/mechanism metrics."""
import argparse
from collections import Counter
import importlib.util
import json
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("shared_analysis", REPO / "benchmarks/nemotron-rsi-pilot-v1/analyze.py")
shared = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shared)
from pactrail_lab.safe import digest, read
from pactrail_lab.store import Store


def coverage(values, total):
    reported = [v for v in values if v is not None]
    return {"reported_sum": sum(reported) if reported else None,
            "reported": len(reported), "total": total,
            "coverage": "complete" if total and len(reported) == total else "partial" if reported else "absent"}


def observable_failure(stderr, tool_errors, succeeded):
    if succeeded: return "success"
    lowered = stderr.lower()
    if "model reached its" in lowered and "output limit" in lowered: return "output-exhaustion"
    if "exceeding" in lowered and "token" in lowered: return "total-token-exhaustion"
    if "did not complete within" in lowered: return "turn-exhaustion"
    if "every tool request failed" in lowered: return "tool-interface-stall"
    if "gateway refused" in lowered or "provider" in lowered: return "provider-or-admission"
    if any("not authorized" in e["summary"].lower() for e in tool_errors): return "policy-or-verification"
    return "incorrect-or-incomplete-solution"


def analyze(root):
    report = shared.analyze(root)
    protocol = json.loads(read(root / "protocol.json"))
    original_results = json.loads(read(root / "results.json"))
    events_export = json.loads(read(root / "events.json", 8_388_608))
    costs, reasoning, cache_write, finishes, providers = [], [], [], Counter(), Counter()
    dispatches = 0
    with Store(root / "campaign") as store:
        events = store.events()
        if events_export != events: raise ValueError("event export differs from authoritative journal")
        for name, expected in {**protocol["frozen_inputs"], **protocol["validations"]}.items():
            if digest(read(name)) != expected: raise ValueError("frozen input or grader evidence changed")
        after = [e["payload"] for e in events if e["kind"] == "pilot-account-after"]
        if after != [report["account_after"]]: raise ValueError("account export differs from journal")
        for event in events:
            if event["kind"] == "free-provider-wire-request": dispatches += 1
            if event["kind"] != "free-provider-raw-response": continue
            payload = event["payload"]
            response = json.loads(store.get(payload["artifact"]))
            usage = response.get("usage") or {}
            costs.append(usage.get("cost"))
            reasoning.append((usage.get("completion_tokens_details") or {}).get("reasoning_tokens"))
            cache_write.append((usage.get("prompt_tokens_details") or {}).get("cache_write_tokens"))
            finishes.update(c.get("finish_reason") for c in response.get("choices", []))
            providers[response.get("provider")] += 1
    failures, all_tools, all_errors = Counter(), Counter(), []
    total_bytes, bytes_coverage = 0, 0
    for trial, original in zip(report["trials"], original_results):
        name = f'{trial["task"]}-r{trial["repeat"] + 1}'
        raw = root / "trials" / name / "raw"
        stderr = (raw / "run.stderr").read_text(errors="replace") if (raw / "run.stderr").exists() else ""
        trace_path = raw / "parent-trace.stdout"
        actions, errors = [], []
        if trace_path.exists():
            trace = json.loads(read(trace_path, 8_388_608))
            envelopes = trace if isinstance(trace, list) else trace.get("events", [])
            if len({r["sequence"] for r in envelopes}) != len(envelopes):
                raise ValueError("duplicate trace sequence in metrics input")
            actions = [r["event"]["data"] for r in envelopes if r.get("event", {}).get("type") == "action_completed"]
            for action in actions:
                if not action.get("actor", "").startswith("tool:"): continue
                all_tools[action["action"]] += 1
                value = (action.get("attributes") or {}).get("output_bytes")
                if value is not None:
                    total_bytes += int(value)
                    bytes_coverage += 1
                if action.get("succeeded") is False:
                    error = {"tool": action["action"], "summary": action.get("summary", "")[:2000]}
                    errors.append(error)
                    all_errors.append(error)
        failure = observable_failure(stderr, errors, trial["strict_success"])
        if original["status"] == "not-attempted": failure = "not-attempted-after-stop"
        elif original["status"] == "infrastructure-failed": failure = "benchmark-infrastructure"
        failures[failure] += 1
        trial["observable_failure"] = failure
        trial["failed_tool_observations"] = errors
        trial["runtime_error_excerpt"] = stderr[-4000:]
        trial["context_compactions"] = sum(a.get("action") == "compact_model_context" for a in actions)
        # Test/manifests remain frozen externally. Flag prompt-scope violations
        # separately; do not rewrite the preregistered primary outcome.
        paths = trial.get("changed_paths")
        trial["scope_review_needed"] = [p for p in paths if
            any(part in {"test", "tests", "__tests__"} for part in Path(p).parts) or
            Path(p).name in {"Cargo.toml", "Cargo.lock", "package.json", "package-lock.json", "pnpm-lock.yaml", "pyproject.toml"}] if paths is not None else None
    report["mechanisms"] = {"observable_failure_taxonomy": dict(failures), "tool_counts": dict(all_tools),
        "tool_error_counts": dict(Counter(e["tool"] for e in all_errors)),
        "tool_output_bytes": {"reported_sum": total_bytes if bytes_coverage else None,
                              "reported_actions": bytes_coverage, "total_actions": sum(all_tools.values())},
        "finish_reasons": dict(finishes), "providers": dict(providers),
        "reasoning_tokens": coverage(reasoning, dispatches), "cache_write_tokens": coverage(cache_write, dispatches),
        "reported_cost": coverage(costs, dispatches)}
    report["analysis_code_sha256"] = digest(read(__file__))
    (root / "analysis.json").write_text(json.dumps(report, sort_keys=True, indent=2) + "\n")
    shared.inventory(root)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    result = analyze(parser.parse_args().output.resolve())
    print(json.dumps({k: v for k, v in result.items() if k != "trials"}, sort_keys=True, indent=2))
