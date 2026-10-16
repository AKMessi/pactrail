import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("nemotron_analysis", Path(__file__).with_name("analyze.py"))
analysis = importlib.util.module_from_spec(spec)
spec.loader.exec_module(analysis)


class AnalysisTests(unittest.TestCase):
    def test_missing_grading_is_unknown_not_a_failed_patch(self):
        self.assertIsNone(analysis.functional({}))
        self.assertIsNone(analysis.functional({"targeted": {"behavior_passed": True}}))

    def test_both_grades_and_coverage_required(self):
        passed = {"coverage_complete": True, "behavior_passed": True}
        self.assertTrue(analysis.functional({"targeted": passed, "regression": passed}))
        self.assertFalse(analysis.functional({"targeted": passed, "regression": {**passed, "coverage_complete": False}}))

    def test_functional_success_is_distinct_from_workflow_success(self):
        passed = {"coverage_complete": True, "behavior_passed": True}
        row = {"status": "harness-incomplete", "strict_success": False, "targeted": passed, "regression": passed}
        self.assertEqual(analysis.category(row, False, ""), "functional-pass-incomplete-workflow")
        self.assertEqual(analysis.category(row, True, ""), "provider-or-transport")

    def test_unattempted_and_budget_are_explicit(self):
        self.assertEqual(analysis.category({"status": "not-attempted"}, False, ""), "not-attempted-after-stop")
        self.assertEqual(analysis.category({"status": "failed", "strict_success": False}, False, "output tokens exhausted"), "budget-or-output-exhaustion")

    def test_completed_export_is_journal_bound(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            protocol = {"commit": "fixture", "model": {"id": "fixture"}, "order": [["task", 0]],
                        "tasks": [{"id": "task"}], "account_before": {}}
            row = {"task": "task", "repeat": 0, "status": "not-attempted", "strict_success": False}
            encoded = analysis.canonical(protocol)
            (root / "protocol.json").write_bytes(encoded)
            (root / "protocol.sha256").write_text(analysis.digest(encoded))
            (root / "results.json").write_text(json.dumps([row]))
            (root / "usage.json").write_text("[]")
            (root / "account-after.json").write_text("{}")
            with analysis.Store(root / "campaign", create=True) as store, store.transaction():
                store.append("pilot-protocol-frozen", {"artifact": store.put(encoded)})
                store.append("pilot-trial-result", {"artifact": store.record(row)})
            result = analysis.analyze(root)
            self.assertEqual(result["attempted_trials"], 0)
            self.assertIsNone(result["tokens"]["input_tokens"]["reported_sum"])
            (root / "results.json").write_text(json.dumps([{**row, "strict_success": True}]))
            with self.assertRaisesRegex(ValueError, "results export differs"):
                analysis.analyze(root)
            (root / "protocol.json").write_bytes(b"{}")
            with self.assertRaisesRegex(ValueError, "protocol hash mismatch"):
                analysis.analyze(root)
