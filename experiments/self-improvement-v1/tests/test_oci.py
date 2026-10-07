import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from pactrail_lab.process import Oci
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


if __name__ == "__main__": unittest.main()
