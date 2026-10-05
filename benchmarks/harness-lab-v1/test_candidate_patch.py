import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import candidate_patch
from container_grade import control


class CandidatePatchTests(unittest.TestCase):
    def test_add_delete_ignored_and_binary_changes_are_included_without_source_writes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            baseline, candidate, applied = root / "base", root / "candidate", root / "applied"
            for path in (baseline, candidate, applied):
                path.mkdir()
            (baseline / "gone.txt").write_bytes(b"old\r\n")
            (baseline / ".gitignore").write_text("ignored.txt\n")
            (baseline / ".gitattributes").write_text("* text eol=lf\n")
            (candidate / ".gitignore").write_text("ignored.txt\n")
            (candidate / ".gitattributes").write_text("* text eol=lf\n")
            (candidate / "ignored.txt").write_bytes(b"new\r\n")
            (candidate / "café.rs").write_bytes(b"unicode path\r\n")
            (candidate / "binary.bin").write_bytes(b"\x00\x01\xff")
            patch = candidate_patch.build(baseline, candidate, control)
            self.assertIn(b"ignored.txt", patch)
            self.assertIn(b"gone.txt", patch)
            self.assertIn(b"GIT binary patch", patch)
            self.assertFalse((baseline / ".git").exists())
            self.assertFalse((candidate / ".git").exists())
            (applied / "gone.txt").write_bytes(b"old\r\n")
            (applied / ".gitignore").write_text("ignored.txt\n")
            (applied / ".gitattributes").write_text("* text eol=lf\n")
            path = root / "patch"
            path.write_bytes(patch)
            control(["git", "-C", str(applied), "apply", str(path)], {"PATH": os.environ["PATH"]})
            self.assertFalse((applied / "gone.txt").exists())
            self.assertEqual((applied / "ignored.txt").read_bytes(), b"new\r\n")
            self.assertEqual((applied / "binary.bin").read_bytes(), b"\x00\x01\xff")
            self.assertEqual((applied / "café.rs").read_bytes(), b"unicode path\r\n")

    def test_identity_detects_new_empty_ignored_files_and_changed_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / ".gitignore").write_text("ignored.txt\n")
            initial = candidate_patch.identity(root)
            (root / "ignored.txt").write_bytes(b"")
            self.assertNotEqual(candidate_patch.identity(root), initial)
            empty = candidate_patch.identity(root)
            (root / "ignored.txt").write_bytes(b"changed")
            self.assertNotEqual(candidate_patch.identity(root), empty)

    def test_file_and_byte_limits_reject_before_staging(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "value").write_bytes(b"data")
            for name in ("MAX_FILES", "MAX_BYTES"):
                with mock.patch.object(candidate_patch, name, 0):
                    with self.assertRaisesRegex(ValueError, "bound"):
                        candidate_patch.files(root)

    def test_empty_directory_trees_are_bounded_and_metadata_is_not_traversed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "one" / "two").mkdir(parents=True)
            for name, limit in (("MAX_ENTRIES", 1), ("MAX_DEPTH", 1)):
                with mock.patch.object(candidate_patch, name, limit):
                    with self.assertRaisesRegex(ValueError, "bound"):
                        candidate_patch.files(root)
            (root / ".git" / "ignored").mkdir(parents=True)
            self.assertEqual(candidate_patch.files(root), [])

    def test_file_root_and_nested_git_metadata_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "file").write_text("data")
            with self.assertRaisesRegex(ValueError, "directory"):
                candidate_patch.files(root / "file")
            (root / "nested" / ".git").mkdir(parents=True)
            with self.assertRaisesRegex(ValueError, "nested Git"):
                candidate_patch.files(root)


if __name__ == "__main__":
    unittest.main()
