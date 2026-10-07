#!/usr/bin/env python3
"""Pactrail self-improvement supervisor. No paid trial runs implicitly."""
import argparse
import json
from pathlib import Path
import sys
import sqlite3
import uuid

from pactrail_lab.campaign import Campaign, initialize, status, fork_campaign
from pactrail_lab.evidence import audit_export, export
from pactrail_lab.safe import Refusal, digest, canonical, identifier
from pactrail_lab.store import Store


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    init = sub.add_parser("init", help="freeze source, verifier, protocols and conservative model budgets")
    init.add_argument("manifest", type=Path)
    init.add_argument("--campaign", type=Path, required=True)
    fork = sub.add_parser("fork", help="continue from an accepted export with fresh confirmation tasks")
    fork.add_argument("manifest", type=Path)
    fork.add_argument("--campaign", type=Path, required=True)
    fork.add_argument("--parent-campaign", type=Path, required=True)
    check = sub.add_parser("verify-export")
    check.add_argument("output", type=Path)
    for name in ("status", "qualify", "baseline", "cycle", "propose", "implement", "evaluate", "approve", "undo", "recover", "review", "export"):
        command = sub.add_parser(name)
        command.add_argument("--campaign", type=Path, required=True)
        if name not in ("status", "review", "export"):
            command.add_argument("--command-id", default="command-" + uuid.uuid4().hex)
            command.add_argument("--expected-head", required=True, help="exact journal head from status; prevents stale commands")
        if name == "implement": command.add_argument("--proposal", required=True)
        if name == "evaluate": command.add_argument("--candidate", required=True)
        if name == "approve":
            command.add_argument("--verdict", required=True)
            command.add_argument("--acknowledge-human-review", action="store_true")
        if name == "undo": command.add_argument("--revision", required=True)
        if name == "export": command.add_argument("--output", type=Path, required=True)
        if name == "review": command.add_argument("--port", type=int, default=3091)
    args = parser.parse_args()
    if args.command == "init": result = initialize(args.campaign, args.manifest)
    elif args.command == "fork": result = fork_campaign(args.parent_campaign, args.campaign, args.manifest)
    elif args.command == "verify-export": result = audit_export(args.output)
    else:
        with Store(args.campaign) as store:
            if args.command == "status": result = status(store)
            elif args.command == "export": result = export(store, args.output)
            elif args.command == "review":
                from pactrail_lab.review import serve
                serve(store.root, args.port)
                return
            else:
                campaign = Campaign(store)
                if args.command == "cycle":
                    identifier(args.command_id)
                    def step(name): return "cycle-" + digest(canonical([args.command_id, name]))[:24]
                    proposed = campaign.propose(step("propose"), args.expected_head)
                    implemented = campaign.implement(step("implement"), store.head(), proposed["proposal"])
                    result = campaign.evaluate(step("evaluate"), store.head(), implemented["candidate"])
                    print(json.dumps(result, indent=2, allow_nan=False))
                    return
                extra = {"implement": [getattr(args, "proposal", None)], "evaluate": [getattr(args, "candidate", None)],
                         "approve": [getattr(args, "verdict", None), getattr(args, "acknowledge_human_review", False)],
                         "undo": [getattr(args, "revision", None)]}.get(args.command, [])
                result = getattr(campaign, args.command)(args.command_id, args.expected_head, *extra)
    print(json.dumps(result, indent=2, allow_nan=False))


if __name__ == "__main__":
    try: main()
    except sqlite3.Error:
        print("Lab storage is unavailable; no uncertain operation will be replayed. Check disk/quota and journal integrity, then recover before continuing.", file=sys.stderr)
        sys.exit(2)
    except (Refusal, OSError, KeyError, TypeError, ValueError) as error:
        print("Lab refused operation: " + str(error), file=sys.stderr)
        sys.exit(2)
