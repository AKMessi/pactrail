import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import subprocess
import unittest

spec = importlib.util.spec_from_file_location("lab", Path(__file__).with_name("lab.py"))
lab = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lab)


def task(index=0):
    return {"id": f"issue-{index}", "base_commit": "a" * 40,
            "repository": "fixture/repo", "language": "python", "source": "fixture-only",
            "split": "development" if index < 15 else "confirmation",
            "grader_identity": "b" * 64, "goal_sha256": hashlib.sha256(b"Fix it").hexdigest()}


class AdmissionTests(unittest.TestCase):
    def test_distinct_explicit_splits_and_pinned_provenance(self):
        catalogue = {"schema_version": 1, "tasks": [task(i) for i in range(20)]}
        self.assertEqual(len(lab.validate_catalogue(catalogue)), 20)
        for bad in (catalogue["tasks"][:19], catalogue["tasks"] + [task(0)]):
            with self.assertRaises(ValueError):
                lab.validate_catalogue({"schema_version": 1, "tasks": bad})
        bad = copy.deepcopy(catalogue)
        bad["tasks"][0]["base_commit"] = "main"
        with self.assertRaises(ValueError):
            lab.validate_catalogue(bad)
        with self.assertRaisesRegex(ValueError, "repository overlaps"):
            lab.validate_catalogue({**catalogue, "partition_policy": "repository_disjoint"})

    def test_zero_tests_setup_failure_and_log_tampering_never_qualify(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "grader.log").write_text("fixture test executed\n")
            current = task()
            row = {"tests_executed": 1, "tests_failed": 0, "exit_code": 0,
                   "timed_out": False, "infrastructure_error": False,
                   "log": "grader.log", "log_sha256": lab.sha256(root / "grader.log")}
            evidence = {"schema_version": 1, "task": current["id"],
                        "task_sha256": hashlib.sha256(lab.canonical(current)).hexdigest(),
                        "grader_identity": current["grader_identity"],
                        "base_targeted": {**row, "tests_failed": 1, "exit_code": 1},
                        "gold_targeted": dict(row), "gold_regression": dict(row)}
            lab.validate_evidence(current, evidence, root)
            for phase, key, value in [("base_targeted", "tests_failed", 0),
                                      ("gold_targeted", "tests_executed", 0),
                                      ("base_targeted", "infrastructure_error", True),
                                      ("gold_regression", "timed_out", True),
                                      ("gold_targeted", "tests_executed", True),
                                      ("gold_targeted", "log", "../escape.log")]:
                bad = copy.deepcopy(evidence)
                bad[phase][key] = value
                with self.assertRaises(ValueError):
                    lab.validate_evidence(current, bad, root)
            altered = {**current, "base_commit": "c" * 40}
            with self.assertRaises(ValueError):
                lab.validate_evidence(altered, evidence, root)
            (root / "grader.log").write_text("rewritten")
            with self.assertRaises(ValueError):
                lab.validate_evidence(current, evidence, root)

    def test_nonfinite_duplicate_and_oversized_json_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "input.json"
            for text in ('{"x":NaN}', '{"x":1,"x":2}', 'null', '[]', '1',
                         ' ' * (lab.MAX_JSON_BYTES + 1)):
                path.write_text(text)
                with self.assertRaises(ValueError):
                    lab.read_json(path)

    def test_catalogue_is_not_eligibility_and_freeze_needs_runtime_proof(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            catalogue = {"schema_version": 1, "tasks": [task(i) for i in range(20)]}
            (root / "catalogue.json").write_text(json.dumps(catalogue))
            protocol = {"schema_version": 1, "seed": 22, "repetitions": 1,
                        "model_identity": {"model": "fixture-only"},
                        "permissions": {"process": "disabled"}, "normalization": "equal ceilings",
                        "lab_split": "development", "source_policy": "sealed",
                        "limits": {"model_turns": 24, "wall_seconds": 300, "output_tokens": 1024,
                                   "context_tokens": 32768, "model_tokens": 500000},
                        "arms": [{"id": "single", "mode": "single", "adapter": ["python3"]}],
                        "tasks": [{"id": f"issue-{i}", "commit": "d" * 40, "goal": "Fix it",
                                   "targeted": ["python3"], "regression": ["python3"]} for i in range(15)]}
            (root / "protocol.json").write_text(json.dumps(protocol))
            with self.assertRaisesRegex(ValueError, "runtime binary"):
                lab.freeze(root / "catalogue.json", root / "protocol.json", root, root / "frozen.json")
            self.assertFalse((root / "frozen.json").exists())

    def test_frozen_sources_logs_and_adapters_are_bound_and_output_is_exclusive(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo = root / "repo"
            repo.mkdir()
            (repo / "value.txt").write_text("bad")
            subprocess.run(["git", "init", str(repo)], check=True, capture_output=True)
            subprocess.run(["git", "-C", str(repo), "add", "."], check=True)
            subprocess.run(["git", "-C", str(repo), "-c", "user.name=Fixture",
                            "-c", "user.email=fixture@invalid", "commit", "-m", "sealed"],
                           check=True, capture_output=True)
            commit = subprocess.check_output(["git", "-C", str(repo), "rev-parse", "HEAD"], text=True).strip()
            tasks = [task(i) for i in range(20)]
            for item in tasks[1:]:
                item["split"] = "confirmation"
            catalogue = {"schema_version": 1, "tasks": tasks}
            (root / "catalogue.json").write_text(json.dumps(catalogue))
            binary = Path(os.sys.executable).resolve()
            grader = root / "grader.py"
            grader.write_text("# Trusted fixture only\n")
            inputs = {str(grader): lab.sha256(grader)}
            protocol = {"schema_version": 1, "seed": 22, "repetitions": 1,
                        "model_identity": {"model": "fixture-only"},
                        "permissions": {"process": "disabled"}, "normalization": "equal ceilings",
                        "lab_split": "development", "source_policy": "sealed", "runtime_identity": {
                            "binary": str(binary), "binary_sha256": lab.sha256(binary), "commit": "e" * 40},
                        "limits": {"model_turns": 24, "wall_seconds": 300, "output_tokens": 1024,
                                   "context_tokens": 32768, "model_tokens": 500000},
                        "arms": [{"id": "single", "mode": "single", "adapter": [str(binary), str(grader)]}],
                        "tasks": [{"id": "issue-0", "commit": commit, "source_base_commit": "a" * 40,
                                   "repository": str(repo), "goal": "Fix it",
                                   "targeted": [str(binary), str(grader)], "regression": [str(binary), str(grader)],
                                   "targeted_inputs": inputs, "regression_inputs": inputs}]}
            (root / "protocol.json").write_text(json.dumps(protocol))
            log = root / "tests.log"
            log.write_text("fixture executed test\n")
            row = {"tests_executed": 1, "tests_failed": 0, "exit_code": 0,
                   "timed_out": False, "infrastructure_error": False,
                   "log": log.name, "log_sha256": lab.sha256(log)}
            evidence = {"schema_version": 1, "task": "issue-0",
                        "task_sha256": hashlib.sha256(lab.canonical(tasks[0])).hexdigest(),
                        "grader_identity": tasks[0]["grader_identity"],
                        "base_targeted": {**row, "tests_failed": 1, "exit_code": 1},
                        "gold_targeted": row, "gold_regression": row}
            (root / "issue-0.json").write_text(json.dumps(evidence))
            out = root / "frozen.json"
            lab.freeze(root / "catalogue.json", root / "protocol.json", root, out)
            frozen = lab.read_json(out)
            # macOS /var aliases and Windows short temporary paths resolve to
            # different spellings; the protocol deliberately binds canonical paths.
            self.assertEqual(frozen["frozen_inputs"][str(log.resolve())], lab.sha256(log))
            self.assertEqual(frozen["frozen_inputs"][str(grader.resolve())], lab.sha256(grader))
            with self.assertRaises(FileExistsError):
                lab.freeze(root / "catalogue.json", root / "protocol.json", root, out)
            log.write_text("tampered")
            with self.assertRaisesRegex(ValueError, "log digest"):
                lab.freeze(root / "catalogue.json", root / "protocol.json", root, root / "second.json")
            self.assertFalse((root / "second.json").exists())


if __name__ == "__main__":
    unittest.main()
