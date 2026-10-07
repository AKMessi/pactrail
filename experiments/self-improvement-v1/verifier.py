"""Frozen parent verifier runs after the candidate process group is gone."""
import json
from pathlib import Path

from pactrail_lab.process import command
from pactrail_lab.safe import decode, read, canonical, Refusal, relative


def verify(root):
    root = Path(root)
    traces = list((root / "state" / "runs").glob("*/trace.jsonl"))
    if len(traces) != 1:
        raise Refusal("expected exactly one candidate run")
    run_id = traces[0].parent.name
    # IDs come from the filesystem, never from candidate-supplied argv.
    import re
    if not re.fullmatch(r"[a-f0-9-]{36}", run_id): raise Refusal("invalid run identity")
    base = ["/parent", "--workspace", "/source", "--state-dir", "/work/state"]
    checks = {"source_isolation_valid": True, "trace_valid": False, "receipt_valid": False, "ready_to_apply": False}
    for name in ("trace", "inspect", "diff"):
        result = command(base + [name, run_id, "--json"], timeout=30, limit=8_388_608)
        (root / ("parent-" + name + ".stdout")).write_bytes(result["stdout"])
        (root / ("parent-" + name + ".stderr")).write_bytes(result["stderr"])
        okay = result["exit_code"] == 0 and result["reason"] is None
        if name == "trace": checks["trace_valid"] = okay
        elif name == "inspect":
            checks["receipt_valid"] = okay
            if okay:
                receipt = decode(result["stdout"], 8_388_608)
                checks["ready_to_apply"] = receipt.get("outcome") == "ready_to_apply"
        elif not okay: checks["receipt_valid"] = False
    candidate = traces[0].parent / "workspace"
    relative(candidate.relative_to(root).as_posix())
    return {"run_id": run_id, "checks": checks, "candidate": candidate.relative_to(root).as_posix()}
