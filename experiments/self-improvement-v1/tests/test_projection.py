from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from pactrail_lab.safe import Refusal
from pactrail_lab.snapshots import import_tree, project_verified_changes
from pactrail_lab.store import Store


class CandidateProjection(unittest.TestCase):
    def test_ignored_files_survive_and_unreceipted_edits_do_not_land(self):
        with tempfile.TemporaryDirectory() as tmp, Store(Path(tmp) / "store", create=True) as store:
            base, raw = Path(tmp) / "base", Path(tmp) / "raw"
            base.mkdir(); raw.mkdir()
            (base / "package-lock.json").write_text("original lock")
            (base / "value").write_text("before")
            (base / "untouched").write_text("keep")
            (raw / "value").write_text("after")
            (raw / "untouched").write_text("unreceipted")
            before, candidate = import_tree(store, base), import_tree(store, raw)
            final = project_verified_changes(store, before, candidate,
                [{"path": "value", "before_digest": "verified-before", "after_digest": "verified-after"}])
            entries = {e["path"]: e for e in store.load(final)["entries"]}
            self.assertEqual(store.get(entries["package-lock.json"]["digest"]), b"original lock")
            self.assertEqual(store.get(entries["untouched"]["digest"]), b"keep")
            self.assertEqual(store.get(entries["value"]["digest"]), b"after")

    def test_explicit_deletion_and_addition_are_preserved(self):
        with tempfile.TemporaryDirectory() as tmp, Store(Path(tmp) / "store", create=True) as store:
            base, raw = Path(tmp) / "base", Path(tmp) / "raw"
            base.mkdir(); raw.mkdir()
            (base / "removed").write_text("before")
            (raw / "added").write_text("after")
            result = project_verified_changes(store, import_tree(store, base), import_tree(store, raw), [
                {"path": "removed", "before_digest": "before", "after_digest": None},
                {"path": "added", "before_digest": None, "after_digest": "after"}])
            self.assertEqual([e["path"] for e in store.load(result)["entries"]], ["added"])

    def test_unsafe_duplicate_and_missing_changes_fail_closed(self):
        with tempfile.TemporaryDirectory() as tmp, Store(Path(tmp) / "store", create=True) as store:
            base = Path(tmp) / "base"; base.mkdir()
            snapshot = import_tree(store, base)
            for changes in ([{"path": "../escape", "before_digest": None, "after_digest": "x"}],
                            [{"path": "missing", "before_digest": None, "after_digest": "x"}],
                            [{"path": "missing", "before_digest": "x", "after_digest": None}]):
                with self.assertRaises(Refusal): project_verified_changes(store, snapshot, snapshot, changes)
            (base / "value").write_text("same")
            snapshot = import_tree(store, base)
            change = {"path": "value", "before_digest": "before", "after_digest": "after"}
            with self.assertRaisesRegex(Refusal, "duplicate"):
                project_verified_changes(store, snapshot, snapshot, [change, change])
