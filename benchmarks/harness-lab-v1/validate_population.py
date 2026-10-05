#!/usr/bin/env python3
"""Explicit operator-side image preparation and serial base/gold validation.

All population outcomes are retained, including unsupported preparation. This
does not score models, replace failed tasks, or admit an experiment protocol.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
from types import SimpleNamespace

from container_grade import bounded_list, control, execute
from lab import read_json, sha256, validate_catalogue


def validate(args):
    catalogue = validate_catalogue(read_json(args.catalogue))
    prepared = {row["id"]: row for row in bounded_list(args.prepared)}
    images = {row["task"]: row for row in bounded_list(args.images)}
    if set(prepared) != set(catalogue) or set(images) != set(catalogue):
        raise ValueError("validation must retain the entire declared population")
    args.output.mkdir(parents=True, exist_ok=False)
    identity = {"catalogue_sha256": sha256(args.catalogue),
                "prepared_sha256": sha256(args.prepared), "images_sha256": sha256(args.images),
                "runner_sha256": sha256(Path(__file__)), "timeout_seconds": args.timeout,
                "order": sorted(catalogue, key=lambda name: (catalogue[name]["split"] != "development", name)),
                "scored": False, "grading_network": "none", "memory_limit": "6g", "cpus": 2,
                "parallel_cases": 1}
    (args.output / "campaign.json").write_text(json.dumps(identity, indent=2) + "\n")
    env = {key: os.environ[key] for key in ("PATH", "HOME", "LANG") if key in os.environ}
    results = []
    for name in identity["order"]:
        case = args.output / name
        case.mkdir()
        row = {"task": name, "status": "unsupported_preparation", "scored": False}
        if prepared[name]["status"] == "prepared_not_validated" and images[name]["status"] == "manifest_locked_not_executed":
            try:
                # Download is explicit preparation. execute() never pulls images.
                pull = control(["docker", "--host", "unix:///var/run/docker.sock", "pull", "--platform", "linux/amd64",
                                images[name]["immutable_image"]], env, timeout=900)
                (case / "image-pull.log").write_bytes(pull)
                reports = {}
                for source in ("base", "gold"):
                    reports[source] = execute(SimpleNamespace(**{**vars(args), "task": name,
                                                                 "source": source, "output": case / source}))
                row.update(status="validated" if all(r.get("qualified") for r in reports.values()) else "validation_failed",
                           base=reports["base"], gold=reports["gold"])
            except (OSError, ValueError, RuntimeError, KeyError, subprocess.SubprocessError) as error:
                row.update(status="infrastructure_failure", error=str(error))
        (case / "result.json").write_text(json.dumps(row, indent=2, allow_nan=False) + "\n")
        results.append(row)
        (args.output / "results.json").write_text(json.dumps(results, indent=2, allow_nan=False) + "\n")
        print(json.dumps({"task": name, "status": row["status"]}), flush=True)
    return results


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("catalogue", "prepared", "images", "parser-python", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=600)
    args = parser.parse_args()
    if not 1 <= args.timeout <= 3600:
        parser.error("timeout must be 1–3600 seconds")
    validate(args)
