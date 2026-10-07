import json
from pathlib import Path
import tempfile
import unittest

import curate


class PopulationTests(unittest.TestCase):
    def test_expansion_preserves_original_tasks_and_repository_partition(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            rows = []
            for repo in curate.REPOSITORIES:
                for index in range(3):
                    rows.append({"row": {"repo": repo,
                        "instance_id": repo.replace("/", "__") + "-" + str(index),
                        "base_commit": "a" * 40, "problem_statement": "Fixture only",
                        "patch": "fixture fix", "test_patch": "fixture test",
                        "eval_script": "fixture command", "log_parser": "fixture parser",
                        "FAIL_TO_PASS": ["target"], "PASS_TO_PASS": ["regression"]}})
            source = root / "snapshot.json"
            source.write_text(json.dumps({"rows": rows}))
            first, expanded = root / "first.json", root / "expanded.json"
            curate.curate([source], first)
            curate.curate([source], expanded, 3)
            a, b = json.loads(first.read_text()), json.loads(expanded.read_text())
            self.assertEqual(len(a["tasks"]), 30)
            self.assertEqual(len(b["tasks"]), 45)
            lookup = {t["id"]: t for t in b["tasks"]}
            self.assertTrue(all(lookup[t["id"]] == t for t in a["tasks"]))
            with self.assertRaises(ValueError):
                curate.curate([source], root / "invalid.json", 4)


if __name__ == "__main__":
    unittest.main()
