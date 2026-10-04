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
Path(r['trial_directory'],'result.json').write_text(json.dumps({'schema_version':1,'model_identity':r['model_identity'],'candidate':r['workspace'],'metrics':{'cost_microusd':0}}))
''')
            grader = [sys.executable, "-c", "from pathlib import Path; assert Path('value.txt').read_text() == 'after'"]
            protocol = {"schema_version": 1, "seed": 42, "repetitions": 1, "model_identity": {"model": "fixture-not-inference"},
                "permissions": {"process": "disabled", "write_paths": ["."]}, "normalization": "equal declared ceilings",
                "limits": {"model_turns": 4, "wall_seconds": 5, "output_tokens": 256, "context_tokens": 4096, "model_tokens": 16384},
                "tasks": [{"id": "fixture", "repository": str(source), "commit": commit, "goal": "Change value.", "targeted": grader, "regression": grader}],
                "arms": [{"id": name, "mode": "single", "adapter": [sys.executable, str(adapter)]} for name in ["valid", "unsupported", "malformed"]]}
            path = root / "protocol.json"
            runner.write(path, protocol)
            rows = runner.run(path, root / "results")
            by_arm = {row["arm"]: row for row in rows}
            self.assertTrue(by_arm["valid"]["task_success"])
            self.assertEqual(by_arm["valid"]["metrics"]["cost_microusd"], 0)
            self.assertEqual(by_arm["unsupported"]["status"], "unsupported")
            self.assertEqual(by_arm["malformed"]["status"], "invalid")
            self.assertEqual((source / "value.txt").read_text(), "before")
            self.assertTrue((root / "results" / "fixture--valid--0" / "sha256.json").exists())
            for bad in ["../escape", "A", ""]:
                protocol["arms"][0]["id"] = bad
                with self.assertRaises(ValueError):
                    runner.validate(protocol)


class PairedAnalysisTests(unittest.TestCase):
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
