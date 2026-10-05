import hashlib
import json
from pathlib import Path
import tempfile
import unittest

import admit
from grading import summarize
from lab import canonical, sha256
from test_lab import task


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value))


class ExportTests(unittest.TestCase):
    def fixture(self, root):
        grader = {"FAIL_TO_PASS": ["target"], "PASS_TO_PASS": ["regression"]}
        tasks = [task(i) for i in range(20)]
        for current in tasks:
            current["grader_identity"] = hashlib.sha256(canonical(grader)).hexdigest()
        catalogue, prepared, campaign = root / "catalogue.json", root / "prepared.json", root / "campaign"
        write(catalogue, {"schema_version": 1, "tasks": tasks})
        write(root / "source" / "grader.json", grader)
        prepared.write_text(json.dumps([{"id": t["id"], "baseline": str(root / "source" / "baseline"), "source_tree_sha256": "a" * 64, "gold_tree_sha256": "a" * 64} for t in tasks]))
        write(campaign / "campaign.json", {"catalogue_sha256": sha256(catalogue),
              "prepared_sha256": sha256(prepared), "scored": False, "order": [t["id"] for t in tasks]})
        for current in tasks:
            write(campaign / current["id"] / "result.json", {"task": current["id"], "status": "unsupported_preparation"})
        current = tasks[0]
        write(campaign / current["id"] / "result.json", {"task": current["id"], "status": "validated"})
        for source in ("base", "gold"):
            directory = campaign / current["id"] / source
            directory.mkdir()
            log = directory / "test.log"
            log.write_bytes(b"retained test output\n")
            statuses = {"target": "FAILED" if source == "base" else "PASSED", "regression": "PASSED"}
            write(directory / "parsed.json", {"upstream_commit": admit.UPSTREAM_COMMIT,
                  "log_sha256": sha256(log), "statuses": statuses})
            write(directory / "result.json", {"task": current["id"], "source": source,
                  "grader_identity": current["grader_identity"], "grading_source_commit": current["base_commit"],
                  "source_tree_sha256": "a" * 64, "timed_out": False,
                  "output_limit": False, "shell_exit_code": 0, "log_sha256": sha256(log),
                  "targeted": summarize(grader["FAIL_TO_PASS"], statuses),
                  "regression": summarize(grader["PASS_TO_PASS"], statuses)})
        return catalogue, prepared, campaign

    def test_actual_behavior_is_distinct_from_shell_exit_and_all_cases_retained(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            args = self.fixture(root)
            output = root / "admission"
            results = admit.export(*args, output)
            self.assertEqual(len(results), 20)
            self.assertEqual(sum(r["status"] == "admitted" for r in results), 1)
            evidence = json.loads((output / "issue-0.json").read_text())
            self.assertEqual(evidence["base_targeted"]["shell_exit_code"], 0)
            self.assertEqual(evidence["base_targeted"]["exit_code"], 1)
            self.assertEqual(evidence["base_targeted"]["exit_code_kind"], "derived_behavioral")
            with self.assertRaises(FileExistsError):
                admit.export(*args, output)

    def test_log_tampering_and_forged_summary_are_refused(self):
        for mutation in ("log", "summary"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                args = self.fixture(root)
                source = args[2] / "issue-0" / "gold"
                if mutation == "log":
                    (source / "test.log").write_text("changed")
                else:
                    p = source / "result.json"
                    value = json.loads(p.read_text())
                    value["targeted"]["tests_executed"] = 999
                    write(p, value)
                results = admit.export(*args, root / "out")
                self.assertEqual(results[0]["status"], "not_admitted")
                self.assertFalse((root / "out" / "issue-0.json").exists())

    def test_incomplete_campaign_emits_no_admission_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            args = self.fixture(root)
            (args[2] / "issue-19" / "result.json").unlink()
            with self.assertRaises(FileNotFoundError):
                admit.export(*args, root / "out")
            self.assertFalse((root / "out").exists())


if __name__ == "__main__":
    unittest.main()
