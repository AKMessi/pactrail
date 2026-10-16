#!/usr/bin/env python3
"""Audit a completed pilot without rewriting frozen inputs or missing metrics."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "experiments/self-improvement-v1"))
from pactrail_lab.store import Store
from pactrail_lab import budget


def aggregate(rows, field):
    values = [r["usage"][field] for r in rows
              if isinstance(r.get("usage"), dict) and r["usage"].get(field) is not None]
    return {"reported_sum": sum(values) if values else None,
            "reported_requests": len(values), "total_requests": len(rows),
            "coverage": "complete" if values and len(values) == len(rows) else "partial" if values else "absent"}


def analyze(root):
    protocol_bytes = (root / "protocol.json").read_bytes()
    protocol_hash = hashlib.sha256(protocol_bytes).hexdigest()
    if (root / "protocol.sha256").read_text().strip() != protocol_hash:
        raise ValueError("protocol integrity mismatch")
    protocol = json.loads(protocol_bytes)
    results = json.loads((root / "results.json").read_bytes())
    usage = json.loads((root / "usage.json").read_bytes())
    if [(r["task"], r["repeat"]) for r in results] != [tuple(v) for v in protocol["order"]]:
        raise ValueError("declared trial order/coverage mismatch")
    with Store(root / "campaign") as store:
        if usage != list(budget.reservations(store).values()):
            raise ValueError("usage export differs from authoritative journal")
        # Store validates hash-chain/projections. Read each object to verify
        # retained binary artifacts as well, one bounded object at a time.
        keys = [row[0] for row in store.db.execute("SELECT digest FROM objects")]
        for key in keys:
            store.get(key)
        events = store.events()
        statuses = [e["payload"]["status"] for e in events if e["kind"] == "free-provider-response"]
        model_ids = []
        for event in events:
            if event["kind"] == "free-provider-raw-response" and event["payload"]["status"] == 200:
                response = json.loads(store.get(event["payload"]["artifact"]))
                model_ids.append(response.get("model"))
        if any(model != protocol["model"]["id"] for model in model_ids):
            raise ValueError("accepted response model identity differs from protocol")
        head = store.head()
    attempted = [r for r in results if r["status"] != "not-attempted"]
    report = {
        "protocol_sha256": protocol_hash, "commit": protocol["commit"],
        "model": protocol["model"]["id"], "response_models": model_ids,
        "declared_trials": len(results), "attempted_trials": len(attempted),
        "unattempted_trials": len(results) - len(attempted),
        "strict_successes": sum(r["strict_success"] for r in results),
        "ready_to_apply_trials": sum(r.get("checks", {}).get("ready_to_apply") is True for r in results),
        "physical_requests": len(usage), "provider_statuses": statuses,
        "tokens": {field: aggregate(usage, field) for field in
                   ("input_tokens", "output_tokens", "cached_input_tokens")},
        "assurance": {field: {"passed": sum(r.get("checks", {}).get(field) is True for r in attempted),
                              "checked": sum(field in r.get("checks", {}) for r in attempted)}
                      for field in ("source_isolation_valid", "trace_valid", "receipt_valid")},
        "artifact_count": len(keys), "journal_head": head,
        "quality_qualified": bool(attempted) and len(attempted) == len(results) and all(s == 200 for s in statuses) and bool(statuses),
        "causal_improvement_evidence": False,
        "billing": {"free_plan_operator_attested": True, "invoice_verified": False},
    }
    (root / "analysis.json").write_text(json.dumps(report, sort_keys=True, indent=2) + "\n")
    inventory = {}
    for path in sorted(root.rglob("*")):
        if path.is_file() and path.name != "inventory.sha256.json":
            with path.open("rb") as stream:
                inventory[path.relative_to(root).as_posix()] = hashlib.file_digest(stream, "sha256").hexdigest()
    (root / "inventory.sha256.json").write_text(json.dumps(inventory, sort_keys=True, indent=2) + "\n")
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    print(json.dumps(analyze(parser.parse_args().output.resolve()), sort_keys=True, indent=2))
