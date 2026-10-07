#!/usr/bin/env python3
"""Finite seeded hostile-input smoke campaign; not a security proof."""
import argparse
import random
import tempfile
from pathlib import Path

from pactrail_lab.archive import unpack
from pactrail_lab.safe import decode, Refusal


def run(seed=20261007, iterations=1000):
    rng = random.Random(seed)
    refusals = 0
    for i in range(iterations):
        data = bytes(rng.randrange(256) for _ in range(rng.randrange(0, 2048)))
        try: decode(data)
        except Refusal: refusals += 1
        with tempfile.TemporaryDirectory(prefix="pactrail-fuzz-") as root:
            try: unpack(data, Path(root) / "archive")
            except Refusal: refusals += 1
    print({"seed": seed, "iterations": iterations, "refusals": refusals,
           "scope": "bounded JSON/archive smoke; no formal security claim"})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seed", type=int, default=20261007)
    parser.add_argument("--iterations", type=int, default=1000)
    args = parser.parse_args()
    if not 1 <= args.iterations <= 100000: parser.error("iterations must be 1–100000")
    run(args.seed, args.iterations)
