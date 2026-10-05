import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).parent))
import container_grade
import parse_upstream


class ContainerBoundaryTests(unittest.TestCase):
    def test_control_success_error_output_limit_and_timeout(self):
        env = {"PATH": os.environ["PATH"]}
        self.assertEqual(container_grade.control([sys.executable, "-c", "print('measured')"], env), b"measured\n")
        with self.assertRaisesRegex(RuntimeError, "fixture error"):
            container_grade.control([sys.executable, "-c", "import sys; sys.stderr.write('fixture error'); sys.exit(1)"], env)
        with mock.patch.object(container_grade, "MAX_LOG_BYTES", 100):
            with self.assertRaisesRegex(ValueError, "bound"):
                container_grade.control([sys.executable, "-c", "print('x'*1000)"], env)
        with self.assertRaises(subprocess.TimeoutExpired):
            container_grade.control([sys.executable, "-c", "import time; time.sleep(10)"], env, timeout=0.1)

    def test_ledger_size_and_shape_are_bounded(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "ledger.json"
            for payload in ("{}", "[]", "[null]", "[{}]" * 100):
                path.write_text(payload)
                with self.assertRaises(ValueError):
                    container_grade.bounded_list(path)

    def test_only_one_complete_test_section_is_parsed(self):
        start, end = ">>>>> Start Test Output", ">>>>> End Test Output"
        self.assertEqual(parse_upstream.test_section("setup " + start + "results" + end + "cleanup"), "results")
        for text in ("", start, end + start, start + end + start):
            with self.assertRaises(ValueError):
                parse_upstream.test_section(text)

    def test_wrong_upstream_installation_is_rejected_before_import_or_log_access(self):
        distribution = mock.Mock()
        distribution.read_text.return_value = '{"vcs_info":{"commit_id":"wrong"}}'
        with mock.patch.object(parse_upstream.importlib.metadata, "distribution", return_value=distribution):
            with self.assertRaisesRegex(ValueError, "upstream commit"):
                parse_upstream.parse(Path("missing"), Path("missing"), "fixture")


if __name__ == "__main__":
    unittest.main()
