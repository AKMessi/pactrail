import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).parent))
spec = importlib.util.spec_from_file_location("prepare", Path(__file__).with_name("prepare.py"))
prepare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prepare)


class PreparationTests(unittest.TestCase):
    def test_base_gold_and_grader_are_separate_and_source_history_is_sealed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            mirrors = root / "mirrors"
            repository = mirrors / "fixture--repo"
            repository.mkdir(parents=True)
            (repository / "value.txt").write_text("bad\n")
            prepare.git(repository, "init", stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            prepare.git(repository, "add", ".", stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            prepare.git(repository, "-c", "user.name=Fixture", "-c", "user.email=fixture@invalid",
                        "commit", "-m", "base", stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            revision = prepare.git(repository, "rev-parse", "HEAD", stdout=subprocess.PIPE).stdout.decode().strip()
            patch = "diff --git a/value.txt b/value.txt\n--- a/value.txt\n+++ b/value.txt\n@@ -1 +1 @@\n-bad\n+good\n"
            grader = {"test_patch": "fixture", "eval_script": "fixture-only", "log_parser": "fixture",
                      "FAIL_TO_PASS": ["fixture-test"], "PASS_TO_PASS": ["fixture-regression"]}
            rows, tasks = [], []
            for index in range(20):
                identity = f"fixture-{index}"
                row = {**grader, "instance_id": identity, "repo": "fixture/repo", "base_commit": revision,
                       "patch": patch, "problem_statement": "Fix it", "image": "fixture-only"}
                rows.append({"row": row})
                tasks.append({"id": identity, "upstream_instance_id": identity, "repository": "fixture/repo",
                              "language": "python", "source": "rows.json", "base_commit": revision,
                              "split": "development" if index < 15 else "confirmation",
                              "goal_sha256": hashlib.sha256(b"Fix it").hexdigest(),
                              "grader_identity": hashlib.sha256(prepare.canonical(grader)).hexdigest(),
                              "reference_patch_sha256": hashlib.sha256(patch.encode()).hexdigest()})
            source = root / "rows.json"
            source.write_text(json.dumps({"rows": rows}))
            catalogue = root / "catalogue.json"
            catalogue.write_text(json.dumps({"schema_version": 1, "tasks": tasks,
                                            "source_snapshots": {source.name: prepare.sha256(source)}}))
            out = root / "prepared"
            results = prepare.prepare(catalogue, root, mirrors, out)
            self.assertTrue(all(row["status"] == "prepared_not_validated" for row in results), results)
            self.assertTrue(all(row["eligible_for_scoring"] is False for row in results))
            baseline = out / "fixture-0" / "baseline"
            self.assertEqual((baseline / "value.txt").read_text(), "bad\n")
            self.assertEqual((out / "fixture-0" / "gold" / "value.txt").read_text(), "good\n")
            self.assertFalse((baseline / "grader.json").exists())
            self.assertEqual(prepare.git(baseline, "remote", stdout=subprocess.PIPE).stdout, b"")
            self.assertEqual(prepare.git(baseline, "rev-list", "--count", "HEAD", stdout=subprocess.PIPE).stdout, b"1\n")
            source.write_text("tampered")
            with self.assertRaisesRegex(ValueError, "snapshot changed"):
                prepare.prepare(catalogue, root, mirrors, root / "refused")
            self.assertFalse((root / "refused").exists())

    def test_git_symlink_is_rejected_before_archive_extraction(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repository = root / "repo"
            repository.mkdir()
            prepare.git(repository, "init", stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            digest = prepare.git(repository, "hash-object", "-w", "--stdin", input=b"../escape",
                                 stdout=subprocess.PIPE).stdout.decode().strip()
            prepare.git(repository, "update-index", "--add", "--cacheinfo", "120000", digest, "link",
                        stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            prepare.git(repository, "-c", "user.name=Fixture", "-c", "user.email=fixture@invalid",
                        "commit", "-m", "link", stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            destination = root / "output"
            destination.mkdir()
            with self.assertRaisesRegex(ValueError, "nonregular"):
                prepare.export(repository, "HEAD", destination)
            self.assertEqual(list(destination.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
