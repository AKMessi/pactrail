#!/usr/bin/env python3
"""Audit completed Nemotron baseline evidence; no model or grader calls."""
import argparse
from collections import Counter
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "experiments/self-improvement-v1"))
from pactrail_lab import budget
from pactrail_lab.safe import canonical, digest
from pactrail_lab.store import Store

spec = importlib.util.spec_from_file_location("usage_analysis", REPO / "benchmarks/groq-qwen-pilot-v1/analyze.py")
usage_analysis = importlib.util.module_from_spec(spec)
spec.loader.exec_module(usage_analysis)


def functional(row):
    phases = [row.get("targeted"), row.get("regression")]
    if any(not isinstance(p, dict) for p in phases): return None
    return all(p.get("coverage_complete") is True and p.get("behavior_passed") is True for p in phases)


def category(row, provider_failed, errors):
    if row["status"] == "not-attempted": return "not-attempted-after-stop"
    if provider_failed: return "provider-or-transport"
    if row["status"] == "infrastructure-failed": return "benchmark-infrastructure"
    if row["strict_success"]: return "success"
    if functional(row) is True: return "functional-pass-incomplete-workflow"
    if any(word in errors.lower() for word in ("exhausted", "turn limit", "token limit", "budget")):
        return "budget-or-output-exhaustion"
    if "every tool request failed" in errors.lower(): return "tool-interface-stall"
    if functional(row) is False: return "targeted-or-regression-failure"
    return "incomplete-ungraded-candidate"


def number(value):
    try:
        result = int(value)
        return result if result >= 0 else None
    except (ValueError, TypeError): return None


def trace_metrics(root):
    path = root / "raw/parent-trace.stdout"
    if not path.exists(): return None
    try: trace = json.loads(path.read_bytes())
    except ValueError: return None
    rows = trace if isinstance(trace, list) else trace.get("events", [])
    # Real model actions only; recovery reservations may exceed this count.
    actions = [r.get("event", {}).get("data", {}) for r in rows
               if r.get("event", {}).get("type") == "action_completed"]
    models = [a for a in actions if a.get("actor", "").startswith("model:") and a.get("action") == "invoke"]
    tools = [a for a in actions if a.get("actor", "").startswith("tool:")]
    counts = Counter(a["actor"].removeprefix("tool:") for a in tools)
    reads = {"read_file", "read_many_files", "read_observation", "search", "list_files", "search_code_graph", "search_change_impact"}
    writes = {"edit_file", "write_file", "replace_text", "remove_file", "apply_patch"}
    return {"events": len(rows), "model_turns": len(models), "tool_calls": len(tools),
            "tool_failures": sum(a.get("succeeded") is False for a in tools), "tool_counts": dict(sorted(counts.items())),
            "unavailable_tool_rejections": sum(a.get("action") == "reject_unavailable_tool" for a in actions),
            "read_calls": sum(counts[k] for k in reads), "write_calls": sum(counts[k] for k in writes),
            "process_calls": counts["run_process"],
            "model_action_duration_ms": sum(number(a.get("duration_ms")) or 0 for a in models)}


def inventory(root):
    values = {}
    for path in sorted(root.rglob("*")):
        if path.is_file() and path.name != "inventory.sha256.json":
            with path.open("rb") as stream:
                values[path.relative_to(root).as_posix()] = hashlib.file_digest(stream, "sha256").hexdigest()
    (root / "inventory.sha256.json").write_text(json.dumps(values, sort_keys=True, indent=2) + "\n")


def analyze(root):
    protocol_bytes = (root / "protocol.json").read_bytes()
    protocol = json.loads(protocol_bytes)
    if (root / "protocol.sha256").read_text().strip() != digest(protocol_bytes):
        raise ValueError("protocol hash mismatch")
    rows = json.loads((root / "results.json").read_bytes())
    if [(r["task"], r["repeat"]) for r in rows] != [tuple(v) for v in protocol["order"]]:
        raise ValueError("declared trial order/coverage mismatch")
    exported_usage = json.loads((root / "usage.json").read_bytes())
    responses, sent = {}, Counter()
    with Store(root / "campaign") as store:
        reservations = budget.reservations(store)
        if exported_usage != list(reservations.values()): raise ValueError("usage export differs from journal")
        keys = [r[0] for r in store.db.execute("SELECT digest FROM objects")]
        for key in keys: store.get(key)
        events = store.events()
        frozen = [store.get(e["payload"]["artifact"]) for e in events if e["kind"] == "pilot-protocol-frozen"]
        if frozen != [protocol_bytes]: raise ValueError("protocol differs from authoritative journal")
        retained = [store.load(e["payload"]["artifact"]) for e in events if e["kind"] == "pilot-trial-result"]
        if rows != retained: raise ValueError("results export differs from journal")
        lease = None
        for event in events:
            if event["kind"] == "model-request": lease = reservations[event["payload"]["id"]]["lease"]
            elif event["kind"] == "free-provider-wire-request": sent[lease] += 1
            elif event["kind"] == "free-provider-raw-response":
                payload = event["payload"]
                value = json.loads(store.get(payload["artifact"]))
                if payload["status"] == 200 and value.get("model") != protocol["model"]["id"]:
                    raise ValueError("accepted response model drift")
                responses.setdefault(lease, []).append({"status": payload["status"], "body": value,
                                                        "roundtrip_ms": payload["wall_time_ms"]})
        head = store.head()
    trials = []
    for row in rows:
        name = f'{row["task"]}-r{row["repeat"] + 1}'
        trial = root / "trials" / name
        records = [r for r in exported_usage if r["lease"] == name]
        received = responses.get(name, [])
        failed = any(r["status"] != 200 for r in received) or sent[name] > len(received)
        errors = "\n".join(p.read_text(errors="replace") for p in (trial / "raw").glob("run.stderr"))
        costs = [r["body"].get("usage", {}).get("cost") for r in received]
        if any(v is not None and v != 0 for v in costs): raise ValueError("nonzero reported cost")
        patch = trial / "targeted/candidate.patch"
        lines = patch.read_text(errors="replace").splitlines() if patch.exists() else None
        execution = trial / "raw/execution.json"
        runtime = json.loads(execution.read_bytes()).get("wall_time_ms") if execution.exists() else None
        receipt_path = trial / "raw/parent-inspect.stdout"
        changed_paths = None
        if receipt_path.exists():
            try:
                receipt = json.loads(receipt_path.read_bytes())
                if isinstance(receipt.get("changes"), list): changed_paths = [c["path"] for c in receipt["changes"]]
            except ValueError: pass
        trials.append({**row, "functional_success": functional(row),
            "failure_category": category(row, failed, errors), "provider_dispatch_attempts": sent[name],
            "reserved_model_attempts": len(records), "provider_statuses": [r["status"] for r in received],
            "tokens": {k: usage_analysis.aggregate(records, k) for k in ("input_tokens", "output_tokens", "cached_input_tokens")},
            "runtime_wall_ms": runtime, "provider_roundtrip_ms": sum(r["roundtrip_ms"] for r in received) if received else None,
            "model_inference_time_ms": None,
            "trace": trace_metrics(trial), "reported_cost": sum(costs) if costs and all(v is not None for v in costs) else None,
            "files_changed": len(changed_paths) if changed_paths is not None else None, "changed_paths": changed_paths,
            "cost_coverage": {"reported": sum(v is not None for v in costs), "responses": len(received)},
            "lines_added": sum(line.startswith("+") and not line.startswith("+++") for line in lines) if lines is not None else None,
            "lines_removed": sum(line.startswith("-") and not line.startswith("---") for line in lines) if lines is not None else None})
    report = {"protocol_sha256": digest(protocol_bytes), "commit": protocol["commit"], "model": protocol["model"]["id"],
        "declared_trials": len(rows), "attempted_trials": sum(r["status"] != "not-attempted" for r in rows),
        "strict_successes": sum(r["strict_success"] for r in rows),
        "functional_successes": sum(r["functional_success"] is True for r in trials),
        "functionally_graded_trials": sum(r["functional_success"] is not None for r in trials),
        "by_task": {task["id"]: {"strict_successes": sum(r["strict_success"] for r in trials if r["task"] == task["id"]),
                                 "declared_trials": sum(r["task"] == task["id"] for r in trials)} for task in protocol["tasks"]},
        "tokens": {k: usage_analysis.aggregate(exported_usage, k) for k in ("input_tokens", "output_tokens", "cached_input_tokens")},
        "provider_dispatch_attempts": sum(sent.values()), "reserved_model_attempts": len(exported_usage),
        "failure_taxonomy": dict(Counter(r["failure_category"] for r in trials)),
        "assurance": {k: {"passed": sum(r.get("checks", {}).get(k) is True for r in rows),
                           "checked": sum(k in r.get("checks", {}) for r in rows)}
                      for k in ("source_isolation_valid", "trace_valid", "receipt_valid", "ready_to_apply")},
        "account_before": protocol["account_before"], "account_after": json.loads((root / "account-after.json").read_bytes()),
        "invoice_verified": False, "unlimited_access_verified": False, "causal_improvement_evidence": False,
        "artifact_count": len(keys), "journal_head": head, "trials": trials}
    (root / "analysis.json").write_text(json.dumps(report, sort_keys=True, indent=2) + "\n")
    inventory(root)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    report = analyze(parser.parse_args().output.resolve())
    print(json.dumps({k: v for k, v in report.items() if k != "trials"}, sort_keys=True, indent=2))
