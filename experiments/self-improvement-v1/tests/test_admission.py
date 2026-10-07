"""Admission uses real Git snapshots, but never invokes a model or container."""
import copy
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from pactrail_lab.campaign import initialize
from pactrail_lab.safe import canonical, digest, Refusal
from pactrail_lab.store import Store


class Admission(unittest.TestCase):
    def test_real_pinned_sources_and_frozen_inputs_with_hostile_admission(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            def repository(name):
                path = root / name
                path.mkdir()
                env = {**os.environ, "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}
                def git(*args):
                    return subprocess.check_output(["git", "-c", "core.hooksPath=" + os.devnull,
                        "-c", "user.name=fixture", "-c", "user.email=fixture@invalid",
                        "-C", str(path), *args], env=env, timeout=10).decode().strip()
                git("init", "--quiet")
                (path / "value.txt").write_text(name + " bad")
                if name == "secret":
                    (path / ".env").write_text("FIXTURE_NOT_A_KEY=secret-like-file")
                git("add", "--all")
                git("commit", "--quiet", "-m", "bad")
                before = git("rev-parse", "HEAD")
                (path / "value.txt").write_text(name + " fixed")
                git("add", "--all")
                git("commit", "--quiet", "-m", "gold")
                return path, before, git("rev-parse", "HEAD")
            source, commit, _ = repository("harness")
            grader = root / "external.py"
            grader.write_text("# trusted admission fixture, not an actual grading program\n")
            operation = dict(argv=["python3", str(grader), "{report}"],
                             inputs={str(grader): digest(grader.read_bytes())}, timeout_seconds=10)
            protocol = dict(schema_version=1, id="cohort", seed=7, repetitions=3,
                            limits=dict(model_turns=4, wall_seconds=10, context_tokens=32768,
                                        output_tokens=1024, model_tokens=100000), tasks=[])
            paths = {}
            for cohort in ("development", "confirmation"):
                repo, before, gold = repository(cohort)
                task = dict(id=cohort, repository=str(repo), commit=before, gold_commit=gold,
                            goal="fix fixture", image="sha256:" + "1" * 64,
                            targeted=operation, regression=operation)
                paths[cohort] = root / (cohort + ".json")
                paths[cohort].write_bytes(canonical({**protocol, "id": cohort, "tasks": [task]}))
            binary = root / "binary"
            binary.write_bytes(b"fixture-binary-never-executed")
            manifest = dict(schema_version=1, id="admission", kind="fixture",
                model=dict(id="fixture", endpoint="https://example.invalid/v1/chat/completions",
                           api_key_env="FIXTURE_KEY", context_tokens=32768, output_tokens=1024,
                           input_rate=0, output_rate=0, temperature=0, reasoning_effort=None),
                limits=dict(cost_microusd=1, requests=100, wall_seconds=60, cycles=1, repetitions=3),
                image="sha256:" + "1" * 64, source=str(source), source_commit=commit,
                baseline_binary=str(binary), baseline_binary_sha256=digest(binary.read_bytes()),
                configuration={"agent_mode": "single"}, memory=[], build=operation,
                gates={name: operation for name in ("engineering", "authority", "compatibility", "recovery", "mechanism")},
                development_protocol=str(paths["development"]), confirmation_protocol=str(paths["confirmation"]))
            config = root / "manifest.json"
            config.write_bytes(canonical(manifest))
            result = initialize(root / "campaign", config)
            self.assertFalse(result["qualified"])
            with Store(root / "campaign") as store:
                self.assertEqual(store.load(store.value("manifest"))["containment"]["memory_mb"], 2048)
                snapshot = store.load(store.load(store.value("active"))["source"])
                self.assertEqual([e["path"] for e in snapshot["entries"]], ["value.txt"])
                self.assertEqual(store.get(store.value("external-inputs")[str(grader)]), grader.read_bytes())
                self.assertIsNone(store.value("reservations"))
            cases = []
            wrong_binary = copy.deepcopy(manifest)
            wrong_binary["baseline_binary_sha256"] = "0" * 64
            cases.append(wrong_binary)
            overlap = copy.deepcopy(manifest)
            overlap["confirmation_protocol"] = overlap["development_protocol"]
            cases.append(overlap)
            wrong_hash = copy.deepcopy(manifest)
            wrong_hash["build"]["inputs"][str(grader)] = "0" * 64
            cases.append(wrong_hash)
            bad_memory = copy.deepcopy(manifest)
            bad_memory["containment"] = {"memory_mb": True, "scratch_bytes": 67108864}
            cases.append(bad_memory)
            secret_repo, secret_commit, _ = repository("secret")
            secret = copy.deepcopy(manifest)
            secret.update(source=str(secret_repo), source_commit=secret_commit)
            cases.append(secret)
            for i, value in enumerate(cases):
                config.write_bytes(canonical(value))
                with self.assertRaises(Refusal): initialize(root / ("refused-" + str(i)), config)
