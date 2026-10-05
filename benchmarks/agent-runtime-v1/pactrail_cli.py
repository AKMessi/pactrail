#!/usr/bin/env python3
"""Real single/text CLI adapter. Current CLI adapters cannot expose hidden states."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys


def invoke(argv, root, name):
    with (root / (name + ".stdout")).open("wb") as stdout, (root / (name + ".stderr")).open("wb") as stderr:
        return subprocess.run(argv, stdout=stdout, stderr=stderr, check=False).returncode


def source_identity(workspace):
    # Sealed lab trials include ignored/new files in their isolation check;
    # Git status alone can miss these. The bounded implementation is shared
    # with external candidate grading, and is frozen with lab protocols.
    path = Path(__file__).resolve().parents[1] / "harness-lab-v1/candidate_patch.py"
    spec = importlib.util.spec_from_file_location("lab_candidate_identity", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.identity(workspace)


def main():
    request = json.loads(Path(sys.argv[1]).read_text())
    root = Path(request["trial_directory"])
    arm = request["arm"]
    if request["permissions"] != {"process": "disabled", "write_paths": ["."]}:
        raise ValueError("this CLI adapter supports only disabled commands and workspace-scoped writes")
    if arm["mode"] == "latent":
        (root / "unsupported.txt").write_text("First-party CLI providers do not expose internal-state export/import. No text fallback was performed.\n")
        return 78
    sealed = request.get("source_policy") == "sealed"
    source_before = source_identity(request["workspace"]) if sealed else None
    binary = str(Path(os.environ["PACTRAIL_BENCH_BINARY"]).resolve(strict=True))
    expected_binary = (request.get("runtime_identity") or {}).get("binary_sha256")
    if expected_binary:
        with Path(binary).open("rb") as executable:
            if hashlib.file_digest(executable, "sha256").hexdigest() != expected_binary:
                raise ValueError("runtime binary differs from frozen experiment")
    base = [binary, "--workspace", request["workspace"], "--state-dir", str(root / "state")]
    limits = request["limits"]
    template = subprocess.check_output(base + ["task-template", request["task"]["goal"]], text=True)
    for field, value in {"wall_time_seconds": limits["wall_seconds"], "model_tokens": limits["model_tokens"],
                         "max_model_attempts": limits["model_turns"]}.items():
        template, count = re.subn(r"(?m)^" + field + r"\s*=\s*\d+\s*$", f"{field} = {value}", template)
        if count != 1:
            raise ValueError(f"task template missing budget: {field}")
    contract = root / "task.toml"
    contract.write_text(template)
    identity = request["model_identity"]
    argv = base + ["run", "--task", str(contract), "--output", "json", "--no-stream", "--provider", identity["provider"],
                   "--model", identity["model"], "--max-turns", str(limits["model_turns"]),
                   "--context-tokens", str(limits["context_tokens"]), "--max-output-tokens", str(limits["output_tokens"]),
                   "--request-timeout-seconds", str(min(300, limits["wall_seconds"]))]
    pricing = identity.get("pricing")
    if pricing is not None:
        fields = {"input_price", "cached_input_price", "cache_creation_price", "output_price"}
        if set(pricing) != fields or any(type(value) is not int or not 0 <= value < 2**64 for value in pricing.values()):
            raise ValueError("pricing requires all four non-negative integer micro-USD-per-million rates")
        for field in sorted(fields):
            argv += ["--" + field.replace("_", "-"), str(pricing[field])]
    if identity.get("base_url"):
        argv += ["--base-url", identity["base_url"]]
    if identity.get("api_key_env"):
        argv += ["--api-key-env", identity["api_key_env"]]
    if identity.get("reasoning_effort"):
        argv += ["--reasoning-effort", identity["reasoning_effort"]]
    if arm["mode"] == "text":
        profile = json.loads(subprocess.check_output(base + ["agent-template"]))
        if arm.get("profile") == "no-critic":
            profile["agents"] = [a for a in profile["agents"] if a["role"] != "critic"]
        elif arm.get("profile") == "minimal":
            profile["agents"] = [a for a in profile["agents"] if a["role"] in ("localizer", "implementer")]
        elif arm.get("profile", "full") != "full":
            raise ValueError("unknown coding profile")
        profile["budget"]["max_model_attempts"] = limits["model_turns"]
        for agent in profile["agents"]:
            agent["max_turns"] = min(agent["max_turns"], limits["model_turns"])
        profile_path = root / "agents.json"
        profile_path.write_text(json.dumps(profile))
        argv += ["--agent-config", str(profile_path)]
    code = invoke(argv, root, "run")
    result = json.loads((root / "run.stdout").read_text()) if code == 0 else {}
    traces = list((root / "state").rglob("trace.jsonl"))
    if not result and len(traces) != 1:
        return code or 1
    trace = Path(result["trace"]) if result else traces[0]
    rows = [json.loads(line) for line in trace.read_text().splitlines() if line.strip()]
    model = [row["event"]["data"] for row in rows if row["event"]["type"] == "action_completed"
             and row["event"]["data"]["actor"].startswith("model:") and row["event"]["data"]["action"] == "invoke"]
    run_id = result.get("run_id", rows[0]["run_id"])
    checks = {"receipt_valid": None, "trace_valid": None, "ready_to_apply": None,
              "source_isolation_valid": not subprocess.check_output(["git", "-C", request["workspace"], "status", "--porcelain"], text=True).strip()}
    source_after = source_identity(request["workspace"]) if sealed else None
    if sealed:
        checks["source_isolation_valid"] = checks["source_isolation_valid"] and source_before == source_after
    checks["trace_valid"] = invoke(base + ["trace", run_id, "--json"], root, "validated-trace") == 0
    receipt = None
    if code == 0:
        checks["receipt_valid"] = invoke(base + ["inspect", run_id, "--json"], root, "inspect") == 0
        if not checks["receipt_valid"]:
            raise ValueError("receipt failed engine validation")
        receipt = json.loads(Path(result["receipt"]).read_text())
        checks["ready_to_apply"] = receipt["outcome"] == "ready_to_apply"
        invoke(base + ["diff", run_id, "--json"], root, "diff")
    reported_models = {row.get("attributes", {}).get("provider.model") for row in model}
    reported_models.discard(None)
    if identity.get("require_response_model") and any("provider.model" not in row.get("attributes", {}) for row in model):
        raise ValueError("provider response model identity was not reported for every scored turn")
    if reported_models and reported_models != {identity["model"]}:
        raise ValueError("provider response model identity differs from frozen model")
    def total(attribute):
        explicit = [row.get("attributes", {}).get("provider.reported_" + attribute) for row in model]
        if explicit and all(x is not None for x in explicit):
            return sum(int(x) for x in explicit)
        values = [row.get("attributes", {}).get(attribute) for row in model]
        # Legacy normalized Usage defaults absent provider fields to zero. Without
        # a reporting marker, zero cannot establish measured provider coverage.
        return sum(int(x) for x in values) if values and all(x is not None and int(x) > 0 for x in values) else None
    def reported_total(attribute):
        values = [row.get("attributes", {}).get(attribute) for row in model]
        return sum(int(x) for x in values) if values and all(x is not None for x in values) else None
    measured = {"model_turns": len(model), "input_tokens": total("input_tokens"), "output_tokens": total("output_tokens"),
                "cached_input_tokens": total("cached_input_tokens"), "model_time_ms": sum(row["duration_ms"] for row in model),
                "tool_calls": sum(row["event"]["type"] == "action_completed" and row["event"]["data"]["actor"].startswith("tool:") for row in rows),
                "cost_microusd": result.get("cost_microusd"), "inference_time_ms": reported_total("model_inference_ms"),
                "latent_import_bytes": reported_total("latent_import_bytes"), "latent_export_bytes": reported_total("latent_export_bytes"), "inter_agent_text_tokens": 0 if arm["mode"] == "single" else None}
    tools = [row["event"]["data"] for row in rows if row["event"]["type"] == "action_completed" and row["event"]["data"]["actor"].startswith("tool:")]
    writes = {"write_file", "edit_file", "replace_text", "apply_patch", "remove_file"}
    measured.update({"cache_creation_input_tokens": None,
        "read_calls": sum(t["actor"][5:] not in writes | {"run_process", "run_shell"} for t in tools),
        "write_calls": sum(t["actor"][5:] in writes for t in tools),
        "process_calls": sum(t["actor"][5:] in {"run_process", "run_shell"} for t in tools),
        "files_changed": len(receipt["changes"]) if receipt else None,
        "provider_errors": None, "timeouts": None,
        "recovery_events": sum(row["event"]["type"] == "note_recorded" and "resum" in str(row["event"]).lower() for row in rows),
        "communication_messages": 0, "communication_bytes": 0})
    if measured["input_tokens"] is not None and measured["cached_input_tokens"] is not None:
        measured["uncached_input_tokens"] = measured["input_tokens"] - measured["cached_input_tokens"]
    if (root / "diff.stdout").exists():
        try:
            diff = json.loads((root / "diff.stdout").read_text())["unified_diff"]
            measured["lines_added"] = sum(line.startswith("+") and not line.startswith("+++") for line in diff.splitlines())
            measured["lines_removed"] = sum(line.startswith("-") and not line.startswith("---") for line in diff.splitlines())
        except (ValueError, KeyError):
            pass
    if arm["mode"] == "text":
        if invoke(base + ["agents", run_id, "--json"], root, "agents") != 0:
            raise ValueError("agent status failed validation")
        accounting = json.loads((root / "agents.stdout").read_text())["accounting"]
        measured.update({"model_attempts": accounting["model_attempts"], "communication_rounds": accounting["communication_rounds"],
                         "inter_agent_text_tokens": accounting["intermediate_text_tokens"], "latent_messages": 0,
                         "communication_messages": accounting["messages"], "communication_bytes": accounting["text_bytes"],
                         "latent_logical_bytes": accounting["latent_logical_bytes"], "latent_stored_bytes": accounting["latent_stored_bytes"]})
    with Path(binary).open("rb") as executable:
        binary_digest = hashlib.file_digest(executable, "sha256").hexdigest()
    (root / "result.json").write_text(json.dumps({"schema_version": 1, "provenance": {"binary_sha256": binary_digest, "source_tree_before_sha256": source_before,
        "source_tree_after_sha256": source_after}, "model_identity": identity,
        "candidate": str(Path(result["receipt"]).parent / "workspace") if result else None, "checks": checks,
        "metrics": measured, "usage_coverage": {key: {"explicit_reported_turns": sum("provider.reported_" + key in r.get("attributes", {}) for r in model), "positive_legacy_turns": sum(int(r.get("attributes", {}).get(key, "0")) > 0 for r in model), "total_turns": len(model)} for key in ("input_tokens", "output_tokens", "cached_input_tokens")}}, indent=2) + "\n")
    return code


if __name__ == "__main__":
    sys.exit(main())
