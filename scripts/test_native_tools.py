"""Headless regression checks for native-script safety and process sampling."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from gui_smoke import Window
from native_tools import application
from release_probe import children, memory


class NativeToolsTests(unittest.TestCase):
    def test_lost_focus_prevents_keyboard_input(self):
        window = Window("42", {}, Path("/unused"))
        with (
            patch("gui_smoke.subprocess.check_output", return_value="99\n"),
            patch("gui_smoke.subprocess.run") as run,
        ):
            with self.assertRaisesRegex(RuntimeError, "lost focus"):
                window.keys("Return")
            with self.assertRaisesRegex(RuntimeError, "lost focus"):
                window.type_text("example")
            run.assert_not_called()

    def test_own_focused_window_accepts_input(self):
        window = Window("0x2a", {}, Path("/unused"))
        with (
            patch("gui_smoke.subprocess.check_output", return_value="42\n"),
            patch("gui_smoke.subprocess.run") as run,
        ):
            window.keys("Return")
            run.assert_called_once_with(
                ["xdotool", "key", "--clearmodifiers", "Return"],
                env={},
                timeout=5,
                check=True,
            )

    def test_application_is_reaped_when_workflow_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "application.log"
            with self.assertRaisesRegex(RuntimeError, "workflow failed"):
                with application(["sleep", "30"], os.environ.copy(), log) as process:
                    raise RuntimeError("workflow failed")
            self.assertIsNotNone(process.returncode)
            with self.assertRaises(ProcessLookupError):
                os.kill(process.pid, 0)

    def test_application_launch_failure_is_reported(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(FileNotFoundError):
                with application(
                    ["/nonexistent/gitk-rs-test-binary"],
                    {},
                    Path(directory) / "application.log",
                ):
                    self.fail("Missing binary was launched")

    def test_memory_sample_handles_process_disappearance(self):
        with patch("pathlib.Path.read_text", side_effect=FileNotFoundError):
            self.assertEqual(memory(42), {"VmRSS": 0, "VmHWM": 0})

    def test_memory_sample_parses_kib_fields(self):
        with patch(
            "pathlib.Path.read_text",
            return_value="Name:\tfixture\nVmHWM:\t2048 kB\nVmRSS:\t1024 kB\n",
        ):
            self.assertEqual(memory(42), {"VmRSS": 1024, "VmHWM": 2048})

    def test_child_sampling_includes_children_of_other_threads(self):
        with subprocess.Popen(["sleep", "30"]) as child:
            try:
                self.assertIn(child.pid, children(os.getpid()))
            finally:
                child.terminate()
                child.wait()


if __name__ == "__main__":
    unittest.main()
