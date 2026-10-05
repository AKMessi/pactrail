"""Build a bounded grading patch without modifying candidate or baseline.

This is trusted external grading preparation, not a second agent mutation API.
New and ignored files are included; deletions are preserved. No Git hooks,
external diff, textconv or caller Git configuration is admitted.
"""
import os
import hashlib
import json
from pathlib import Path
import shutil
import stat
import tempfile

MAX_FILES = 100_000
MAX_BYTES = 512 * 1024 * 1024
MAX_ENTRIES = 200_000
MAX_DEPTH = 128


def files(root):
    root = Path(root).resolve(strict=True)
    if not root.is_dir():
        raise ValueError("grading source must be a directory")
    result, size, visited = [], 0, 0
    pending = [(root, 0)]
    while pending:
        directory, depth = pending.pop()
        with os.scandir(directory) as entries:
            for entry in entries:
                relative = Path(entry.path).relative_to(root)
                if relative.parts[0] in (".git", ".pactrail"):
                    continue
                visited += 1
                if visited > MAX_ENTRIES or depth + 1 > MAX_DEPTH:
                    raise ValueError("grading source exceeds entry/depth bound")
                if ".git" in relative.parts:
                    raise ValueError("nested Git metadata in grading source")
                metadata = entry.stat(follow_symlinks=False)
                if stat.S_ISLNK(metadata.st_mode):
                    raise ValueError("symlink in grading source")
                if stat.S_ISDIR(metadata.st_mode):
                    pending.append((Path(entry.path), depth + 1))
                    continue
                if not stat.S_ISREG(metadata.st_mode):
                    raise ValueError("special file in grading source")
                size += metadata.st_size
                result.append(relative)
                if len(result) > MAX_FILES or size > MAX_BYTES:
                    raise ValueError("grading source exceeds file/byte bound")
    return sorted(result)


def build(baseline, candidate, control):
    baseline, candidate = Path(baseline).resolve(strict=True), Path(candidate).resolve(strict=True)
    before, after = files(baseline), files(candidate)
    env = {key: os.environ[key] for key in ("PATH", "HOME", "LANG") if key in os.environ}
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
    with tempfile.TemporaryDirectory(prefix="pactrail-grade-") as directory:
        staging = Path(directory)
        for relative in before:
            target = staging / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(baseline / relative, target)
        git = ["git", "-c", "core.hooksPath=" + os.devnull, "-C", str(staging)]
        control(git + ["init"], env)
        # Source-controlled attributes must not normalize bytes or invoke a
        # filter while preparing the grader's patch.
        (staging / ".git" / "info" / "attributes").write_text(
            "* -text -filter -ident -working-tree-encoding\n")
        control(git + ["add", "--force", "--all"], env)
        control(git + ["-c", "user.name=Harness Lab", "-c", "user.email=lab@invalid",
                       "commit", "--allow-empty", "-m", "grading baseline"], env)
        for path in staging.iterdir():
            if path.name != ".git":
                if path.is_dir():
                    shutil.rmtree(path)
                else:
                    path.unlink()
        for relative in after:
            target = staging / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(candidate / relative, target)
        control(git + ["add", "--force", "--all"], env)
        return control(git + ["diff", "--cached", "--no-ext-diff", "--no-textconv", "--binary", "HEAD"], env)


def identity(root):
    root = Path(root).resolve(strict=True)
    records = []
    for relative in files(root):
        path = root / relative
        with path.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        records.append({"path": relative.as_posix(), "sha256": digest,
                        "executable": bool(path.stat().st_mode & 0o111)})
    return hashlib.sha256(json.dumps(records, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
