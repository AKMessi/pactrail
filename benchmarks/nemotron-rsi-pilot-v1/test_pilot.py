import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("nemotron_pilot", Path(__file__).with_name("pilot.py"))
pilot = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pilot)


class PilotTests(unittest.TestCase):
    def catalog(self):
        return {"id": pilot.MODEL, "pricing": {"prompt": "0", "completion": "0"},
                "supported_parameters": ["tools", "reasoning", "temperature", "max_tokens"]}

    def test_exact_free_catalog_only(self):
        pilot.check_catalog(self.catalog())
        for change in ({"id": pilot.MODEL.removesuffix(":free")},
                       {"pricing": {"prompt": "0", "completion": "0.00001"}},
                       {"pricing": {"prompt": "0", "completion": "0", "request": "1"}},
                       {"supported_parameters": []}):
            with self.assertRaises(pilot.Refusal): pilot.check_catalog({**self.catalog(), **change})

    def test_zero_is_finite_numeric_and_not_missing_or_boolean(self):
        for value in (None, True, False, "NaN", "Infinity", -1, 1): self.assertFalse(pilot.zero(value))
        for value in (0, "0", "0.0"): self.assertTrue(pilot.zero(value))

    def test_wire_preserves_conversation_and_clamps_routing(self):
        original = {"model": pilot.MODEL, "messages": [{"role": "user", "content": "task"}],
                    "tools": [{"type": "function", "function": {"name": "read_file"}}]}
        wire = pilot.wire_request(original)
        self.assertEqual({k: v for k, v in wire.items() if k != "provider"}, original)
        self.assertNotIn("provider", original)
        self.assertEqual(wire["provider"], {"allow_fallbacks": False, "max_price": {"prompt": 0, "completion": 0}})

    def test_key_parsing_never_evaluates_shell(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / ".env"
            path.write_text('OPENROUTER_API_KEY="$(touch /tmp/not-executed)"\n')
            self.assertEqual(pilot.credential(path), "$(touch /tmp/not-executed)")
            path.write_text('OPENROUTER_API_KEY=one\nOPENROUTER_API_KEY=two\n')
            with self.assertRaises(pilot.Refusal): pilot.credential(path)

    def test_any_gateway_rejection_blocks_later_network(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "campaign"
            with pilot.Store(root, create=True): pass
            gateway = pilot.FreeGateway(root, {}, {}, "fixture-secret")
            with self.assertRaises(pilot.Refusal): gateway.submit("unknown", {})
            with pilot.Store(root) as store: self.assertTrue(store.value("free-provider-blocked"))

    def test_worker_retains_rate_rejection_without_retry(self):
        frames = []
        class Sender:
            def send_bytes(self, value): frames.append(value)
            def close(self): pass
        with patch.object(pilot, "request", return_value=({"status": 429, "headers": {}}, b'{"error":"rate limited"}')):
            pilot.provider_worker("secret", b"{}", Sender())
        self.assertEqual(len(frames), 2)
        self.assertIn(b"429", frames[0])
        self.assertIn(b"rate limited", frames[1])

    def test_request_bounds_redacts_and_allowlists_headers(self):
        class Response:
            code = 200
            headers = {"x-ratelimit-limit": "20", "authorization": "secret"}
            def read(self, size):
                self.asserted_size = size
                return b"secret"
            def __enter__(self): return self
            def __exit__(self, *_): pass
        class Opener:
            def open(self, *_args, **_kwargs): return Response()
        with patch.object(pilot.urllib.request, "build_opener", return_value=Opener()):
            meta, body = pilot.request("secret", "models")
        self.assertEqual(body, b"[REDACTED]")
        self.assertEqual(meta["headers"], {"x-ratelimit-limit": "20"})
        with patch.object(Response, "read", return_value=b"x" * (pilot.MAX_BODY + 1)), \
                patch.object(pilot.urllib.request, "build_opener", return_value=Opener()):
            with self.assertRaises(pilot.Refusal): pilot.request("secret", "models")
