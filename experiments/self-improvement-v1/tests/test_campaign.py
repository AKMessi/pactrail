from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from pactrail_lab.campaign import Campaign, status, fork_campaign, grade
from pactrail_lab.evidence import export, audit_export
from pactrail_lab.execution import supervisor_inputs, write_frozen
from pactrail_lab.safe import Refusal
from pactrail_lab.snapshots import import_tree, revision
from pactrail_lab.store import Store


class FixtureExecution:
    """Deterministic runtime fixture, never a model-quality benchmark."""
    def __init__(self, store, bad, good):
        self.store, self.bad, self.good = store, bad, good
        self.calls = 0

    def operation(self, source, binary, operation, image=None):
        passed = source == self.good if operation["argv"] == ["targeted"] else True
        return {"passed": passed, "record": self.store.record({"fixture": True}),
                "binary": self.store.put(b"improved") if operation["argv"] == ["build"] else None,
                "verification": dict(schema_version=1, executed=1, passed=int(passed), failed=int(not passed), errors=0, skipped=0),
                "execution": dict(exit_code=0 if passed else 1, reason=None)}

    def trial(self, revision_key, task_source, goal, limits, lease_name):
        self.calls += 1
        record = self.store.load(revision_key)
        implementation = goal.startswith("Implement")
        correct = implementation or self.store.get(record["binary"]) == b"improved"
        return {"status": "completed", "record": self.store.record({"fixture": True}),
                "checks": {name: True for name in ("source_isolation_valid", "trace_valid", "receipt_valid", "ready_to_apply")},
                "candidate_source": self.good if correct else self.bad}


class CampaignTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.store = Store(self.root / "campaign", create=True)
        for name in ("bad", "good"):
            (self.root / name).mkdir()
            (self.root / name / "lib.rs").write_text(name)
        self.bad, self.good = (import_tree(self.store, self.root / name) for name in ("bad", "good"))
        operation = lambda name: dict(argv=[name], inputs={"/outside/verifier.py": "0" * 64}, timeout_seconds=1)
        self.manifest = dict(kind="fixture", model={"id": "deterministic-fixture"}, image="sha256:" + "0" * 64,
            gates={name: operation(name) for name in ("engineering", "authority", "compatibility", "recovery", "mechanism")},
            build=operation("build"), limits=dict(cycles=1, repetitions=3, requests=1000, cost_microusd=1000000, wall_seconds=3600))
        baseline = revision(self.store, self.bad, b"parent", {"agent_mode": "single"}, ["original memory"])
        self.parent = baseline
        self.protocol = dict(schema_version=1, id="fixture", seed=7, repetitions=3,
            limits=dict(model_turns=2, wall_seconds=10, context_tokens=4096, output_tokens=256, model_tokens=100000),
            tasks=[dict(id="task-" + str(t), base_source=self.bad, gold_source=self.good,
                        goal="fix fixture", image=self.manifest["image"], targeted=operation("targeted"),
                        regression=operation("regression")) for t in range(8)])
        with self.store.transaction():
            inputs = supervisor_inputs()
            for path, key in inputs.items():
                self.assertEqual(self.store.put(Path(path).read_bytes()), key)
            self.store.set("supervisor-inputs", inputs)
            self.store.set("external-inputs", {"/outside/verifier.py": self.store.put(b"hidden grader")})
            self.store.set("manifest", self.store.record(self.manifest))
            self.store.set("baseline", baseline)
            self.store.set("active", baseline)
            self.store.set("lineage", {baseline: dict(parent=None, state="active")})
            self.store.set("deadline-ns", time.time_ns() + 3600 * 10**9)
            self.store.set("confirmation-spent", False)
            for name in ("development", "confirmation"):
                self.store.set(name + "-protocol", self.store.record(self.protocol))
        self.execution = FixtureExecution(self.store, self.bad, self.good)
        self.campaign = Campaign(self.store, self.execution)

    def tearDown(self):
        self.store.close()
        self.temp.cleanup()

    def proposal(self):
        proposal = dict(schema_version=1, id="fixture-proposal", parent=self.parent,
                        weaknesses=self.store.value("weaknesses")[:1], hypothesis="fixture improves", mechanism="fixture mechanism",
                        predicted_gain=.1, risks="fixture only", primary_metric="strict_completion")
        with self.store.transaction():
            key = self.store.record(proposal)
            self.store.set("proposals", [key])
        return key

    def candidate(self):
        self.campaign.qualify("qualify", self.store.head())
        self.campaign.baseline("baseline", self.store.head())
        proposal_key = self.proposal()
        return self.campaign.implement("implement", self.store.head(), proposal_key)["candidate"]

    def test_full_fixture_loop_explicit_approval_and_complete_undo(self):
        candidate = self.candidate()
        verdict = self.campaign.evaluate("evaluate", self.store.head(), candidate)
        self.assertEqual(verdict["result"]["verdict"], "supported")
        self.assertIn("fixture", verdict["result"]["claim"])
        self.assertEqual(self.store.value("active"), self.parent)
        self.campaign.approve("approve", self.store.head(), verdict["verdict"], True)
        self.assertEqual(self.store.value("active"), candidate)
        self.assertEqual(self.campaign.approve("approve", "0" * 64, verdict["verdict"], True)["active"], candidate)
        result = self.campaign.undo("undo", self.store.head(), candidate)
        self.assertEqual(result["active"], self.parent)
        self.assertEqual(self.store.load(result["active"])["memory"], ["original memory"])
        self.assertEqual(self.store.value("lineage")[candidate]["state"], "retired")
        self.assertTrue(self.store.value("confirmation-spent"))
        output = self.root / "export"
        export(self.store, output)
        self.assertGreater(audit_export(output)["verified_files"], 10)
        (output / "active-pactrail").write_bytes(b"tampered")
        with self.assertRaises(Refusal): audit_export(output)

    def test_stale_approval_fails_and_failed_work_remains_recoverable(self):
        candidate = self.candidate()
        verdict = self.campaign.evaluate("evaluate", self.store.head(), candidate)
        with self.assertRaises(Refusal): self.campaign.approve("approve", "0" * 64, verdict["verdict"], True)
        self.assertEqual(self.store.value("active"), self.parent)
        with self.assertRaises(Refusal): self.campaign.approve("approve", self.store.head(), verdict["verdict"], False)
        self.assertIsNotNone(self.store.value("inflight"))
        # No containers exist in this fixture, so recovery performs no Docker I/O.
        self.campaign.recover("recover", self.store.head())
        self.assertIsNone(self.store.value("inflight"))
        self.assertEqual(self.store.value("active"), self.parent)

    def test_crash_after_reservation_never_repeats_trial_and_supervisor_drift_fails(self):
        self.campaign.qualify("qualify", self.store.head())
        with self.store.transaction():
            self.store.set("inflight", dict(id="crashed", state="dispatched", kind="baseline"))
        count = self.execution.calls
        with self.assertRaises(Refusal): self.campaign.baseline("new-command", self.store.head())
        self.assertEqual(self.execution.calls, count)
        self.campaign.recover("recover", self.store.head())
        with self.store.transaction(): self.store.set("supervisor-inputs", {"forged": "0" * 64})
        with self.assertRaises(Refusal): Campaign(self.store, self.execution)

    def test_model_container_never_receives_hidden_grader(self):
        output = self.root / "frozen"
        write_frozen(self.store, output, include_verifiers=False)
        self.assertFalse((output / "inputs").exists())
        self.assertTrue((output / "agent-runtime-v1/pactrail_cli.py").exists())

    def test_fixture_executor_refused_in_research_campaign(self):
        with self.store.transaction(): self.store.set("manifest", self.store.record({**self.manifest, "kind": "research"}))
        with self.assertRaises(Refusal): Campaign(self.store, self.execution)

    def test_empty_skipped_setup_failure_and_exit_only_grading_are_refused(self):
        good = dict(schema_version=1, executed=1, passed=1, failed=0, errors=0, skipped=0)
        for report, code in ((None, 0), ({**good, "executed": 0, "passed": 0}, 0),
                             ({**good, "skipped": 1}, 0), ({**good, "errors": 1}, 0),
                             (good, 2), ({**good, "schema_version": True}, 0)):
            with self.assertRaises(Refusal):
                grade(dict(passed=True, verification=report, execution=dict(exit_code=code, reason=None)))

    def test_fresh_campaign_preserves_ancestral_undo_and_retires_descendants(self):
        candidate = self.candidate()
        verdict = self.campaign.evaluate("evaluate", self.store.head(), candidate)
        self.campaign.approve("approve", self.store.head(), verdict["verdict"], True)
        child_root = self.root / "next-campaign"
        def initialize_fixture(root, _manifest):
            with Store(root, create=True) as child, child.transaction():
                for key, body in self.store.db.execute("SELECT digest,body FROM objects"):
                    child.put(body)
                current = self.store.load(candidate)
                baseline = revision(child, current["source"], child.get(current["binary"]), current["configuration"], current["memory"])
                child.set("active", baseline)
                child.set("manifest", child.record(self.manifest))
                child.set("supervisor-inputs", self.store.value("supervisor-inputs"))
                child.set("deadline-ns", time.time_ns() + 3600 * 10**9)
                for name in ("development", "confirmation"):
                    child.set(name + "-protocol", child.record({"tasks": [{"base_source": "f" * 64}]}))
        with patch("pactrail_lab.campaign.initialize", initialize_fixture):
            fork_campaign(self.store.root, child_root, "fixture-manifest")
        with Store(child_root) as child:
            grandchild = revision(child, self.good, b"generation-two", {"agent_mode": "text"}, ["descendant memory"], candidate)
            with child.transaction():
                lineage = child.value("lineage")
                lineage[candidate]["state"] = "ancestor"
                lineage[grandchild] = {"parent": candidate, "state": "active"}
                child.set("lineage", lineage)
                child.set("active", grandchild)
            result = Campaign(child, FixtureExecution(child, self.bad, self.good)).undo("undo-ancestor", child.head(), candidate)
            self.assertEqual(result["active"], self.parent)
            self.assertEqual(set(result["retired"]), {candidate, grandchild})
            self.assertEqual(child.load(result["active"])["memory"], ["original memory"])


if __name__ == "__main__": unittest.main()
