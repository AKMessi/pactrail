import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("groq_analysis", Path(__file__).with_name("analyze.py"))
analysis = importlib.util.module_from_spec(spec)
spec.loader.exec_module(analysis)


class AnalysisTests(unittest.TestCase):
    def test_missing_usage_is_absent_not_zero(self):
        self.assertEqual(analysis.aggregate([{"usage": None}], "input_tokens"),
                         {"reported_sum": None, "reported_requests": 0, "total_requests": 1, "coverage": "absent"})

    def test_partial_usage_keeps_coverage_and_real_zero(self):
        rows = [{"usage": {"cached_input_tokens": 0}}, {"usage": None}]
        self.assertEqual(analysis.aggregate(rows, "cached_input_tokens"),
                         {"reported_sum": 0, "reported_requests": 1, "total_requests": 2, "coverage": "partial"})

    def test_complete_measured_sum(self):
        rows = [{"usage": {"output_tokens": 3}}, {"usage": {"output_tokens": 4}}]
        self.assertEqual(analysis.aggregate(rows, "output_tokens")["reported_sum"], 7)
        self.assertEqual(analysis.aggregate(rows, "output_tokens")["coverage"], "complete")
