import hashlib
import json
from pathlib import Path
import tempfile
import unittest

import cohort
from lab import canonical, sha256
from test_lab import task


class CohortTests(unittest.TestCase):
    def fixture(self, root, admitted=20):
        population = root / "population.json"
        tasks = [task(i) for i in range(22)]
        population.write_text(json.dumps({"schema_version": 1, "tasks": tasks}))
        admission = root / "admission"
        admission.mkdir()
        log = admission / "tests.log"
        log.write_text("fixture-only grading log")
        cases = []
        for index, current in enumerate(tasks):
            case = {"task": current["id"], "status": "not_admitted", "qualification_status": "unsupported_preparation"}
            if index < admitted:
                record = {"tests_executed": 1, "tests_failed": 0, "exit_code": 0,
                          "timed_out": False, "infrastructure_error": False,
                          "log": log.name, "log_sha256": sha256(log)}
                evidence = {"schema_version": 1, "task": current["id"],
                            "task_sha256": hashlib.sha256(canonical(current)).hexdigest(),
                            "grader_identity": current["grader_identity"],
                            "base_targeted": {**record, "tests_failed": 1, "exit_code": 1},
                            "gold_targeted": record, "gold_regression": record}
                path = admission / (current["id"] + ".json")
                path.write_text(json.dumps(evidence))
                case.update(status="admitted", qualification_status="validated", evidence_sha256=sha256(path))
            cases.append(case)
        (admission / "admission-ledger.json").write_text(json.dumps({
            "catalogue_sha256": sha256(population), "scored": False, "cases": cases}))
        return population, admission

    def test_cohort_preserves_task_specs_exclusions_and_hashes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            args = self.fixture(root)
            output = root / "cohort.json"
            result = cohort.derive(*args, output)
            self.assertEqual(len(result["tasks"]), 20)
            self.assertEqual(result["eligibility"]["original_population_count"], 22)
            self.assertEqual(len(result["eligibility"]["excluded"]), 2)
            original = {t["id"]: t for t in json.loads(args[0].read_text())["tasks"]}
            self.assertTrue(all(t == original[t["id"]] for t in result["tasks"]))
            self.assertEqual(result["qualification_inputs"][str(args[0].resolve())], sha256(args[0]))
            with self.assertRaises(FileExistsError):
                cohort.derive(*args, output)

    def test_insufficient_population_or_modified_logs_emit_no_cohort(self):
        for mutation in ("small", "log"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                args = self.fixture(root, admitted=19 if mutation == "small" else 20)
                if mutation == "log":
                    (args[1] / "tests.log").write_text("tampered")
                with self.assertRaises(ValueError):
                    cohort.derive(*args, root / "cohort.json")
                self.assertFalse((root / "cohort.json").exists())


if __name__ == "__main__":
    unittest.main()
