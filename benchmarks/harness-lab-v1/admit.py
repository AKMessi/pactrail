#!/usr/bin/env python3
"""Export integrity-bound admission evidence from a complete grading campaign.

Behavioral exit codes are derived explicitly from complete named-test outcomes;
raw shell exits remain separate. Rejected cases stay in the export ledger.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil

from container_grade import bounded_list
from grading import summarize
from parse_upstream import UPSTREAM_COMMIT
from lab import canonical, read_json, sha256, validate_catalogue, validate_evidence


def checked_report(case, source, task):
    root = case / source
    report = read_json(root / "result.json")
    parsed = read_json(root / "parsed.json")
    if parsed.get("upstream_commit") != UPSTREAM_COMMIT:
        raise ValueError("parser upstream identity mismatch")
    if report.get("task") != task["id"] or report.get("source") != source:
        raise ValueError("report identity mismatch")
    if report.get("grading_source_commit") != task["base_commit"]:
        raise ValueError("grading source commit mismatch")
    if report.get("grader_identity") != task["grader_identity"]:
        raise ValueError("report grader mismatch")
    if report.get("timed_out") is not False or report.get("output_limit") is not False or report.get("infrastructure_error"):
        raise ValueError("incomplete grading execution")
    log = root / "test.log"
    if sha256(log) != report.get("log_sha256") or sha256(log) != parsed.get("log_sha256"):
        raise ValueError("grading log digest mismatch")
    return report, parsed["statuses"], log


def export(catalogue_path, prepared_path, campaign_path, output):
    catalogue = read_json(catalogue_path)
    tasks = validate_catalogue(catalogue)
    campaign_path = Path(campaign_path)
    campaign = read_json(campaign_path / "campaign.json")
    if campaign.get("catalogue_sha256") != sha256(catalogue_path) or campaign.get("scored") is not False:
        raise ValueError("unscored matching campaign required")
    if campaign.get("prepared_sha256") != sha256(prepared_path):
        raise ValueError("prepared identity mismatch")
    prepared = {row["id"]: row for row in bounded_list(prepared_path)}
    order = campaign.get("order")
    if not isinstance(order, list) or len(order) != len(tasks) or set(order) != set(tasks):
        raise ValueError("complete population campaign required")
    # A running campaign cannot produce a final cohort admission ledger.
    rows = [read_json(campaign_path / name / "result.json") for name in order]
    output = Path(output)
    output.mkdir(parents=True, exist_ok=False)
    results = []
    for name, row in zip(order, rows):
        task = tasks[name]
        record = {"task": name, "status": "not_admitted", "qualification_status": row.get("status")}
        try:
            if row.get("task") != name or row.get("status") != "validated":
                raise ValueError("case did not validate base and gold")
            case = campaign_path / name
            base, base_statuses, base_log = checked_report(case, "base", task)
            gold, gold_statuses, gold_log = checked_report(case, "gold", task)
            for report, identity in ((base, "source_tree_sha256"), (gold, "gold_tree_sha256")):
                if report.get("source_tree_sha256") != prepared[name][identity]:
                    raise ValueError("prepared source tree identity mismatch")
            grader = read_json(Path(prepared[name]["baseline"]).parent / "grader.json")
            if hashlib.sha256(canonical(grader)).hexdigest() != task["grader_identity"]:
                raise ValueError("grader content identity mismatch")
            for report, statuses in ((base, base_statuses), (gold, gold_statuses)):
                for phase, declaration in (("targeted", "FAIL_TO_PASS"), ("regression", "PASS_TO_PASS")):
                    if summarize(grader[declaration], statuses) != report[phase]:
                        raise ValueError("grading summary differs from parsed named tests")
            evidence = {"schema_version": 1, "task": name,
                        "task_sha256": hashlib.sha256(canonical(task)).hexdigest(),
                        "grader_identity": task["grader_identity"]}
            for key, report, phase, log in (("base_targeted", base, "targeted", base_log),
                                            ("gold_targeted", gold, "targeted", gold_log),
                                            ("gold_regression", gold, "regression", gold_log)):
                outcome = report[phase]
                expected = outcome.get("behavior_failed") if key == "base_targeted" else outcome.get("behavior_passed")
                if expected is not True or outcome.get("coverage_complete") is not True:
                    raise ValueError("complete behavioral result required")
                local = name + ("-base.log" if key == "base_targeted" else "-gold.log")
                evidence[key] = {"tests_executed": outcome["tests_executed"],
                                 "tests_failed": outcome["tests_failed"],
                                 "exit_code": 1 if key == "base_targeted" else 0,
                                 "exit_code_kind": "derived_behavioral",
                                 "shell_exit_code": report["shell_exit_code"],
                                 "timed_out": False, "infrastructure_error": False,
                                 "log": local, "log_sha256": sha256(log)}
                if not (output / local).exists():
                    shutil.copyfile(log, output / local)
            validate_evidence(task, evidence, output)
            (output / (name + ".json")).write_text(json.dumps(evidence, indent=2) + "\n")
            record.update(status="admitted", evidence_sha256=sha256(output / (name + ".json")))
        except (ValueError, KeyError, TypeError, OSError) as error:
            record["reason"] = str(error)
        results.append(record)
    (output / "admission-ledger.json").write_text(json.dumps({
        "catalogue_sha256": sha256(catalogue_path), "campaign_sha256": sha256(campaign_path / "campaign.json"),
        "scored": False, "cases": results}, indent=2) + "\n")
    return results


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("catalogue", type=Path)
    parser.add_argument("prepared", type=Path)
    parser.add_argument("campaign", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    print(json.dumps(export(args.catalogue, args.prepared, args.campaign, args.output)))
