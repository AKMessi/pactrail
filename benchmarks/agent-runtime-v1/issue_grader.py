#!/usr/bin/env python3
"""External grading only; sealed reference tests never appear in agent requests."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess


def grade(manifest, task_id, phase, workspace):
    task = next(t for t in json.loads(manifest.read_text()) if t["id"] == task_id)
    for forbidden in task["forbidden_paths"]:
        before, after = Path(task["baseline"]) / forbidden, workspace / forbidden
        if before.is_dir():
            paths = {p.relative_to(before) for p in before.rglob("*") if p.is_file()}
            current = {p.relative_to(after) for p in after.rglob("*") if p.is_file()} if after.is_dir() else set()
            if paths != current or any((before / p).read_bytes() != (after / p).read_bytes() for p in paths):
                raise ValueError("forbidden directory changed: " + forbidden)
        elif before.exists() != after.exists() or before.exists() and before.read_bytes() != after.read_bytes():
            raise ValueError("forbidden file changed: " + forbidden)
    for relative, expected in task["overlay_sha256"].items():
        source = Path(task["overlays"]) / relative
        if hashlib.sha256(source.read_bytes()).hexdigest() != expected:
            raise ValueError("sealed grader hash changed")
        target = workspace / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
    command = task[phase]
    lock = Path(task["baseline"]) / "Cargo.lock"
    if lock.exists():
        shutil.copyfile(lock, workspace / "Cargo.lock")
    # copytree preserves historical timestamps. Cargo can otherwise reuse a
    # bad/gold path-dependency artifact across grading copies in a shared target.
    # Touch only the disposable grading inputs; dependency caches stay reusable.
    for path in workspace.rglob("*"):
        if path.is_file() and (path.suffix == ".rs" or path.name == "Cargo.toml"):
            os.utime(path, None)
    # Known historical repositories are trusted grader executables. Their
    # dependencies are fetched during validation, never by the model.
    env = {k: os.environ[k] for k in ("PATH", "HOME", "LANG", "TMPDIR") if k in os.environ}
    env.update(CARGO_BUILD_JOBS="1", CARGO_NET_OFFLINE="true",
               CARGO_TARGET_DIR=str(manifest.parent / "grader-targets" / task_id))
    return subprocess.run(command, cwd=workspace, env=env, check=False).returncode


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("task_id")
    parser.add_argument("phase", choices=("targeted", "regression"))
    args = parser.parse_args()
    raise SystemExit(grade(args.manifest.resolve(), args.task_id, args.phase, Path.cwd()))
