#!/usr/bin/env python3
"""Export pre-fix trees and seal gold/grader material outside agent repositories.

Explicit preparation uses already-fetched, operator-owned local Git mirrors.
It performs no model requests, upstream fetching, dependency install or scoring.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import tarfile
import tempfile

from lab import canonical, read_json, sha256, validate_catalogue


MAX_FILES = 100_000
MAX_TREE_BYTES = 512 * 1024 * 1024


def git(repository, *args, **kwargs):
    env = {"PATH": os.environ["PATH"], "GIT_CONFIG_NOSYSTEM": "1",
           "GIT_CONFIG_GLOBAL": os.devnull, "GIT_TERMINAL_PROMPT": "0"}
    return subprocess.run(["git", "-c", "core.hooksPath=" + os.devnull,
                           "-C", str(repository), *args], env=env, timeout=120,
                          check=True, **kwargs)


def export(repository, revision, destination):
    # Bound admission before extracting any untrusted archive entry.
    metadata = git(repository, "ls-tree", "-rzl", revision, stdout=subprocess.PIPE,
                   stderr=subprocess.PIPE).stdout
    entries = [entry for entry in metadata.split(b"\0") if entry]
    if len(entries) > MAX_FILES:
        raise ValueError("repository file count exceeds preparation bound")
    total = 0
    for entry in entries:
        info, _ = entry.split(b"\t", 1)
        mode, kind, _, size = info.split()
        if mode not in (b"100644", b"100755") or kind != b"blob":
            raise ValueError("nonregular tracked file is unsupported")
        total += int(size)
        if total > MAX_TREE_BYTES:
            raise ValueError("repository bytes exceed preparation bound")
    with tempfile.TemporaryFile() as archive:
        git(repository, "archive", revision, stdout=archive, stderr=subprocess.PIPE)
        if archive.tell() > MAX_TREE_BYTES:
            raise ValueError("repository archive exceeds preparation bound")
        archive.seek(0)
        with tarfile.open(fileobj=archive) as tree:
            total = 0
            count = 0
            for member in tree:
                path = PurePosixPath(member.name)
                if path.is_absolute() or ".." in path.parts or ".git" in path.parts:
                    raise ValueError("unsafe archive path")
                if not (member.isfile() or member.isdir()):
                    raise ValueError("symlink or special archive member: " + member.name)
                total += member.size
                count += 1
                if total > MAX_TREE_BYTES or count > MAX_FILES:
                    raise ValueError("repository tree exceeds preparation bound")
            archive.seek(0)
            with tarfile.open(fileobj=archive) as checked:
                checked.extractall(destination, filter="data")


def tree_identity(directory):
    records = []
    root = Path(directory)
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(root)
        if relative.parts[0] == ".git":
            continue
        if path.is_symlink():
            raise ValueError("symlink in prepared tree")
        if path.is_file():
            records.append({"path": relative.as_posix(), "sha256": sha256(path),
                            "executable": bool(path.stat().st_mode & 0o111)})
    return hashlib.sha256(canonical(records)).hexdigest()


def prepare(catalogue_path, source_directory, mirrors, output):
    catalogue = read_json(catalogue_path)
    tasks = validate_catalogue(catalogue)
    sources = {}
    for name, expected in catalogue["source_snapshots"].items():
        path = source_directory / name
        if Path(name).name != name or sha256(path) != expected:
            raise ValueError("source snapshot changed")
        for item in read_json(path)["rows"]:
            sources[item["row"]["instance_id"]] = item["row"]
    output.mkdir(parents=True, exist_ok=False)
    results = []
    for identity, task in sorted(tasks.items()):
        root = output / identity
        root.mkdir()
        result = {"id": identity, "status": "preparation_failed", "eligible_for_scoring": False}
        try:
            row = sources[task["upstream_instance_id"]]
            if row["repo"] != task["repository"] or row["base_commit"] != task["base_commit"]:
                raise ValueError("dataset identity mismatch")
            if hashlib.sha256(row["patch"].encode()).hexdigest() != task["reference_patch_sha256"]:
                raise ValueError("reference fix identity mismatch")
            mirror = mirrors / task["repository"].replace("/", "--")
            baseline = root / "baseline"
            baseline.mkdir()
            export(mirror, task["base_commit"], baseline)
            source_identity = tree_identity(baseline)
            git(baseline, "init", stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            git(baseline, "add", "--all", stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            git(baseline, "-c", "user.name=Harness Lab", "-c", "user.email=lab@invalid",
                "commit", "-m", "sealed pre-fix source", stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            commit = git(baseline, "rev-parse", "HEAD", stdout=subprocess.PIPE).stdout.decode().strip()
            # Gold and graders are siblings of the agent source, never its files/history.
            gold = root / "gold"
            shutil.copytree(baseline, gold)
            reference = root / "reference.patch"
            reference.write_text(row["patch"])
            git(gold, "apply", "--check", str(reference.resolve()), stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            git(gold, "apply", str(reference.resolve()), stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            gold_identity = tree_identity(gold)
            grader = {key: row[key] for key in ("test_patch", "eval_script", "log_parser",
                                               "FAIL_TO_PASS", "PASS_TO_PASS")}
            if hashlib.sha256(canonical(grader)).hexdigest() != task["grader_identity"]:
                raise ValueError("grader identity mismatch")
            (root / "grader.json").write_text(json.dumps(grader, indent=2) + "\n")
            result.update(status="prepared_not_validated", baseline=str(baseline.resolve()),
                          gold=str(gold.resolve()), synthetic_commit=commit,
                          source_tree_sha256=source_identity, gold_tree_sha256=gold_identity,
                          task_sha256=hashlib.sha256(canonical(task)).hexdigest(),
                          image_tag=row.get("image"), image_digest=None,
                          goal=row["problem_statement"])
        except (OSError, ValueError, KeyError, subprocess.SubprocessError, tarfile.TarError) as error:
            result["error"] = f"{type(error).__name__}: {error}"
            if isinstance(error, subprocess.CalledProcessError) and error.stderr:
                (root / "preparation.stderr").write_bytes(error.stderr)
        (root / "preparation.json").write_text(json.dumps(result, indent=2) + "\n")
        results.append(result)
        (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    return results


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("catalogue", type=Path)
    parser.add_argument("source_snapshots", type=Path)
    parser.add_argument("mirrors", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    prepare(args.catalogue, args.source_snapshots, args.mirrors, args.output)
