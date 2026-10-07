#!/usr/bin/env python3
"""Task-clustered paired bootstrap; reports coverage, not a synthetic leaderboard."""
import argparse
import json
from pathlib import Path
import random


def paired(rows, control, treatment, seed=42, samples=10000, metric="task_success"):
    if control == treatment or metric not in ("task_success", "strict_completion"):
        raise ValueError("distinct arms and a supported outcome metric required")
    if type(samples) is not int or not 1 <= samples <= 1000000:
        raise ValueError("bounded positive bootstrap samples required")
    groups = {}
    for row in rows:
        if row["arm"] not in (control, treatment):
            continue
        if row["status"] not in ("scored", "failed", "invalid", "unsupported"):
            raise ValueError("unknown trial status")
        arms = groups.setdefault((row["task"], row["repeat"]), {})
        if row["arm"] in arms:
            raise ValueError("duplicate trial identity")
        outcome = row.get(metric)
        if outcome is not None and type(outcome) is not bool:
            raise ValueError("outcome must be boolean or unknown")
        # Infrastructure/unsupported outcomes remain unsuccessful declared trials;
        # unknown strict assurance never turns into a confirmed success.
        arms[row["arm"]] = int(row["status"] == "scored" and outcome is True)
    tasks = {}
    for (task, _), arms in groups.items():
        if control in arms and treatment in arms:
            tasks.setdefault(task, []).append(arms[treatment] - arms[control])
    deltas = [sum(values) / len(values) for values in tasks.values()]
    if not deltas:
        return {"paired_tasks": 0, "difference": None, "interval_95": None}
    rng = random.Random(seed)
    resampled = sorted(sum(rng.choices(deltas, k=len(deltas))) / len(deltas) for _ in range(samples))
    return {"paired_tasks": len(deltas), "paired_trials": sum(len(x) for x in tasks.values()),
            "difference": sum(deltas) / len(deltas), "interval_95": [resampled[int(samples * .025)], resampled[int(samples * .975)]],
            "method": "task-clustered paired bootstrap; small task counts do not establish generalization"}


def resource_summary(rows, arm):
    selected = [row for row in rows if row["arm"] == arm]
    keys = ("input_tokens", "output_tokens", "cached_input_tokens", "uncached_input_tokens",
            "model_turns", "model_time_ms", "tool_calls", "communication_messages", "communication_bytes")
    report = {"declared_trials": len(selected), "successes": sum(row["task_success"] for row in selected)}
    for key in keys + ("wall_time_ms",):
        values = []
        by_task = {}
        for row in selected:
            value = (row.get("execution") or {}).get(key) if key == "wall_time_ms" else row["metrics"].get(key)
            if value is not None:
                values.append(value)
                by_task.setdefault(row["task"], []).append(value)
        report[key] = {"coverage": len(values), "total": sum(values) if values else None,
                       "task_mean": sum(sum(v) / len(v) for v in by_task.values()) / len(by_task) if by_task else None}
    def reported_complete(row, key):
        coverage = (row.get("usage_coverage") or {}).get(key) or {}
        total = coverage.get("total_turns", 0)
        return type(total) is int and total > 0 and coverage.get("explicit_reported_turns") == total
    covered = [r for r in selected if all(reported_complete(r, key) and r["metrics"].get(key) is not None
                                        for key in ("input_tokens", "cached_input_tokens"))]
    inputs = sum(r["metrics"]["input_tokens"] for r in covered)
    report["cache_hit_ratio"] = {"covered_trials": len(covered), "value": sum(r["metrics"]["cached_input_tokens"] for r in covered) / inputs if inputs else None}
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("results", type=Path)
    parser.add_argument("--control", required=True)
    parser.add_argument("--treatment", required=True)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--metric", choices=("task_success", "strict_completion"), default="task_success")
    args = parser.parse_args()
    rows = json.loads(args.results.read_text())
    report = paired(rows, args.control, args.treatment, args.seed, metric=args.metric)
    report["outcome_metric"] = args.metric
    report["coverage"] = {arm: {status: sum(row["arm"] == arm and row["status"] == status for row in rows)
                               for status in ("scored", "failed", "invalid", "unsupported")} for arm in (args.control, args.treatment)}
    report["resources"] = {arm: resource_summary(rows, arm) for arm in (args.control, args.treatment)}
    print(json.dumps(report, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
