import io
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from pactrail_lab.archive import pack, unpack
from pactrail_lab.judge import decide, paired
from pactrail_lab.safe import Refusal


class JudgeTests(unittest.TestCase):
    def rows(self, candidate=True):
        return [dict(task=str(t), arm=a, repeat=r, strict_completion=candidate if a == "candidate" else False,
                     status="completed") for t in range(8) for a in ("parent", "candidate", "restored") for r in range(3)]

    def test_causal_gain_fluke_missing_duplicate_and_infrastructure_failure(self):
        protocol = dict(tasks=[dict(id=str(i)) for i in range(8)], repetitions=3, seed=41)
        self.assertEqual(decide(self.rows(), protocol, {"gate": True})["verdict"], "supported")
        self.assertEqual(decide(self.rows(False), protocol, {"gate": True})["verdict"], "inconclusive")
        with self.assertRaises(Refusal): decide(self.rows()[1:], protocol, {"gate": True})
        with self.assertRaises(Refusal): decide(self.rows() + [self.rows()[0]], protocol, {"gate": True})
        rows = self.rows()
        rows[0]["status"] = "infrastructure-failed"
        self.assertNotEqual(decide(rows, protocol, {"gate": True})["verdict"], "supported")
        self.assertEqual(decide(self.rows(), protocol, {"gate": False})["verdict"], "rejected")

    def test_repetitions_are_not_independent_tasks(self):
        rows = [dict(task="one", arm=a, repeat=r, strict_completion=a == "candidate")
                for a in ("parent", "candidate") for r in range(10)]
        self.assertEqual(paired(rows, ["one"], 10, "parent", "candidate")["p_one_sided"], .5)

    def test_archive_roundtrip_and_hostile_members(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "source"
            root.mkdir()
            (root / ("é" * 80)).write_text("hello\r\n")
            unpack(pack(root), Path(tmp) / "restored")
            self.assertEqual((Path(tmp) / "restored" / ("é" * 80)).read_bytes(), b"hello\r\n")
            for name, kind in (("../escape", tarfile.REGTYPE), ("link", tarfile.SYMTYPE), ("device", tarfile.CHRTYPE)):
                buffer = io.BytesIO()
                with tarfile.open(fileobj=buffer, mode="w") as archive:
                    info = tarfile.TarInfo(name)
                    info.type = kind
                    archive.addfile(info)
                with self.assertRaises(Refusal): unpack(buffer.getvalue(), Path(tmp) / ("bad-" + kind.decode()))


if __name__ == "__main__": unittest.main()
