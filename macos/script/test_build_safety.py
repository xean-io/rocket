"""Static regression checks: no launch, process signaling, or network IO."""
from pathlib import Path
import importlib.util
from contextlib import redirect_stderr
import io
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("build_and_run.sh")
HELPER = Path(__file__).with_name("build_rocket.py")


class BuildSafetyTests(unittest.TestCase):
    def helper(self):
        spec = importlib.util.spec_from_file_location("build_rocket", HELPER)
        helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(helper)
        return helper
    def source(self):
        return SCRIPT.read_text() + (HELPER.read_text() if HELPER.exists() else "")

    def test_no_name_based_process_control(self):
        self.assertTrue("pkill" not in self.source(), "name-based signaling remains")
        self.assertTrue("pgrep" not in self.source(), "name-based verification remains")

    def test_gui_receives_explicit_environment(self):
        self.assertTrue(HELPER.exists(), "safe bundle helper is missing")
        helper = self.helper()
        with patch.object(helper.subprocess, "run") as run, patch.object(helper, "matching_processes", return_value=[{"pid": 1}]):
            helper.launch(Path("/test/Rocket Verify.app"), {"ROCKET_HOME": "/tmp/rkv-abc123", "ROCKET_BIN": "/repo/rocket"}, "light")
        command = run.call_args.args[0]
        self.assertEqual(command[:2], ["/usr/bin/open", "-n"])
        self.assertIn("ROCKET_HOME=/tmp/rkv-abc123", command)
        self.assertIn("ROCKET_BIN=/repo/rocket", command)
        self.assertEqual(command.count("--env"), 2)
        self.assertEqual(command[-3:], ["--args", "--verification-appearance", "light"])

    def test_verification_uses_separate_bundle_and_marker(self):
        self.assertIn("RocketVerificationBundle", self.source())
        self.assertIn('"verification"', self.source())
        self.assertIn("com.xean.rocket.verify", self.source())

    def test_cleanup_requires_process_receipt(self):
        self.assertIn("--keep-running", self.source())
        self.assertIn("--cleanup", self.source())
        self.assertIn("start_usec", self.source())
        self.assertIn("proc_pidpath", self.source())

    def test_log_filter_is_process_scoped(self):
        self.assertIn("processIdentifier ==", self.source())

    def test_reused_pid_cannot_receive_a_signal(self):
        helper = self.helper()
        expected = {"pid": 123, "executable": "/test/Rocket", "uid": helper.os.getuid(), "start_sec": 1, "start_usec": 2}
        for changed in ({**expected, "start_usec": 3}, {**expected, "executable": "/unrelated/process"},
                        {**expected, "uid": expected["uid"] + 1}):
            with patch.object(helper, "process_identity", return_value=changed), patch.object(helper.os, "kill") as kill:
                with self.assertRaises(helper.SafetyError):
                    helper.stop_owned(expected)
                kill.assert_not_called()

    def test_remaining_daemon_files_fail_cleanup_verification(self):
        helper = self.helper()
        with patch.object(Path, "exists", return_value=True):
            with self.assertRaises(helper.SafetyError):
                helper.wait_daemon_files_gone(Path("/tmp/rkv-abc123"), timeout=0)

    def test_running_jobs_prevent_daemon_cleanup(self):
        helper = self.helper()
        root = helper.BUILD / "verification"
        root.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="mock-", dir=root) as directory:
            bundle = Path(directory) / "Mock.app"
            daemon = {"pid": 123, "executable": str(bundle.parent / "rocket"), "uid": helper.os.getuid(), "start_sec": 1, "start_usec": 2}
            receipt_path = Path(directory) / "receipt.json"
            helper.save_receipt(receipt_path, {"bundle": str(bundle), "verification": True, "home": "/tmp/rkv-abc123",
                                               "app": None, "daemon": daemon})
            def response(_receipt, path, body=None):
                if path == "/v1/jobs?all=true":
                    return {"jobs": [{"id": "j", "status": "running"}]}
                return {}
            with patch.object(helper, "private_home"), patch.object(helper, "assert_identity", side_effect=lambda identity: identity), \
                 patch.object(helper, "stop_owned") as stop, patch.object(helper, "api", side_effect=response):
                with self.assertRaises(helper.SafetyError):
                    helper.cleanup(receipt_path)
                self.assertNotIn(unittest.mock.call(daemon), stop.call_args_list)

    def test_failure_before_daemon_receipt_preserves_original_error(self):
        helper = self.helper()
        root = helper.BUILD / "verification"
        root.mkdir(parents=True, exist_ok=True)
        args = SimpleNamespace(fixture=None, appearance=None, keep_running=False)
        with tempfile.TemporaryDirectory(prefix="mock-", dir=root) as directory:
            bundle = Path(directory) / "Mock.app"
            with patch.object(helper.tempfile, "mkdtemp", return_value="/tmp/rkv-abc123"), \
                 patch.object(helper, "private_home"), patch.object(helper.shutil, "copy2"), \
                 patch.dict(helper.os.environ, {"ROCKET_BIN": "/test/rocket"}), \
                 patch.object(helper.subprocess, "run", side_effect=RuntimeError("original startup failure")), \
                 patch.object(helper, "cleanup", side_effect=helper.SafetyError("secondary cleanup failure")) as cleanup, \
                 redirect_stderr(io.StringIO()) as output:
                with self.assertRaisesRegex(RuntimeError, "original startup failure"):
                    helper.verify(args, bundle)
                cleanup.assert_called_once_with(bundle.parent / "receipt.json")
                self.assertIn("secondary cleanup failure", output.getvalue())


if __name__ == "__main__":
    unittest.main()
