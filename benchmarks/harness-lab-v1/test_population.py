import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).parent))
import validate_population
from test_lab import task


class PopulationRetentionTests(unittest.TestCase):
    def test_unsupported_cases_are_retained_without_docker_or_parser_calls(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            catalogue, prepared, images = root / "catalogue.json", root / "prepared.json", root / "images.json"
            tasks = [task(i) for i in range(20)]
            catalogue.write_text(json.dumps({"schema_version": 1, "tasks": tasks}))
            prepared.write_text(json.dumps([{"id": t["id"], "status": "preparation_failed"} for t in tasks]))
            images.write_text(json.dumps([{"task": t["id"], "status": "unsupported_preparation"} for t in tasks]))
            args = SimpleNamespace(catalogue=catalogue, prepared=prepared, images=images,
                                   output=root / "evidence", timeout=1, parser_python=Path(sys.executable))
            with mock.patch.object(validate_population, "control", side_effect=AssertionError("unexpected Docker call")):
                rows = validate_population.validate(args)
            self.assertEqual(len(rows), 20)
            self.assertEqual({r["task"] for r in rows}, {t["id"] for t in tasks})
            self.assertTrue(all(r["status"] == "unsupported_preparation" for r in rows))
            with self.assertRaises(FileExistsError):
                validate_population.validate(args)
            images.write_text("[]")
            args.output = root / "refused"
            with self.assertRaises(ValueError):
                validate_population.validate(args)
            self.assertFalse(args.output.exists())


if __name__ == "__main__":
    unittest.main()
