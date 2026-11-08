#!/usr/bin/env python3
"""Capture a regex-filtered repository after loading its complete history (X11)."""

import argparse
import datetime
import json
import math
import os
import re
import shutil
import struct
import subprocess
import tempfile
import time
import tomllib
from dataclasses import dataclass
from pathlib import Path

from gui_smoke import Window, find_window
from native_tools import X11, application, require_commands, wait_until

HISTORY = re.compile(
    r"gitk-rs: history generation=\d+ branch=\S+ loaded=(\d+) visible=(\d+) "
    r"request=\d+ eof=(true|false) filtering=(true|false) regex=(true|false) "
    r"query_bytes=(\d+) error=(true|false)"
)
SELECTION = re.compile(r"gitk-rs: selection generation=\d+ commit=(\w+) row=(\d+)")
DIFF = re.compile(
    r"gitk-rs: diff commit=(\w+) selection=\d+ request=\d+ "
    r"highlighting=(true|false) error=(true|false)"
)


@dataclass(frozen=True)
class Progress:
    loaded: int
    visible: int
    eof: bool
    filtering: bool
    regex: bool
    query_bytes: int
    error: bool

    def ready(self, total: int, regex: bool, query: str) -> bool:
        return (
            self.loaded == total
            and self.eof
            and not self.filtering
            and self.regex == regex
            and self.query_bytes == len(query.encode())
            and not self.error
        )


def history_progress(log: str) -> Progress | None:
    matches = list(HISTORY.finditer(log))
    if not matches:
        return None
    loaded, visible, eof, filtering, regex, size, error = matches[-1].groups()
    return Progress(
        int(loaded),
        int(visible),
        eof == "true",
        filtering == "true",
        regex == "true",
        int(size),
        error == "true",
    )


def selection_ready(log: str, row: int) -> str | None:
    selections = list(SELECTION.finditer(log))
    diffs = list(DIFF.finditer(log))
    if not selections or not diffs:
        return None
    commit, selected_row = selections[-1].groups()
    diff_commit, highlighting, error = diffs[-1].groups()
    if int(selected_row) == row and diff_commit == commit:
        if error == "true":
            raise RuntimeError("Selected diff failed; inspect application.log")
        if highlighting == "false":
            return commit
    return None


def default_query(head_date: str) -> str:
    day = datetime.date.fromisoformat(head_date)
    after = day - datetime.timedelta(days=6)
    before = day + datetime.timedelta(days=1)
    return (
        rf"message:cargo|rust path:\.nix$ after:{after} before:{before} "
        "committer:GitHub merge:true"
    )


def positive_seconds(value: str) -> float:
    result = float(value)
    if not math.isfinite(result) or result <= 0:
        raise argparse.ArgumentTypeError("must be a finite positive number")
    return result


def nonnegative(value: str) -> int:
    result = int(value)
    if result < 0:
        raise argparse.ArgumentTypeError("must be nonnegative")
    return result


def size(value: str) -> tuple[int, int]:
    match = re.fullmatch(r"([1-9]\d*)x([1-9]\d*)", value)
    if not match:
        raise argparse.ArgumentTypeError("expected WIDTHxHEIGHT, e.g. 1920x1275")
    return int(match[1]), int(match[2])


def publish_capture(source: Path, destination: Path, expected: tuple[int, int]) -> None:
    with source.open("rb") as image:
        header = image.read(24)
    if (
        len(header) != 24
        or header[:8] != b"\x89PNG\r\n\x1a\n"
        or header[12:16] != b"IHDR"
        or struct.unpack(">II", header[16:24]) != expected
    ):
        raise RuntimeError(
            "Capture is not a PNG of the requested size; old image preserved"
        )
    destination.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(
        prefix=f".{destination.name}.", suffix=".png", dir=destination.parent
    )
    os.close(descriptor)
    try:
        shutil.copy2(source, temporary)
        os.replace(temporary, destination)
    finally:
        Path(temporary).unlink(missing_ok=True)


def capture(options: argparse.Namespace, artifacts: Path) -> None:
    require_commands("git", "xdotool", "xwininfo", "import")
    repository = options.repository.expanduser().resolve()
    binary = options.binary.expanduser().resolve()
    output = options.output.expanduser().resolve()
    env = os.environ.copy()
    env.pop("WAYLAND_DISPLAY", None)
    env["GIT_OPTIONAL_LOCKS"] = "0"
    env["GPUI_X11_SCALE_FACTOR"] = str(options.scale)

    def git(*args: str) -> str:
        return subprocess.check_output(
            ["git", "-C", str(repository), *args], env=env, text=True, timeout=30
        ).strip()

    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise RuntimeError(
            f"Build the application first; executable not found: {binary}"
        )
    if git("status", "--porcelain"):
        raise RuntimeError("Use a clean repository for the commit screenshot")
    head = git("rev-parse", "HEAD")
    total = int(git("rev-list", "--count", head))
    if not total:
        raise RuntimeError("The repository has no commits")
    query = options.query or default_query(git("show", "-s", "--format=%as", head))
    if not query.strip():
        raise RuntimeError("The screenshot query must not be empty")
    config = artifacts / "config" / "gitk-rs"
    config.mkdir(parents=True)
    (config / "preferences.toml").write_text("font_size = 13.0\nwatch = false\n")
    (config / "state.toml").write_text(
        "[search]\nregex = false\n[layout]\ngraph_width = 250.0\n"
        "history_height = 245.0\nfile_list_width = 290.0\n[diff]\n"
        'version = "diff"\ncontext = 3\nignore_space = true\nwords = "lines"\n'
    )
    env["XDG_CONFIG_HOME"] = str(artifacts / "config")
    log = artifacts / "application.log"
    print(f"HEAD {head}; waiting for all {total:,} commits", flush=True)
    args = [str(binary), "--no-limit", "--nowatch", "--verbose", str(repository)]
    with application(args, env, log) as app:

        def text() -> str:
            if app.poll() is not None:
                raise RuntimeError("Application exited early; inspect application.log")
            return log.read_text()

        window = None

        def window_ready() -> bool:
            nonlocal window
            text()
            try:
                window = find_window(app.pid, env)
                return True
            except subprocess.CalledProcessError:
                return False

        wait_until(
            window_ready, "No owned application window appeared", options.timeout
        )
        ui = Window(window, env, artifacts)
        ui.run("windowsize", "--sync", window, *(str(n) for n in options.size))

        def wait_history(regex: bool, query: str) -> Progress:
            last_report = 0.0
            latest = None

            def ready() -> bool:
                nonlocal last_report, latest
                current = text()
                latest = history_progress(current)
                if latest is None:
                    if "gitk-rs: history generation=" in current:
                        raise RuntimeError(
                            "Binary lacks progress fields; rebuild this checkout"
                        )
                    return False
                if latest.error:
                    raise RuntimeError("History/filter failed; inspect application.log")
                if time.monotonic() - last_report >= 10:
                    print(
                        f"Loaded {latest.loaded:,}/{total:,}; matches {latest.visible:,}",
                        flush=True,
                    )
                    last_report = time.monotonic()
                return latest.ready(total, regex, query)

            wait_until(ready, "History/filter did not finish", options.timeout)
            return latest

        initial = wait_history(False, "")
        if initial.visible != total:
            raise RuntimeError("Initial unfiltered history count does not match Git")
        print("EOF reached before applying the filter", flush=True)
        ui.run("windowfocus", "--sync", window)
        ui.keys("slash", "alt+r")
        ui.type_text(query, delay=5)
        filtered = wait_history(True, query)
        if not filtered.visible:
            raise RuntimeError("No matches; provide a different --query")
        row = min(options.row, filtered.visible - 1)
        ui.keys("Escape", "Home")
        for target in range(row + 1):
            if target:
                ui.keys("alt+j")
            wait_until(
                lambda target=target: selection_ready(text(), target) is not None,
                f"Diff/highlighting for row {target} did not finish",
                options.timeout,
            )
        ui.keys("ctrl+Prior")
        ui.run(
            "mousemove",
            "--window",
            window,
            str(options.size[0] - 4),
            str(options.size[1] - 4),
        )
        time.sleep(1)  # Allow the ready model's next paint and hover dismissal.
        if git("rev-parse", "HEAD") != head:
            raise RuntimeError(
                "Repository HEAD changed during capture; rerun the script"
            )
        if find_window(app.pid, env) != window:
            raise RuntimeError("Application window changed; refusing to capture")
        ui.shot("screenshot")
        selected = selection_ready(text(), row)
        state = tomllib.loads((config / "state.toml").read_text())
        if not state["search"]["regex"] or state["diff"]["words"] != "lines":
            raise RuntimeError("Expected regex and line-diff state were not retained")
        with X11() as x11:
            x11.close_window(window)
        app.wait(timeout=15)
        if app.returncode:
            raise RuntimeError(
                "Application failed during shutdown; old image preserved"
            )
    metadata = {
        "repository": str(repository),
        "head": head,
        "loaded": total,
        "matches": filtered.visible,
        "query": query,
        "row": row,
        "selected": selected,
        "regex": True,
        "diff": "lines",
        "size": options.size,
    }
    (artifacts / "capture.json").write_text(json.dumps(metadata, indent=2) + "\n")
    publish_capture(artifacts / "screenshot.png", output, options.size)
    print(f"Saved {output} ({filtered.visible:,} matches / {total:,} loaded)")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "repository", type=Path, help="clean nixpkgs checkout, read only"
    )
    parser.add_argument("--binary", type=Path, default=Path("target/debug/gitk-rs"))
    parser.add_argument("--output", type=Path, default=Path("screenshot.png"))
    parser.add_argument(
        "--query", help="regex query; default uses cargo/rust and HEAD-relative dates"
    )
    parser.add_argument(
        "--row", type=nonnegative, default=6, help="zero-based result row to select"
    )
    parser.add_argument(
        "--size", type=size, default=(1920, 1275), metavar="WIDTHxHEIGHT"
    )
    parser.add_argument(
        "--scale", type=positive_seconds, default=1.5, help="X11 scale factor"
    )
    parser.add_argument(
        "--timeout", type=positive_seconds, default=300, help="seconds per stage"
    )
    parser.add_argument(
        "--artifacts", type=Path, help="directory for logs and isolated configuration"
    )
    options = parser.parse_args()
    artifacts = (
        options.artifacts.expanduser().resolve()
        if options.artifacts
        else Path(tempfile.mkdtemp(prefix="gitk-rs-screenshot-", dir="/tmp"))
    )
    artifacts.mkdir(parents=True, exist_ok=True)
    print(f"Artifacts: {artifacts}", flush=True)
    try:
        capture(options, artifacts)
    except (RuntimeError, OSError, subprocess.SubprocessError, ValueError) as error:
        parser.exit(1, f"Capture failed: {error}\nLogs/configuration: {artifacts}\n")


if __name__ == "__main__":
    main()
