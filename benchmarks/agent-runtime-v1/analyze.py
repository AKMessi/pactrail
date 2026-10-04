#!/usr/bin/env python3
"""Task-clustered paired bootstrap; reports coverage, not a synthetic leaderboard."""
import argparse
import json
from pathlib import Path
import random


def paired(rows, control, treatment, seed=42, samples=10000):
    groups = {}
    for row in rows:
        if row["arm"] in (control, treatment) and row["status"] in ("scored", "failed"):
            groups.setdefault((row["task"], row["repeat"]), {})[row["arm"]] = int(row["task_success"])
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("results", type=Path)
    parser.add_argument("--control", required=True)
    parser.add_argument("--treatment", required=True)
    parser.add_argument("--seed", type=int, default=42)
    args = parser.parse_args()
    rows = json.loads(args.results.read_text())
    report = paired(rows, args.control, args.treatment, args.seed)
    report["coverage"] = {arm: {status: sum(row["arm"] == arm and row["status"] == status for row in rows)
                               for status in ("scored", "failed", "invalid", "unsupported")} for arm in (args.control, args.treatment)}
    print(json.dumps(report, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
