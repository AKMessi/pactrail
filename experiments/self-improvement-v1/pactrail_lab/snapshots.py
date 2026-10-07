"""Source/configuration/memory revisions are immutable, never host-tree rewinds."""
import os
from pathlib import Path
import subprocess

from .safe import MAX_FILES, MAX_TREE, Refusal, canonical, clean_environment, digest, read, relative, tree

EXCLUDED = (".git", ".pactrail", "benchmark-results", "benchmarks", "experiments")


def import_tree(store, root):
    entries = tree(root, EXCLUDED)
    for item in entries:
        if item["kind"] == "file":
            if Path(item["path"]).name in (".env", "credentials") or Path(item["path"]).suffix in (".pem", ".key"):
                raise Refusal("secret-like source file is not exportable")
            if store.put(read(Path(root) / item["path"])) != item["digest"]:
                raise Refusal("source changed while snapshotting")
    if tree(root, EXCLUDED) != entries:
        raise Refusal("source changed while snapshotting")
    return store.record({"schema_version": 1, "entries": entries})


def import_git(store, root, commit):
    env = clean_environment()
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull, GIT_TERMINAL_PROMPT="0")
    command = ["git", "-c", "core.hooksPath=" + os.devnull, "-C", str(root)]
    actual = subprocess.check_output(command + ["rev-parse", "--verify", commit + "^{commit}"], env=env, timeout=30).decode().strip()
    if actual != commit:
        raise Refusal("source commit identity mismatch")
    listing = subprocess.check_output(command + ["ls-tree", "-rlz", commit], env=env, timeout=30)
    if len(listing) > 16 * 1024 * 1024:
        raise Refusal("Git source manifest exceeds bound")
    entries, size = [], 0
    for row in listing.split(b"\0"):
        if not row:
            continue
        header, raw_path = row.split(b"\t", 1)
        mode, kind, object_id, length = header.split()
        name = raw_path.decode("utf-8")
        relative(name)
        if any(name == p or name.startswith(p + "/") for p in EXCLUDED):
            continue
        if mode not in (b"100644", b"100755") or kind != b"blob":
            raise Refusal("Git source contains nonregular entries")
        if Path(name).name in (".env", "credentials") or Path(name).suffix in (".key", ".pem"):
            raise Refusal("secret-like source file is not exportable")
        n = int(length)
        size += n
        if n > 64 * 1024 * 1024 or size > MAX_TREE or len(entries) >= MAX_FILES:
            raise Refusal("Git source exceeds bounds")
        data = subprocess.check_output(command + ["cat-file", "blob", object_id.decode()], env=env, timeout=30)
        if len(data) != n:
            raise Refusal("Git source size mismatch")
        entries.append({"path": name, "kind": "file", "digest": store.put(data), "bytes": n,
                        "executable": mode == b"100755"})
    # Normalize parent directories so physical and Git imports share one shape.
    directories = {p.as_posix() for e in entries for p in Path(e["path"]).parents if p.as_posix() != "."}
    entries.extend({"path": p, "kind": "directory"} for p in directories)
    return store.record({"schema_version": 1, "entries": sorted(entries, key=lambda e: e["path"])})


def materialize(store, key, root):
    root = Path(root)
    root.mkdir(mode=0o700, parents=True, exist_ok=False)
    snapshot = store.load(key)
    total = 0
    if snapshot.get("schema_version") != 1 or len(snapshot.get("entries", [])) > MAX_FILES:
        raise Refusal("invalid source snapshot")
    seen = set()
    for item in snapshot["entries"]:
        name = str(relative(item["path"]))
        if name in seen:
            raise Refusal("duplicate snapshot path")
        seen.add(name)
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        if item["kind"] == "directory":
            path.mkdir(exist_ok=True)
        elif item["kind"] == "file":
            data = store.get(item["digest"])
            total += len(data)
            if len(data) != item["bytes"] or total > MAX_TREE:
                raise Refusal("snapshot length mismatch")
            path.write_bytes(data)
            path.chmod(0o755 if item["executable"] else 0o644)
        else:
            raise Refusal("unknown snapshot entry kind")
    if digest(canonical(tree(root))) != digest(canonical(snapshot["entries"])):
        raise Refusal("materialized snapshot identity mismatch")


def revision(store, source, binary, configuration, memory, parent=None, proposal=None):
    return store.record({"schema_version": 1, "source": source, "binary": store.put(binary),
                         "configuration": configuration, "memory": memory,
                         "parent": parent, "proposal": proposal})


def diff(store, before, after):
    a = {e["path"]: e for e in store.load(before)["entries"]}
    b = {e["path"]: e for e in store.load(after)["entries"]}
    return [{"path": p, "before": a.get(p), "after": b.get(p)} for p in sorted(a.keys() | b.keys()) if a.get(p) != b.get(p)]

