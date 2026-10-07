import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("runner", Path(__file__).with_name("run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class RunnerTests(unittest.TestCase):
    def test_external_rust_grader_does_not_reuse_bad_or_gold_path_artifacts(self):
        grader = Path(__file__).with_name("issue_grader.py").resolve()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            baseline = root / "baseline"
            baseline.mkdir()
            (baseline / "Cargo.toml").write_text('[package]\nname="grading-cache-fixture"\nversion="0.1.0"\nedition="2024"\n[workspace]\n')
            manifest = root / "issues.json"
            manifest.write_text(json.dumps([{"id": "cache", "baseline": str(baseline), "forbidden_paths": [], "overlay_sha256": {}, "targeted": ["cargo", "test", "--quiet"]}]))
            for index, value in enumerate([0, 1, 0]):
                workspace = root / str(index)
                (workspace / "src").mkdir(parents=True)
                (workspace / "Cargo.toml").write_text((baseline / "Cargo.toml").read_text())
                source = workspace / "src/lib.rs"
                source.write_text(f'pub fn value()->u8 {{{value}}}\n#[test] fn regression() {{assert_eq!(value(),1);}}\n')
                import os
                os.utime(source, (946684800, 946684800))
                result = subprocess.run([sys.executable, str(grader), str(manifest), "cache", "targeted"], cwd=workspace, capture_output=True)
                self.assertEqual(result.returncode == 0, value == 1, result.stdout + result.stderr)

    def test_sealed_workspace_removes_origin_and_rejects_additional_history(self):
        import os
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, workspace = root / "source", root / "workspace"
            source.mkdir()
            subprocess.run(["git", "init", str(source)], capture_output=True, check=True)
            def git(*args):
                return subprocess.check_output(["git", "-C", str(source), *args], text=True).strip()
            (source / "value").write_text("baseline")
            git("add", ".")
            git("-c", "user.name=Fixture", "-c", "user.email=fixture@invalid", "commit", "-m", "baseline")
            commit = git("rev-parse", "HEAD")
            subprocess.run(["git", "clone", "--no-hardlinks", str(source), str(workspace)],
                           capture_output=True, check=True)
            runner.seal_workspace(workspace, commit, os.environ)
            self.assertEqual(subprocess.check_output(["git", "-C", str(workspace), "remote"], text=True), "")
            self.assertEqual(git("rev-parse", "HEAD"), commit)
            (source / "value").write_text("future fix")
            git("add", ".")
            git("-c", "user.name=Fixture", "-c", "user.email=fixture@invalid", "commit", "-m", "future")
            with self.assertRaisesRegex(ValueError, "synthetic baseline"):
                runner.seal_workspace(source, git("rev-parse", "HEAD"), os.environ)

    def test_functional_correctness_does_not_imply_strict_completion(self):
        self.assertIsNone(runner.strict_completion(True, None))
        self.assertIsNone(runner.strict_completion(True, {"receipt_valid": True}))
        checks = {key: True for key in ("receipt_valid", "trace_valid", "source_isolation_valid", "ready_to_apply")}
        self.assertTrue(runner.strict_completion(True, checks))
        self.assertFalse(runner.strict_completion(False, checks))
        self.assertFalse(runner.strict_completion(True, {**checks, "ready_to_apply": False}))
        with self.assertRaises(ValueError):
            runner.strict_completion(True, {**checks, "receipt_valid": 1})

    def test_missing_and_zero_metrics_differ(self):
        measured = runner.metrics({"metrics": {"cost_microusd": 0}})
        self.assertEqual(measured["cost_microusd"], 0)
        self.assertIsNone(measured["input_tokens"])

    def test_rejects_invalid_measurements(self):
        for value in [-1, True, float("nan"), "0"]:
            with self.assertRaises(ValueError):
                runner.metrics({"metrics": {"input_tokens": value}})
        with self.assertRaises(ValueError):
            runner.metrics({"metrics": {"imagined_metric": 7}})

    def test_process_group_timeout(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result = runner.command([sys.executable, "-c", "import time; time.sleep(60)"], root, root, 0.05)
            self.assertTrue(result["timed_out"])
            self.assertNotEqual(result["exit_code"], 0)

    def test_matched_trials_keep_raw_results_and_do_not_score_unsupported_or_malformed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            source.mkdir()
            subprocess.run(["git", "init", str(source)], capture_output=True, check=True)
            (source / "value.txt").write_text("before")
            subprocess.run(["git", "-C", str(source), "add", "."], check=True)
            subprocess.run(["git", "-C", str(source), "-c", "user.name=Fixture", "-c", "user.email=fixture@invalid", "commit", "-m", "fixture"], capture_output=True, check=True)
            commit = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
            adapter = root / "adapter.py"
            adapter.write_text('''import json,sys
from pathlib import Path
r=json.loads(Path(sys.argv[1]).read_text())
assert set(r['task']) == {'id','goal','commit'}
if r['arm']['id']=='unsupported': sys.exit(78)
if r['arm']['id']=='malformed': Path(r['trial_directory'],'result.json').write_text('{}'); sys.exit(0)
p=Path(r['workspace'],'value.txt');p.write_text('after')
if r['arm']['id']=='partial':
 Path(r['trial_directory'],'result.json').write_text(json.dumps({'schema_version':1,'model_identity':r['model_identity'],'partial_candidate':r['workspace'],'checks':{'trace_valid':True,'source_isolation_valid':True},'metrics':{'input_tokens':99}})); sys.exit(1)
Path(r['trial_directory'],'result.json').write_text(json.dumps({'schema_version':1,'model_identity':r['model_identity'],'candidate':r['workspace'],'metrics':{'cost_microusd':0}}))
''')
            grader = [sys.executable, "-c", "from pathlib import Path; assert Path('value.txt').read_text() == 'after'; assert not Path('grader-output').exists(); Path('grader-output').write_text('hidden overlay or generated artifact')"]
            protocol = {"schema_version": 1, "seed": 42, "repetitions": 1, "grade_partial_candidates": True, "model_identity": {"model": "fixture-not-inference"},
                "permissions": {"process": "disabled", "write_paths": ["."]}, "normalization": "equal declared ceilings",
                "limits": {"model_turns": 4, "wall_seconds": 5, "output_tokens": 256, "context_tokens": 4096, "model_tokens": 16384},
                "tasks": [{"id": "fixture", "repository": str(source), "commit": commit, "goal": "Change value.", "targeted": grader, "regression": grader}],
                "arms": [{"id": name, "mode": "single", "adapter": [sys.executable, str(adapter)]} for name in ["valid", "unsupported", "malformed", "partial"]]}
            path = root / "protocol.json"
            runner.write(path, protocol)
            rows = runner.run(path, root / "results")
            by_arm = {row["arm"]: row for row in rows}
            self.assertTrue(by_arm["valid"]["task_success"])
            self.assertEqual(by_arm["valid"]["metrics"]["cost_microusd"], 0)
            self.assertEqual(by_arm["unsupported"]["status"], "unsupported")
            self.assertEqual(by_arm["malformed"]["status"], "invalid")
            self.assertEqual(by_arm["partial"]["status"], "failed")
            self.assertFalse(by_arm["partial"]["task_success"])
            self.assertFalse(by_arm["partial"]["strict_completion"])
            self.assertTrue(by_arm["partial"]["diagnostic_targeted_passed"])
            self.assertTrue(by_arm["partial"]["diagnostic_regression_passed"])
            self.assertEqual(by_arm["partial"]["metrics"]["input_tokens"], 99)
            self.assertEqual((source / "value.txt").read_text(), "before")
            self.assertTrue((root / "results" / "fixture--valid--0" / "sha256.json").exists())
            for bad in ["../escape", "A", ""]:
                protocol["arms"][0]["id"] = bad
                with self.assertRaises(ValueError):
                    runner.validate(protocol)


class PairedAnalysisTests(unittest.TestCase):
    def test_analysis_keeps_invalid_and_unsupported_pairs_and_rejects_duplicates(self):
        spec = importlib.util.spec_from_file_location("analysis", Path(__file__).with_name("analyze.py"))
        analysis = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(analysis)
        rows = [{"task": "one", "repeat": 0, "arm": "base", "status": "scored", "task_success": True, "strict_completion": None},
                {"task": "one", "repeat": 0, "arm": "new", "status": "invalid", "task_success": False},
                {"task": "two", "repeat": 0, "arm": "base", "status": "unsupported", "task_success": False},
                {"task": "two", "repeat": 0, "arm": "new", "status": "scored", "task_success": True, "strict_completion": True}]
        result = analysis.paired(rows, "base", "new", samples=100)
        self.assertEqual(result["paired_tasks"], 2)
        self.assertEqual(result["difference"], 0)
        self.assertEqual(analysis.paired(rows, "base", "new", samples=100, metric="strict_completion")["difference"], .5)
        with self.assertRaisesRegex(ValueError, "duplicate"):
            analysis.paired(rows + [rows[0]], "base", "new")
        metrics = {"input_tokens": 100, "cached_input_tokens": 0}
        row = {"task": "one", "arm": "base", "task_success": True, "metrics": metrics}
        self.assertIsNone(analysis.resource_summary([row], "base")["cache_hit_ratio"]["value"])
        row["usage_coverage"] = {k: {"total_turns": 1, "explicit_reported_turns": 1} for k in metrics}
        self.assertEqual(analysis.resource_summary([row], "base")["cache_hit_ratio"]["value"], 0)

    def test_paired_analysis_uses_tasks_as_clusters_and_exposes_no_coverage(self):
        spec = importlib.util.spec_from_file_location("analysis", Path(__file__).with_name("analyze.py"))
        analysis = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(analysis)
        rows = [{"task": "one", "repeat": 0, "arm": "correct", "status": "scored", "task_success": True},
                {"task": "one", "repeat": 0, "arm": "zero", "status": "scored", "task_success": False},
                {"task": "two", "repeat": 0, "arm": "correct", "status": "unsupported", "task_success": False}]
        result = analysis.paired(rows, "correct", "zero", samples=100)
        self.assertEqual(result["paired_tasks"], 1)
        self.assertEqual(result["difference"], -1)
        self.assertEqual(analysis.paired(rows, "correct", "absent")["difference"], None)


if __name__ == "__main__":
    unittest.main()
