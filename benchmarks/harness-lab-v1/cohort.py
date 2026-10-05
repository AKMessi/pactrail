#!/usr/bin/env python3
"""Derive an eligible cohort without deleting the original population outcomes.

Eligibility is environment-based and must be disclosed as selection bias. This
operation precedes scoring and does not examine model outcomes.
"""
import argparse
import json
from pathlib import Path

from lab import read_json, sha256, validate_catalogue, validate_evidence


def derive(population_path, admission_path, output):
    population = read_json(population_path)
    tasks = validate_catalogue(population)
    admission_path = Path(admission_path)
    ledger_path = admission_path / "admission-ledger.json"
    ledger = read_json(ledger_path)
    if ledger.get("catalogue_sha256") != sha256(population_path) or ledger.get("scored") is not False:
        raise ValueError("matching unscored admission ledger required")
    cases = ledger.get("cases")
    if not isinstance(cases, list) or len(cases) != len(tasks) or any(not isinstance(c, dict) for c in cases):
        raise ValueError("complete population admission ledger required")
    if {c.get("task") for c in cases} != set(tasks):
        raise ValueError("admission cases differ from population")
    selected = []
    exclusions = []
    inputs = {str(Path(population_path).resolve()): sha256(population_path),
              str(ledger_path.resolve()): sha256(ledger_path)}
    for case in cases:
        name = case["task"]
        if case.get("status") == "admitted":
            evidence_path = admission_path / (name + ".json")
            if sha256(evidence_path) != case.get("evidence_sha256"):
                raise ValueError("admitted evidence digest changed")
            evidence = read_json(evidence_path)
            validate_evidence(tasks[name], evidence, admission_path)
            inputs[str(evidence_path.resolve())] = sha256(evidence_path)
            for phase in ("base_targeted", "gold_targeted", "gold_regression"):
                log = admission_path / evidence[phase]["log"]
                inputs[str(log.resolve())] = sha256(log)
            selected.append(tasks[name])
        elif case.get("status") == "not_admitted":
            exclusions.append({"task": name, "qualification_status": case.get("qualification_status"),
                               "reason": case.get("reason")})
        else:
            raise ValueError("unknown admission state")
    cohort = {**population, "tasks": sorted(selected, key=lambda task: task["id"]),
              "eligibility": {"policy": "complete local base failure and gold targeted/regression passes",
                              "model_outcomes_used": False, "original_population_count": len(tasks),
                              "eligible_count": len(selected), "excluded": exclusions,
                              "limitation": "environment-qualified public cohort; selection bias, not independent curation"},
              "qualification_inputs": inputs}
    validate_catalogue(cohort)
    with Path(output).open("x") as stream:
        stream.write(json.dumps(cohort, indent=2, allow_nan=False) + "\n")
    return cohort


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("population", type=Path)
    parser.add_argument("admission", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    print(json.dumps({"eligible": len(derive(args.population, args.admission, args.output)["tasks"])}))
