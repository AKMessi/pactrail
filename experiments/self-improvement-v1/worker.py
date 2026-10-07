#!/usr/bin/env python3
"""Frozen OCI worker. All writable bytes reside in a size-limited tmpfs."""
import os
from pathlib import Path
import shutil
import sys

from pactrail_lab.archive import pack
from pactrail_lab.process import command
from pactrail_lab.safe import canonical, decode, read, Refusal


def main():
    request = decode(read("/request.json"))
    work = Path("/work")
    # Copy source because builds/checks may need scratch files. The read-only
    # host mount cannot be mutated even by a changed policy implementation.
    shutil.copytree("/source", work / "source")
    replacements = {"{source}": "/work/source", "{candidate}": "/work/source",
                    "{binary}": "/harness", "{output}": "/work/pactrail"}
    argv = [replacements.get(arg, arg) for arg in request["argv"]]
    result = command(argv, cwd="/work/source", timeout=request["timeout_seconds"], limit=8_388_608)
    (work / "execution.json").write_bytes(canonical({key: value for key, value in result.items() if key not in ("stdout", "stderr")}))
    (work / "stdout.txt").write_bytes(result["stdout"])
    (work / "stderr.txt").write_bytes(result["stderr"])
    # Never return build scratch or arbitrary source caches, only declared output.
    shutil.rmtree(work / "source")
    sys.stdout.buffer.write(pack(work))
    return 0


if __name__ == "__main__":
    try: sys.exit(main())
    except (Refusal, OSError, ValueError): sys.exit(65)
