import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from pactrail_lab.gateway import Gateway, validate_request
from pactrail_lab.process import command
from pactrail_lab.safe import Refusal, canonical
from pactrail_lab.store import Store

MODEL = dict(id="frozen/model", endpoint="https://example.invalid/v1/chat/completions",
             api_key_env="LAB_KEY", context_tokens=4096, output_tokens=256,
             input_rate=1000000, output_rate=2000000, temperature=0, reasoning_effort=None)
LIMITS = dict(requests=4, cost_microusd=30000, wall_seconds=30)
BODY = dict(model="frozen/model", messages=[dict(role="user", content="Fix the parser")], max_tokens=256)


class GatewayTests(unittest.TestCase):
    def test_physical_calls_are_charged_before_io_and_unknown_usage_stays_unknown(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "campaign"
            Store(root, create=True).close()
            key = "secret-not-retained-1234"
            calls = []
            def transport(body):
                with Store(root) as store:
                    self.assertEqual(len(store.value("dispatched")), len(calls) + 1)
                calls.append(body)
                return canonical(dict(model=MODEL["id"], choices=[dict(message=dict(content=key))]))
            gateway = Gateway(root, MODEL, LIMITS, key, transport)
            token = gateway.lease("trial-a", 1, 30)
            response = gateway.submit(token, BODY)
            self.assertNotIn(key.encode(), response)
            self.assertEqual(calls[0]["temperature"], 0)
            with self.assertRaises(Refusal): gateway.submit(token, BODY)
            with Store(root) as store:
                row = next(iter(store.value("reservations").values()))
                self.assertIsNone(row["usage"])
                self.assertEqual(row["reserved"], 4608)
                for value, in store.db.execute("SELECT body FROM objects"):
                    self.assertNotIn(key.encode(), value)

    def test_failure_never_refunds_or_substitutes_model(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "campaign"
            Store(root, create=True).close()
            gateway = Gateway(root, MODEL, LIMITS, "secret", lambda _: canonical(dict(model="wrong", choices=[{}])))
            token = gateway.lease("trial-b", 1, 30)
            with self.assertRaises(Refusal): gateway.submit(token, BODY)
            with self.assertRaises(Refusal): gateway.submit(token, BODY)
            with Store(root) as store:
                self.assertEqual(len(store.value("reservations")), 1)
                self.assertEqual(next(iter(store.value("reservations").values()))["state"], "reserved")
            gateway.revoke(token)
            with self.assertRaises(Refusal): gateway.submit(token, BODY)

    def test_protocol_escalation_and_unbounded_input_are_refused(self):
        for body in ({**BODY, "model": "other"}, {**BODY, "stream": True},
                     {**BODY, "temperature": 1}, {**BODY, "reasoning_effort": "high"},
                     {**BODY, "max_tokens": 257}, {**BODY, "messages": [{"role": "user", "content": [{}]}]},
                     {**BODY, "messages": [{"role": "assistant", "content": "x", "audio": {"id": "hidden-billing"}}]},
                     {**BODY, "messages": [{"role": "user", "content": "x" * 4096}]},
                     {**BODY, "unknown": True}):
            with self.subTest(body=body), self.assertRaises(Refusal): validate_request(body, MODEL)

    def test_bounded_process_timeout_and_output(self):
        result = command([sys.executable, "-c", "import time; time.sleep(5)"], timeout=0.1)
        self.assertEqual(result["reason"], "timeout")
        result = command([sys.executable, "-c", "print('x' * 10000)"], limit=100)
        self.assertEqual(result["reason"], "output-limit")
        self.assertLessEqual(len(result["stdout"]), 100)

    def test_forged_command_cache_rejected(self):
        with tempfile.TemporaryDirectory() as tmp, Store(Path(tmp) / "campaign", create=True) as store:
            store.mutate("operation", store.head(), {}, lambda: {"done": True})
            store.db.execute("UPDATE commands SET request=?", ("0" * 64,))
            with self.assertRaises(Refusal): store.mutate("operation", store.head(), {}, lambda: {})


if __name__ == "__main__": unittest.main()
