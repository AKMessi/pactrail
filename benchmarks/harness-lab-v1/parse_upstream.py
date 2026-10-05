#!/usr/bin/env python3
"""Invoke an externally installed, commit-pinned official SWE-bench parser.

Install this development-only dependency in a separate environment. No upstream
implementation is vendored into Pactrail and no parser input becomes executable.
"""
import argparse
import hashlib
import importlib.metadata
import json
from pathlib import Path
import re

UPSTREAM_COMMIT = "02e7a74ffd0b707aab73d203fe87bdc7c76afc8e"
MAX_LOG_BYTES = 8 * 1024 * 1024


def test_section(text):
    start, end = ">>>>> Start Test Output", ">>>>> End Test Output"
    if text.count(start) != 1 or text.count(end) != 1:
        raise ValueError("one complete test-output section required")
    if text.index(start) >= text.index(end):
        raise ValueError("test-output marker order is invalid")
    return text.split(start, 1)[1].split(end, 1)[0]


def parse(grader_path, log_path, instance_id):
    distribution = importlib.metadata.distribution("swebench")
    origin = json.loads(distribution.read_text("direct_url.json") or "{}")
    if origin.get("vcs_info", {}).get("commit_id") != UPSTREAM_COMMIT:
        raise ValueError("official parser installation must match the declared upstream commit")
    from swebench.harness import log_parsers
    from swebench.types import TestSpec
    with Path(grader_path).open("rb") as stream:
        raw = stream.read(16 * 1024 * 1024 + 1)
    if len(raw) > 16 * 1024 * 1024:
        raise ValueError("grader exceeds bound")
    grader = json.loads(raw)
    name = grader["log_parser"]
    if not isinstance(name, str) or not re.fullmatch(r"parse_log_[a-z0-9_]+", name):
        raise ValueError("invalid parser identifier")
    parser = getattr(log_parsers, name, None)
    if not callable(parser):
        raise ValueError("declared upstream parser unavailable: " + name)
    with Path(log_path).open("rb") as stream:
        raw_log = stream.read(MAX_LOG_BYTES + 1)
    if len(raw_log) > MAX_LOG_BYTES:
        raise ValueError("test log exceeds bound")
    text = raw_log.decode("utf-8", errors="replace")
    # Bash tracing prints these markers. Parsing setup/git output can accidentally
    # turn source text into alleged test results, so use the test section only.
    section = test_section(text)
    spec = TestSpec(instance_id=instance_id, image="", eval_script_list=[], repo="",
                    version="", FAIL_TO_PASS=grader["FAIL_TO_PASS"],
                    PASS_TO_PASS=grader["PASS_TO_PASS"], log_parser=name)
    statuses = parser(section, spec)
    sources = {}
    package = Path(distribution.locate_file("swebench"))
    for path in sorted(package.rglob("*.py")):
        sources[path.relative_to(package).as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
    return {"upstream_commit": UPSTREAM_COMMIT, "parser": name,
            "log_sha256": hashlib.sha256(raw_log).hexdigest(),
            "parser_source_hashes": sources, "statuses": statuses}


if __name__ == "__main__":
    command = argparse.ArgumentParser(description=__doc__)
    command.add_argument("grader", type=Path)
    command.add_argument("log", type=Path)
    command.add_argument("instance_id")
    args = command.parse_args()
    print(json.dumps(parse(args.grader, args.log, args.instance_id), allow_nan=False))
