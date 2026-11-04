import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("crouton_analysis", Path(__file__).with_name("analyze.py"))
analysis = importlib.util.module_from_spec(spec)
spec.loader.exec_module(analysis)


class AnalysisTests(unittest.TestCase):
    def test_unknown_values_are_not_measured_zero(self):
        self.assertIsNone(analysis.coverage([None], 1)["reported_sum"])
        self.assertEqual(analysis.coverage([0], 1)["reported_sum"], 0)
        self.assertEqual(analysis.coverage([0, None], 2)["coverage"], "partial")

    def test_failure_types_preserve_distinct_mechanisms(self):
        cases = [("model reached its 8192-token output limit", "output-exhaustion"),
                 ("model used 330000 tokens, exceeding the budget", "total-token-exhaustion"),
                 ("model did not complete within 16 turns", "turn-exhaustion"),
                 ("stalled because every tool request failed", "tool-interface-stall")]
        for text, expected in cases:
            self.assertEqual(analysis.observable_failure(text, [], False), expected)
        self.assertEqual(analysis.observable_failure("", [], True), "success")
