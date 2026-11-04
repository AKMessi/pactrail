import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("crouton_analysis", Path(__file__).with_name("analyze.py"))
analysis = importlib.util.module_from_spec(spec)
spec.loader.exec_module(analysis)


class AnalysisTests(unittest.TestCase):
    def test_projection_audit_rejects_unrelated_grader_tree_changes(self):
        from pactrail_lab.snapshots import import_tree, materialize, project_verified_changes
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            base, raw = root / "base", root / "sparse"
            base.mkdir(); raw.mkdir()
            (base / "package-lock.json").write_text("keep")
            (base / "value").write_text("before")
            (raw / "value").write_text("after")
            receipt = {"changes": [{"path": "value", "before_digest": "before", "after_digest": "after"}]}
            with analysis.Store(root / "campaign", create=True) as store, store.transaction():
                baseline, sparse = import_tree(store, base), import_tree(store, raw)
                projected = project_verified_changes(store, baseline, sparse, receipt["changes"])
                materialize(store, projected, root / "trials/task-r1/candidate")
                protocol = {"commit": "fixture", "model": {"id": "fixture"}, "order": [["task", 0]],
                            "tasks": [{"id": "task", "source": baseline}], "account_before": {},
                            "frozen_inputs": {}, "validations": {}, "candidate_grading": "fixture"}
                row = {"task": "task", "repeat": 0, "status": "completed", "strict_success": False,
                       "checks": {"receipt_valid": True}, "candidate_source": projected}
                store.append("candidate-projected", {"baseline": baseline, "raw_candidate": sparse,
                    "projected": projected, "receipt": store.put(analysis.canonical(receipt))})
                store.append("pilot-protocol-frozen", {"artifact": store.put(analysis.canonical(protocol))})
                store.append("pilot-trial-result", {"artifact": store.record(row)})
                store.append("pilot-account-after", {})
                (root / "events.json").write_bytes(analysis.canonical(store.events()))
            (root / "protocol.json").write_bytes(analysis.canonical(protocol))
            (root / "protocol.sha256").write_text(analysis.digest(analysis.canonical(protocol)))
            (root / "results.json").write_text(json.dumps([row]))
            (root / "usage.json").write_text("[]")
            (root / "account-after.json").write_text("{}")
            analysis.analyze(root)
            (root / "trials/task-r1/candidate/package-lock.json").unlink()
            with self.assertRaisesRegex(ValueError, "graded candidate differs"):
                analysis.analyze(root)

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

    def test_full_audit_retains_request_identity_and_measured_zero(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            protocol = {"commit": "fixture", "model": {"id": "fixture"}, "order": [["task", 0]],
                        "tasks": [{"id": "task"}], "account_before": {}, "frozen_inputs": {}, "validations": {}}
            row = {"task": "task", "repeat": 0, "status": "harness-incomplete", "strict_success": False}
            response = {"model": "fixture", "provider": "fixture", "usage": {"cost": 0},
                        "choices": [{"finish_reason": "tool_calls", "message": {"tool_calls": [
                            {"function": {"name": "read_file", "arguments": '{"path":"a"}'}},
                            {"function": {"name": "read_file", "arguments": '{ "path": "a" }'}}]}}]}
            reservation = {"request_id": "request-fixture", "lease": "task-r1", "state": "settled", "usage": None}
            with analysis.Store(root / "campaign", create=True) as store, store.transaction():
                store.set("model-reservation-request-fixture", reservation)
                store.append("model-request", {"id": "request-fixture"})
                store.append("free-provider-wire-request", {"artifact": store.put(b"{}")})
                store.append("free-provider-raw-response", {"status": 200, "wall_time_ms": 1,
                                                            "artifact": store.put(analysis.canonical(response))})
                store.append("pilot-protocol-frozen", {"artifact": store.put(analysis.canonical(protocol))})
                store.append("pilot-trial-result", {"artifact": store.record(row)})
                store.append("pilot-account-after", {})
                (root / "events.json").write_bytes(analysis.canonical(store.events()))
            (root / "protocol.json").write_bytes(analysis.canonical(protocol))
            (root / "protocol.sha256").write_text(analysis.digest(analysis.canonical(protocol)))
            (root / "results.json").write_text(json.dumps([row]))
            (root / "usage.json").write_text(json.dumps([reservation]))
            (root / "account-after.json").write_text("{}")
            report = analysis.analyze(root)
            self.assertEqual(report["mechanisms"]["reported_cost"]["reported_sum"], 0)
            self.assertEqual(report["mechanisms"]["repeated_exact_tool_requests_within_trial"], 1)
            self.assertIsNone(report["mechanisms"]["reasoning_tokens"]["reported_sum"])
            # Real rate-limit responses omit provider/model/usage. Their absent
            # metadata must not crash analysis or become measured zero usage.
            with analysis.Store(root / "campaign") as store, store.transaction():
                store.append("free-provider-wire-request", {"artifact": store.put(b"{}")})
                store.append("free-provider-raw-response", {"status": 429, "wall_time_ms": 1,
                    "artifact": store.put(analysis.canonical({"error": {"code": 429, "message": "rate limited"}}))})
                (root / "events.json").write_bytes(analysis.canonical(store.events()))
            report = analysis.analyze(root)
            self.assertEqual(report["mechanisms"]["providers"]["not_reported"], 1)
            self.assertEqual(report["mechanisms"]["reported_cost"]["coverage"], "partial")
