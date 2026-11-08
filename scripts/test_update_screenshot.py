"""Headless checks for screenshot readiness and publication."""

import argparse
import struct
import tempfile
import unittest
from pathlib import Path

from update_screenshot import (
    default_query,
    history_progress,
    positive_seconds,
    publish_capture,
    selection_ready,
)


class ScreenshotTests(unittest.TestCase):
    def test_history_requires_eof_and_completed_current_filter(self):
        prefix = (
            "gitk-rs: history generation=1 branch=main loaded=10 visible=2 request=4 "
        )
        log = prefix + "eof=true filtering=false regex=true query_bytes=2 error=false"
        progress = history_progress(log)
        self.assertTrue(progress.ready(10, True, "é"))
        self.assertFalse(progress.ready(11, True, "é"))
        self.assertFalse(progress.ready(10, False, "é"))
        self.assertFalse(progress.ready(10, True, "abc"))
        for old, new in [
            ("eof=true", "eof=false"),
            ("filtering=false", "filtering=true"),
            ("error=false", "error=true"),
        ]:
            self.assertFalse(
                history_progress(log + "\n" + log.replace(old, new)).ready(
                    10, True, "é"
                )
            )
        self.assertIsNone(history_progress("application starting"))

    def test_diff_waits_for_selected_commit_and_highlighting(self):
        selected = "gitk-rs: selection generation=1 commit=abc row=6\n"
        diff = "gitk-rs: diff commit=abc selection=2 request=3 highlighting=false error=false"
        self.assertEqual(selection_ready(selected + diff, 6), "abc")
        self.assertIsNone(selection_ready(selected + diff, 5))
        self.assertIsNone(
            selection_ready(selected + diff.replace("commit=abc", "commit=def"), 6)
        )
        self.assertIsNone(
            selection_ready(
                selected + diff.replace("highlighting=false", "highlighting=true"), 6
            )
        )
        with self.assertRaisesRegex(RuntimeError, "Selected diff failed"):
            selection_ready(selected + diff.replace("error=false", "error=true"), 6)

    def test_default_dates_cross_year_boundary(self):
        query = default_query("2026-01-01")
        self.assertIn("after:2025-12-26 before:2026-01-02", query)
        self.assertIn(r"path:\.nix$", query)
        self.assertIn("message:cargo|rust", query)

    def test_invalid_capture_preserves_existing_image(self):
        with tempfile.TemporaryDirectory() as directory:
            source, destination = (
                Path(directory) / "new.png",
                Path(directory) / "old.png",
            )
            destination.write_bytes(b"old screenshot")
            source.write_bytes(b"invalid")
            with self.assertRaises(RuntimeError):
                publish_capture(source, destination, (100, 50))
            self.assertEqual(destination.read_bytes(), b"old screenshot")
            header = (
                b"\x89PNG\r\n\x1a\n"
                + struct.pack(">I", 13)
                + b"IHDR"
                + struct.pack(">II", 100, 50)
            )
            source.write_bytes(header)
            with self.assertRaises(RuntimeError):
                publish_capture(source, destination, (200, 50))
            self.assertEqual(destination.read_bytes(), b"old screenshot")
            publish_capture(source, destination, (100, 50))
            self.assertEqual(destination.read_bytes(), header)

    def test_timeouts_reject_nonfinite_values(self):
        for value in ["nan", "inf", "-inf", "0", "-1"]:
            with self.assertRaises(argparse.ArgumentTypeError):
                positive_seconds(value)


if __name__ == "__main__":
    unittest.main()
