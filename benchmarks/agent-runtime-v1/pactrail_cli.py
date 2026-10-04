#!/usr/bin/env python3
"""Real single/text CLI adapter. Current CLI adapters cannot expose hidden states."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys


def invoke(argv, root, name):
    with (root / (name + ".stdout")).open("wb") as stdout, (root / (name + ".stderr")).open("wb") as stderr:
        return subprocess.run(argv, stdout=stdout, stderr=stderr, check=False).returncode


def main():
    request = json.loads(Path(sys.argv[1]).read_text())
    root = Path(request["trial_directory"])
    arm = request["arm"]
    if request["permissions"] != {"process": "disabled", "write_paths": ["."]}:
        raise ValueError("this CLI adapter supports only disabled commands and workspace-scoped writes")
    if arm["mode"] == "latent":
        (root / "unsupported.txt").write_text("First-party CLI providers do not expose internal-state export/import. No text fallback was performed.\n")
        return 78
    binary = str(Path(os.environ["PACTRAIL_BENCH_BINARY"]).resolve(strict=True))
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
    if arm["mode"] == "text":
        profile = json.loads(subprocess.check_output(base + ["agent-template"]))
        profile["budget"]["max_model_attempts"] = limits["model_turns"]
        for agent in profile["agents"]:
            agent["max_turns"] = min(agent["max_turns"], limits["model_turns"])
        profile_path = root / "agents.json"
        profile_path.write_text(json.dumps(profile))
        argv += ["--agent-config", str(profile_path)]
    code = invoke(argv, root, "run")
    if code != 0:
        return code
    result = json.loads((root / "run.stdout").read_text())
    trace = Path(result["trace"])
    receipt = Path(result["receipt"])
    rows = [json.loads(line) for line in trace.read_text().splitlines() if line.strip()]
    model = [row["event"]["data"] for row in rows if row["event"]["type"] == "action_completed"
             and row["event"]["data"]["actor"].startswith("model:") and row["event"]["data"]["action"] == "invoke"]
    def total(attribute):
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
    if arm["mode"] == "text":
        if invoke(base + ["agents", result["run_id"], "--json"], root, "agents") != 0:
            raise ValueError("agent status failed validation")
        accounting = json.loads((root / "agents.stdout").read_text())["accounting"]
        measured.update({"model_attempts": accounting["model_attempts"], "communication_rounds": accounting["communication_rounds"],
                         "inter_agent_text_tokens": accounting["intermediate_text_tokens"], "latent_messages": 0,
                         "latent_logical_bytes": accounting["latent_logical_bytes"], "latent_stored_bytes": accounting["latent_stored_bytes"]})
    with Path(binary).open("rb") as executable:
        binary_digest = hashlib.file_digest(executable, "sha256").hexdigest()
    (root / "result.json").write_text(json.dumps({"schema_version": 1, "provenance": {"binary_sha256": binary_digest}, "model_identity": identity,
        "candidate": str(receipt.parent / "workspace"), "metrics": measured, "usage_coverage": {key: {"positive_reported_turns": sum(int(r.get("attributes", {}).get(key, "0")) > 0 for r in model), "total_turns": len(model), "zero_fields": "not distinguishable from absent in the legacy normalized ledger"} for key in ("input_tokens", "output_tokens", "cached_input_tokens")}}, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
