"""Headless regression checks for native-script focus guards and cleanup."""

import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from gui_smoke import Window
from native_tools import application


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


if __name__ == "__main__":
    unittest.main()
