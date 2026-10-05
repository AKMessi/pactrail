#!/usr/bin/env python3
"""Deterministic candidate population from captured public dataset-server rows.

Gold/test patches stay in the private input snapshots. The output is a candidate
registry, not proof of local executable validity or independent curation.
"""
import argparse
import hashlib
import json
from pathlib import Path

from lab import canonical, read_json, sha256, validate_catalogue


REPOSITORIES = {
    "psf/requests": "python", "pytest-dev/pytest": "python",
    "pylint-dev/pylint": "python", "sympy/sympy": "python",
    "astral-sh/ruff": "rust", "tokio-rs/axum": "rust",
    "tokio-rs/tokio": "rust", "sharkdp/bat": "rust",
    "axios/axios": "javascript", "immutable-js/immutable-js": "javascript",
    "preactjs/preact": "javascript", "vuejs/core": "typescript",
    "caddyserver/caddy": "go", "gin-gonic/gin": "go", "gohugoio/hugo": "go",
}
SEED = "pactrail-quality-population-v1-20261005"


def rank(value):
    return hashlib.sha256((SEED + ":" + value).encode()).hexdigest()


def curate(inputs, output):
    records = {}
    snapshots = {}
    for path in inputs:
        snapshots[str(path.name)] = sha256(path)
        for item in read_json(path)["rows"]:
            row = item["row"]
            if row["repo"] in REPOSITORIES:
                if row["instance_id"] in records:
                    raise ValueError("duplicate dataset instance")
                records[row["instance_id"]] = (row, path.name)
    # Repository-disjoint confirmation; stratify before picking the partition.
    splits = {}
    families = {"typescript": "javascript"}
    for language in ("python", "rust", "javascript", "go"):
        repos = sorted((r for r, lang in REPOSITORIES.items()
                        if families.get(lang, lang) == language), key=rank)
        for index, repo in enumerate(repos):
            splits[repo] = "development" if index < (len(repos) + 1) // 2 else "confirmation"
    tasks = []
    for repo, language in sorted(REPOSITORIES.items()):
        choices = sorted((entry for entry in records.values() if entry[0]["repo"] == repo),
                         key=lambda entry: rank(entry[0]["instance_id"]))
        if len(choices) < 2:
            raise ValueError("two historical issues required: " + repo)
        for row, source in choices[:2]:
            grader = {key: row[key] for key in ("test_patch", "eval_script", "log_parser",
                                               "FAIL_TO_PASS", "PASS_TO_PASS")}
            tasks.append({"id": row["instance_id"].replace("__", "-").replace("_", "-"),
                          "upstream_instance_id": row["instance_id"],
                          "repository": repo, "language": language,
                          "source": source, "source_snapshot_sha256": snapshots[source],
                          "base_commit": row["base_commit"], "split": splits[repo],
                          "goal_sha256": hashlib.sha256(row["problem_statement"].encode()).hexdigest(),
                          "grader_identity": hashlib.sha256(canonical(grader)).hexdigest(),
                          "reference_patch_sha256": hashlib.sha256(row["patch"].encode()).hexdigest(),
                          "validation": "pending_local_base_and_gold_execution"})
    catalogue = {"schema_version": 1, "population_seed": SEED,
                 "partition_policy": "repository_disjoint",
                 "selection": "two hash-ranked issues per declared repository; repository-disjoint stratified split",
                 "curation": "public upstream task selection, not independent Pactrail held-out curation",
                 "source_snapshots": snapshots, "tasks": sorted(tasks, key=lambda task: task["id"])}
    validate_catalogue(catalogue)
    with Path(output).open("x") as stream:
        stream.write(json.dumps(catalogue, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("inputs", nargs="+", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    curate(args.inputs, args.output)
