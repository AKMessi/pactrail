import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("crouton_profile", ROOT / "benchmarks/nemotron-rsi-pilot-v1/pilot.py")
pilot = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pilot)


class ProfileTests(unittest.TestCase):
    def test_frozen_profile_is_exact_and_bounded(self):
        value = pilot.load_profile(Path(__file__).with_name("profile.json"))
        self.assertEqual(value["model"], "stealth/crouton-alpha")
        self.assertEqual(value["max_requests"], len(value["tasks"]) * 3 * value["engine_limits"]["model_turns"])
        pilot.check_catalog({"id": value["model"], "pricing": {"prompt": "0", "completion": "0"},
                             "supported_parameters": ["tools", "reasoning", "temperature", "max_tokens"]}, value["model"])

    def test_no_paid_substitution_or_expanded_limits(self):
        original = json.loads(Path(__file__).with_name("profile.json").read_text())
        variants = [{**original, "model": "another-model"}, {**original, "max_requests": 193},
                    {**original, "max_reported_tokens": 4000001}, {**original, "extra": True},
                    {**original, "tasks": [original["tasks"][0]] * 4},
                    {**original, "tasks": [{}]},
                    {**original, "engine_limits": {**original["engine_limits"], "output_tokens": 8193}}]
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "profile.json"
            for variant in variants:
                path.write_text(json.dumps(variant))
                with self.assertRaises(pilot.Refusal): pilot.load_profile(path)
