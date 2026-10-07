"""Paired task-cluster inference, with missing trials counted as failures."""
import itertools
import math
import random

from .safe import Refusal, canonical, digest


def paired(rows, task_ids, repetitions, control, treatment, seed=0, samples=10000):
    expected = {(task, arm, repeat) for task in task_ids for arm in (control, treatment)
                for repeat in range(repetitions)}
    selected = {}
    for row in rows:
        if row["arm"] not in (control, treatment):
            continue
        key = (row["task"], row["arm"], row["repeat"])
        if key not in expected or key in selected or type(row.get("strict_completion")) is not bool:
            raise Refusal("duplicate, foreign or unknown trial outcome")
        selected[key] = row["strict_completion"]
    if set(selected) != expected:
        raise Refusal("experiment coverage is incomplete; no promotion verdict")
    differences = [sum(int(selected[(t, treatment, r)]) - int(selected[(t, control, r)])
                       for r in range(repetitions)) / repetitions for t in sorted(task_ids)]
    observed = sum(differences) / len(differences)
    rng = random.Random(seed)
    boot = sorted(sum(rng.choice(differences) for _ in differences) / len(differences) for _ in range(samples))
    lower, upper = boot[math.floor(.025 * (samples - 1))], boot[math.ceil(.975 * (samples - 1))]
    nonzero = [d for d in differences if d]
    # Exact one-sided sign randomization for up to 16 nonzero task clusters;
    # larger suites use a frozen Monte Carlo seed and add-one correction.
    if len(nonzero) <= 16:
        values = [sum(d * sign for d, sign in zip(nonzero, signs))
                  for signs in itertools.product((-1, 1), repeat=len(nonzero))]
        p = sum(v >= sum(nonzero) - 1e-12 for v in values) / len(values)
        method = "exact-task-sign-randomization"
    else:
        values = [sum(d * rng.choice((-1, 1)) for d in nonzero) for _ in range(samples)]
        p = (1 + sum(v >= sum(nonzero) - 1e-12 for v in values)) / (samples + 1)
        method = "seeded-task-sign-randomization"
    return {"difference": observed, "cluster_interval_95": [lower, upper], "p_one_sided": p,
            "method": method, "tasks": len(differences), "trials_per_arm": len(selected) // 2,
            "seed": seed, "samples": samples}


def decide(rows, protocol, gates, alpha=.025):
    tasks = [task["id"] for task in protocol["tasks"]]
    a = paired(rows, tasks, protocol["repetitions"], "parent", "candidate", protocol["seed"])
    r = paired(rows, tasks, protocol["repetitions"], "restored", "candidate", protocol["seed"])
    credible = lambda result: result["difference"] >= .05 and result["cluster_interval_95"][0] > 0 and result["p_one_sided"] <= alpha
    all_gates = bool(gates) and all(value is True for value in gates.values())
    infrastructure_complete = all(row.get("status") == "completed" for row in rows)
    all_gates = all_gates and infrastructure_complete
    verdict = "supported" if all_gates and credible(a) and credible(r) else "inconclusive"
    if any(result["difference"] < 0 for result in (a, r)) or not all_gates:
        verdict = "rejected"
    return {"schema_version": 1, "verdict": verdict, "parent_comparison": a, "restored_comparison": r,
            "alpha": alpha, "gates_passed": all_gates, "raw_results_sha256": digest(canonical(rows)),
            "infrastructure_complete": infrastructure_complete,
            "claim": "fixture runtime mechanics only" if protocol.get("fixture") else "task-specific observed harness improvement"}
