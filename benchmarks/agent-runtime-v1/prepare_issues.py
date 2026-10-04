#!/usr/bin/env python3
"""Seal existing historical issue tasks; no model calls or credential handling."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile


def git(repository, *args):
    return subprocess.check_output(["git", "-C", str(repository), *args])


def export(repository, revision, destination):
    destination.mkdir()
    with tempfile.TemporaryFile() as archive:
        subprocess.run(["git", "-C", str(repository), "archive", revision], stdout=archive, check=True)
        archive.seek(0)
        with tarfile.open(fileobj=archive) as tree:
            tree.extractall(destination, filter="data")


def prepare(destination, pactrail):
    manifest = json.loads((pactrail / "benchmarks/issue-replay-v1/cases.json").read_text())
    tasks = []
    for case in manifest["tasks"]:
        name = case["id"]
        repository = pactrail if case["repository"] == "AKMessi/pactrail" else destination / "repositories" / case["repository"].split("/")[-1]
        baseline = destination / "tasks" / name
        baseline.parent.mkdir(exist_ok=True)
        export(repository, case["base_commit"], baseline)
        if name == "bytes-get-int-zero":
            cargo = baseline / "Cargo.toml"
            cargo.write_text(cargo.read_text().replace('name = "bytes"', 'name = "bytes"\nautobenches = false', 1))
        cargo = baseline / "Cargo.toml"
        if "[workspace]" not in cargo.read_text():
            cargo.write_text(cargo.read_text() + "\n[workspace]\n")
        # No future fixes, remotes or reference tests enter the model's Git history.
        subprocess.run(["git", "init", str(baseline)], capture_output=True, check=True)
        subprocess.run(["git", "-C", str(baseline), "add", "."], check=True)
        subprocess.run(["git", "-C", str(baseline), "-c", "user.name=Qualification", "-c", "user.email=qualification@invalid", "commit", "-m", "sealed historical baseline"], capture_output=True, check=True)
        gold = destination / "gold" / name
        gold.parent.mkdir(exist_ok=True)
        shutil.copytree(baseline, gold, ignore=shutil.ignore_patterns(".git"))
        changed = git(repository, "diff", "--name-only", case["base_commit"], case["reference_commit"]).decode().splitlines()
        for relative in changed:
            if relative.startswith(("src/", "crates/")) and relative not in case["hidden_overlay_paths"]:
                path = gold / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(git(repository, "show", case["reference_commit"] + ":" + relative))
        overlays = destination / "graders" / name
        overlays.mkdir(parents=True)
        for relative in case["hidden_overlay_paths"]:
            path = overlays / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(git(repository, "show", case["reference_commit"] + ":" + relative))
        entry = {**case, "repository": str(baseline), "commit": git(baseline, "rev-parse", "HEAD").decode().strip(),
                 "gold": str(gold), "overlays": str(overlays), "baseline": str(baseline)}
        for phase, key in [("targeted", "targeted_test"), ("regression", "regression_test")]:
            entry[phase] = [case[key]["program"], *case[key]["args"]]
        entry["overlay_sha256"] = {p: hashlib.sha256((overlays / p).read_bytes()).hexdigest() for p in case["hidden_overlay_paths"]}
        tasks.append(entry)
    (destination / "issues.json").write_text(json.dumps(tasks, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--pactrail", type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    prepare(args.destination.resolve(), args.pactrail.resolve())
