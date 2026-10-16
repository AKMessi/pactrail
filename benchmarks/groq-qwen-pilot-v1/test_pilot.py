import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("groq_pilot", Path(__file__).with_name("pilot.py"))
pilot = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pilot)


class PilotTests(unittest.TestCase):
    def test_credential_is_literal_and_duplicates_refused(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / ".env"
            path.write_text('export GROQ_API_KEY="$(touch /tmp/never-execute)"\n')
            self.assertEqual(pilot.credential(path), "$(touch /tmp/never-execute)")
            path.write_text('GROQ_API_KEY=one\nGROQ_API_KEY=two\n')
            with self.assertRaises(pilot.Refusal): pilot.credential(path)

    def test_transport_redacts_key_and_only_retains_limit_headers(self):
        frames = []
        class Sender:
            def send_bytes(self, data): frames.append(data)
            def close(self): pass
        class Response:
            code = 200
            headers = {"x-ratelimit-limit-tokens": "8000", "Authorization": "fixture-secret", "set-cookie": "private"}
            def read(self, limit):
                self.asserted_limit = limit
                return b'fixture-secret'
            def __enter__(self): return self
            def __exit__(self, *_): pass
        class Opener:
            def open(self, request, timeout):
                self.request = request
                return Response()
        opener = Opener()
        with patch.object(pilot.urllib.request, "build_opener", return_value=opener):
            pilot.provider_worker("fixture-secret", b"{}", Sender())
        self.assertEqual(json.loads(frames[0])["headers"], {"x-ratelimit-limit-tokens": "8000"})
        self.assertEqual(frames[1], b"[REDACTED]")
        self.assertEqual(opener.request.get_header("User-agent"), "pactrail-free-pilot/1.0")

    def test_oversized_provider_response_refused(self):
        frames = []
        class Sender:
            def send_bytes(self, data): frames.append(data)
            def close(self): pass
        class Response:
            code, headers = 200, {}
            def read(self, _limit): return b"x" * (pilot.MAX_BODY + 1)
            def __enter__(self): return self
            def __exit__(self, *_): pass
        class Opener:
            def open(self, *_args, **_kwargs): return Response()
        with patch.object(pilot.urllib.request, "build_opener", return_value=Opener()):
            pilot.provider_worker("fixture-secret", b"{}", Sender())
        self.assertEqual(json.loads(frames[0])["transport_error"], True)
        self.assertEqual(len(frames), 1)


if __name__ == "__main__": unittest.main()
