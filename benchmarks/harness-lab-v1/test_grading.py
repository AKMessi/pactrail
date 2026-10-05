import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("grading", Path(__file__).with_name("grading.py"))
grading = importlib.util.module_from_spec(spec)
spec.loader.exec_module(grading)


class BehavioralGradingTests(unittest.TestCase):
    def test_only_declared_executed_results_establish_behavior(self):
        result = grading.summarize(["target"], {"target": "FAILED", "other": "PASSED"})
        self.assertTrue(result["behavior_failed"])
        self.assertFalse(result["behavior_passed"])
        self.assertEqual(result["tests_executed"], 1)
        self.assertTrue(grading.summarize(["target"], {"target": "PASSED"})["behavior_passed"])

    def test_missing_skipped_errors_timeout_and_infrastructure_never_qualify(self):
        cases = [({}, {}), ({"target": "SKIPPED"}, {}), ({"target": "ERROR"}, {}),
                 ({"target": "PASSED"}, {"timed_out": True}),
                 ({"target": "FAILED"}, {"infrastructure_error": True})]
        for statuses, flags in cases:
            with self.subTest(statuses=statuses, flags=flags):
                result = grading.summarize(["target"], statuses, **flags)
                self.assertFalse(result["coverage_complete"])
                self.assertFalse(result["behavior_passed"])
                self.assertFalse(result["behavior_failed"])

    def test_partial_execution_cannot_validate_base_or_gold(self):
        result = grading.summarize(["a", "b"], {"a": "FAILED"})
        self.assertEqual(result["tests_executed"], 1)
        self.assertEqual(result["missing"], ["b"])
        self.assertFalse(result["behavior_failed"])
        self.assertFalse(result["behavior_passed"])

    def test_malformed_or_empty_results_are_rejected(self):
        for required in ([], ["x", "x"], [None], "x", ["x"] * (grading.MAX_TESTS + 1)):
            with self.assertRaises(ValueError):
                grading.summarize(required, {})
        for statuses in ([], {"x": 0}, {"x": "ok"}, {None: "PASSED"}):
            with self.assertRaises(ValueError):
                grading.summarize(["x"], statuses)
        with self.assertRaises(ValueError):
            grading.summarize(["x"], {}, timed_out=1)


if __name__ == "__main__":
    unittest.main()
