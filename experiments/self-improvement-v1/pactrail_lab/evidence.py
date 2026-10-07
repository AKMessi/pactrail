"""Operator exports contain immutable artifacts and a SHA-256 inventory."""
from pathlib import Path

from .safe import canonical, read, digest, Refusal
from .snapshots import materialize


def export(store, output):
    store.verify()
    output = Path(output)
    output.mkdir(parents=True, exist_ok=False, mode=0o700)
    objects = output / "objects"
    objects.mkdir()
    for key, in store.db.execute("SELECT digest FROM objects ORDER BY digest"):
        (objects / key).write_bytes(store.get(key))
    (output / "events.json").write_bytes(canonical(store.events()))
    active = store.load(store.value("active"))
    materialize(store, active["source"], output / "active-source")
    (output / "active-pactrail").write_bytes(store.get(active["binary"]))
    (output / "active-pactrail").chmod(0o755)
    (output / "active-configuration.json").write_bytes(canonical(active["configuration"]))
    (output / "active-memory.json").write_bytes(canonical(active["memory"]))
    inventory = {p.relative_to(output).as_posix(): digest(read(p)) for p in sorted(output.rglob("*")) if p.is_file()}
    (output / "sha256.json").write_bytes(canonical({"schema_version": 1, "head": store.head(), "files": inventory}))
    return {"output": str(output), "head": store.head(), "inventory_sha256": digest(read(output / "sha256.json"))}


def audit_export(output):
    from .safe import decode, relative
    output = Path(output)
    inventory = decode(read(output / "sha256.json", 8_388_608))
    for name, expected in inventory["files"].items():
        if digest(read(output / str(relative(name)))) != expected:
            raise Refusal("export inventory mismatch")
    actual = {p.relative_to(output).as_posix() for p in output.rglob("*") if p.is_file() and p.name != "sha256.json"}
    if actual != set(inventory["files"]): raise Refusal("export has additional or missing files")
    return {"verified_files": len(actual), "head": inventory["head"]}
