import os
from pathlib import Path
import sys
import tempfile
import unittest
import time
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from pactrail_lab.process import Oci
from pactrail_lab import budget
from pactrail_lab.safe import Refusal
from pactrail_lab.store import Store


class OciContracts(unittest.TestCase):
    def test_bounded_mounts_no_network_and_recovery_failure_stays_pending(self):
        image = "sha256:" + "1" * 64
        calls = []
        def invoke(argv, **kwargs):
            calls.append(argv)
            return dict(exit_code=0, reason=None, wall_time_ms=0, stdout=(image + "\n").encode() if "image" in argv else b"", stderr=b"")
        with tempfile.TemporaryDirectory() as tmp, Store(Path(tmp) / "campaign", create=True) as store:
            source = Path(tmp) / "source"
            source.mkdir()
            with patch("pactrail_lab.process.command", invoke):
                Oci(store, image).run(["true"], [(source, "/source", False)])
                args = next(c for c in calls if "create" in c)
                for arg in ("--network=none", "--read-only", "--cap-drop=ALL", "--security-opt=no-new-privileges"):
                    self.assertIn(arg, args)
                self.assertTrue(any(arg.startswith("/work:rw,nosuid,size=1073741824,mode=0700,uid=") for arg in args))
                with self.assertRaises(Refusal): Oci(store, image).run(["true"], [(source, "/work", False)])
                with self.assertRaises(Refusal): Oci(store, image).run(["true"], [(source, "/source", True)])
                with self.assertRaises(Refusal): Oci(store, image).run(["true"], [(source, "/source/../escape", False)])
                count = len(calls)
                with self.assertRaises(Refusal): Oci(store, image).run([], [])
                self.assertEqual(len(calls), count)
                with self.assertRaises(Refusal): Oci(store, image).run(["true"], [], memory_mb=16385)
                with store.transaction(): store.set("deadline-ns", time.time_ns() - 1)
                before = len([c for c in calls if "create" in c])
                with self.assertRaisesRegex(Refusal, "wall-clock"): Oci(store, image).run(["true"], [])
                self.assertEqual(len([c for c in calls if "create" in c]), before)
            with store.transaction(): store.set("containers", {"pending": {"state": "reserved", "image": image}})
            with patch("pactrail_lab.process.command", lambda *_args, **_kw: dict(exit_code=1, reason=None, stdout=b"", stderr=b"permission denied")):
                with self.assertRaises(Refusal): Oci(store, image).recover()
            self.assertEqual(store.value("containers")["pending"]["state"], "reserved")


class RealContainment(unittest.TestCase):
    @unittest.skipUnless(os.environ.get("PACTRAIL_LAB_TEST_IMAGE"), "real OCI qualification requires an explicitly provisioned image")
    def test_real_worker_has_no_host_write_or_network_authority(self):
        from pactrail_lab.execution import Execution, supervisor_inputs
        from pactrail_lab.snapshots import import_tree
        with tempfile.TemporaryDirectory() as tmp, Store(Path(tmp) / "campaign", create=True) as store:
            source = Path(tmp) / "source"
            source.mkdir()
            (source / "marker").write_text("unchanged")
            snapshot = import_tree(store, source)
            with store.transaction():
                inputs = supervisor_inputs()
                for path in inputs: store.put(Path(path).read_bytes())
                store.set("supervisor-inputs", inputs)
                store.set("external-inputs", {})
                store.set("manifest", store.record({"image": os.environ["PACTRAIL_LAB_TEST_IMAGE"]}))
            script = "import socket,os; assert os.getuid()!=0; denied=False\ntry: open('/source/marker','w').write('bad')\nexcept OSError: denied=True\nassert denied\ns=socket.socket();s.settimeout(.5)\ntry:s.connect(('1.1.1.1',443));raise AssertionError('network access')\nexcept OSError:pass\n"
            try:
                result = Execution(store).operation(snapshot, store.put(b"test binary"),
                          dict(argv=["python3", "-c", script], inputs={}, timeout_seconds=5))
            except Refusal:
                for event in store.events():
                    if event["kind"] == "execution-retained":
                        record = store.load(event["payload"]["record"])
                        print(store.get(record["stderr"]).decode("utf-8", "replace"))
                raise
            self.assertTrue(result["passed"])
            self.assertEqual((source / "marker").read_text(), "unchanged")

    @unittest.skipUnless(os.environ.get("PACTRAIL_LAB_TEST_IMAGE") and os.environ.get("PACTRAIL_BENCH_BINARY"), "real engine qualification needs pinned local image and executable")
    def test_real_engine_gateway_parent_inspection_and_candidate_retention(self):
        from pactrail_lab.execution import Execution, supervisor_inputs
        from pactrail_lab.snapshots import import_tree, revision
        from pactrail_lab.safe import canonical, read
        def provider(_self, body):
            if any(m["role"] == "tool" for m in body["messages"]):
                message, finish = {"content": "Updated fixture; tests were not run."}, "stop"
            else:
                message, finish = {"content": "", "tool_calls": [{"id": "write-fixture", "type": "function", "function": {
                    "name": "write_file", "arguments": '{"path":"value.txt","content":"after"}'}}]}, "tool_calls"
            return canonical({"model": "offline-fixture", "choices": [{"message": message, "finish_reason": finish}],
                              "usage": {"prompt_tokens": 100, "completion_tokens": 10}})
        with tempfile.TemporaryDirectory() as tmp, Store(Path(tmp) / "campaign", create=True) as store:
            source = Path(tmp) / "source"
            source.mkdir()
            (source / "value.txt").write_text("before")
            snapshot = import_tree(store, source)
            baseline = revision(store, snapshot, read(os.environ["PACTRAIL_BENCH_BINARY"]), {"agent_mode": "single"}, [])
            model = dict(id="offline-fixture", endpoint="https://example.invalid/v1/chat/completions", api_key_env="LAB_FIXTURE_KEY",
                         context_tokens=32768, output_tokens=1024, input_rate=0, output_rate=0, temperature=0, reasoning_effort=None)
            with store.transaction():
                inputs = supervisor_inputs()
                for path in inputs: store.put(Path(path).read_bytes())
                store.set("supervisor-inputs", inputs)
                store.set("external-inputs", {})
                store.set("manifest", store.record({"image": os.environ["PACTRAIL_LAB_TEST_IMAGE"], "model": model,
                    "limits": dict(requests=12, cost_microusd=1, wall_seconds=60)}))
            with patch.dict(os.environ, LAB_FIXTURE_KEY="offline-fixture-not-a-real-key"), patch("pactrail_lab.gateway.Gateway._provider", provider):
                outcome = Execution(store).trial(baseline, snapshot, "Change value.txt to after.",
                    dict(model_turns=8, wall_seconds=40, context_tokens=32768, output_tokens=1024, model_tokens=262144), "offline-fixture")
            if outcome["status"] != "completed" or not all(outcome["checks"].values()):
                record = store.load(outcome["record"])
                print(store.get(record["stderr"]).decode("utf-8", "replace"))
                from pactrail_lab.archive import unpack
                unpack(store.get(record["stdout"]), Path(tmp) / "failed-output")
                for name in ("trusted-verification.json", "adapter.stderr", "run.stderr", "parent-trace.stderr", "parent-inspect.stderr", "parent-diff.stderr"):
                    path = Path(tmp) / "failed-output" / name
                    if path.exists(): print(name, path.read_text())
            self.assertEqual(outcome["status"], "completed")
            self.assertTrue(all(outcome["checks"].values()))
            changed = next(e for e in store.load(outcome["candidate_source"])["entries"] if e["path"] == "value.txt")
            self.assertEqual(store.get(changed["digest"]), b"after")
            self.assertEqual((source / "value.txt").read_text(), "before")
            self.assertEqual(len(budget.reservations(store)), 2)


if __name__ == "__main__": unittest.main()
