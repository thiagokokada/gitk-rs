#!/usr/bin/env python3
"""Exercise a real X11 window with disposable Git repositories and preferences.

Requires git, xdotool, xwininfo, ImageMagick import and libX11. Run in an
environment with the application's native runtime libraries available.
"""

import argparse
import os
import re
import subprocess
import tempfile
import time
from pathlib import Path

from native_tools import X11, application, require_commands, wait_until


def git_at(repository: Path, *arguments: str) -> str:
    return subprocess.check_output(
        [
            "git",
            "-C",
            str(repository),
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            *arguments,
        ],
        text=True,
    ).strip()


def create_fixture(repository: Path, template: Path) -> str:
    repository.mkdir()
    template.mkdir()

    def git(*arguments: str) -> str:
        return git_at(repository, *arguments)

    git("init", "--template=" + str(template), "-q", "-b", "main")
    (repository / "main.rs").write_text('fn main() { println!("base"); }\n')
    git("add", ".")
    git("commit", "-qm", "root")
    root_hash = git("rev-parse", "HEAD")
    git("checkout", "-qb", "side")
    (repository / "main.rs").write_text('fn main() { println!("side"); }\n')
    git("add", ".")
    git("commit", "-qm", "side")
    git("checkout", "-q", "main")
    (repository / "main.rs").write_text('fn main() { println!("unsaved"); }\n')
    (repository / "new.go").write_text("package main\nfunc main() { println(42) }\n")
    git("add", "new.go")
    (repository / "flake.nix").write_text(
        "{ pkgs }: { answer = 42; package = pkgs.hello; }\n"
    )
    return root_hash


class Window:
    """Send input only while our application owns the focused window."""

    def __init__(self, window: str, env: dict[str, str], artifacts: Path):
        self.window = window
        self.env = env
        self.artifacts = artifacts

    def run(self, *arguments: str) -> None:
        subprocess.run(["xdotool", *arguments], env=self.env, timeout=5, check=True)

    def assert_focus(self) -> None:
        focused = subprocess.check_output(
            ["xdotool", "getwindowfocus"], env=self.env, text=True, timeout=5
        ).strip()
        if int(focused, 0) != int(self.window, 0):
            raise RuntimeError(
                "Test window lost focus; refusing to send input elsewhere"
            )

    def keys(self, *values: str) -> None:
        self.assert_focus()
        self.run("key", "--clearmodifiers", *values)

    def type_text(self, text: str, delay: int = 20) -> None:
        self.assert_focus()
        self.run("type", "--clearmodifiers", "--delay", str(delay), text)

    def shot(self, name: str) -> None:
        subprocess.run(
            ["import", "-window", self.window, str(self.artifacts / f"{name}.png")],
            env=self.env,
            timeout=10,
            check=True,
        )


def find_window(pid: int, env: dict[str, str]) -> str:
    found = subprocess.check_output(
        ["xdotool", "search", "--onlyvisible", "--pid", str(pid)],
        env=env,
        text=True,
        timeout=5,
    )
    window = found.splitlines()[0]
    owner = subprocess.check_output(
        ["xdotool", "getwindowpid", window], env=env, text=True, timeout=5
    ).strip()
    if owner != str(pid):
        raise RuntimeError("Refusing to use a window not owned by the test application")
    return window


def exercise_workflows(
    ui: Window,
    repository: Path,
    template: Path,
    root_hash: str,
    log: Path,
    config: Path,
) -> None:
    def git(*arguments: str) -> str:
        return git_at(repository, *arguments)

    ui.keys("Home")
    time.sleep(1)
    ui.shot("local-nix")
    ui.keys("Down")
    time.sleep(0.5)
    ui.shot("staged-go")
    ui.keys("Down")
    time.sleep(0.5)
    ui.shot("commit-rust")
    ui.keys("F1")
    time.sleep(0.2)
    ui.shot("help")
    ui.keys("Escape", "ctrl+b")
    time.sleep(0.5)
    ui.shot("branches")
    ui.keys("Down", "Return")
    time.sleep(0.7)
    ui.shot("branch-protected")
    assert git("branch", "--show-current") == "main"
    assert "unsaved" in (repository / "main.rs").read_text()
    ui.keys("Escape")
    (repository / "main.rs").write_text('fn main() { println!("base"); }\n')
    ui.keys("ctrl+b")
    time.sleep(0.5)
    ui.type_text("side")
    time.sleep(0.2)
    ui.shot("branch-filtered")
    ui.keys("Return")
    time.sleep(1)
    assert git("branch", "--show-current") == "side"

    def ancestor_selected() -> bool:
        text = log.read_text()
        generation = re.search(r"history generation=(\d+) branch=side", text)
        return bool(
            generation
            and f"selection generation={generation[1]} commit={root_hash}" in text
        )

    wait_until(
        ancestor_selected,
        "Branch switch did not preserve the selected ancestor commit",
    )
    ui.shot("branch-switched")
    exercise_preferences(ui, config)
    open_alternate_repository(ui, repository.parent, template, log)


def exercise_preferences(ui: Window, config: Path) -> None:
    ui.keys("alt+r")
    time.sleep(0.2)
    ui.shot("watch-disabled")
    ui.keys("ctrl+comma")
    time.sleep(0.2)
    ui.shot("preferences")
    ui.keys("ctrl+Return")
    time.sleep(0.4)
    assert (config / "gitk-rs/preferences.json").exists()
    ui.keys("alt+t")
    time.sleep(0.4)
    ui.shot("light-theme")
    ui.keys("alt+t", "slash")
    ui.type_text("root")
    time.sleep(0.3)
    ui.shot("workflow-filter")
    ui.keys("ctrl+e", "ctrl+u", "Escape", "F5")
    time.sleep(0.6)
    ui.shot("workflow-reloaded")


def open_alternate_repository(
    ui: Window, root: Path, template: Path, log: Path
) -> None:
    alternate = root / "alternate"
    alternate.mkdir()
    git_at(alternate, "init", "--template=" + str(template), "-q", "-b", "alternate")
    (alternate / "README.md").write_text("# Alternate repository\n")
    git_at(alternate, "add", ".")
    git_at(alternate, "commit", "-qm", "alternate repository")
    ui.keys("ctrl+shift+o")
    time.sleep(0.2)
    ui.shot("repository-path-dialog")
    ui.keys("ctrl+a", "ctrl+k")
    ui.type_text(str(alternate), delay=5)
    ui.keys("Return")
    wait_until(
        lambda: "branch=alternate" in log.read_text(),
        "Alternate repository did not load; see " + str(log),
    )
    time.sleep(0.3)
    ui.shot("repository-opened")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", nargs="?", default="target/debug/gitk-rs")
    parser.add_argument(
        "output_directory", type=Path, nargs="?", default=Path("/tmp/gitk-rs-gui-smoke")
    )
    options = parser.parse_args()
    require_commands("git", "xdotool", "xwininfo", "import")
    artifacts = options.output_directory
    artifacts.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="gitk-rs-ui-fixture-") as directory:
        root = Path(directory)
        repository = root / "repo"
        template = root / "template"
        config = root / "config"
        root_hash = create_fixture(repository, template)
        env = os.environ.copy()
        env.pop("WAYLAND_DISPLAY", None)
        env.update(
            GIT_CONFIG_NOSYSTEM="1",
            GIT_CONFIG_GLOBAL="/dev/null",
            XDG_CONFIG_HOME=str(config),
        )
        log = artifacts / "application.log"
        arguments = [options.binary, "--timings", "--verbose", str(repository)]
        with application(arguments, env, log) as app:

            def history_ready() -> bool:
                if app.poll() is not None:
                    raise RuntimeError(log.read_text())
                return "phase=HistoryPainted" in log.read_text()

            wait_until(
                history_ready, "History did not paint; see " + str(log), timeout=10
            )
            ui = Window(find_window(app.pid, env), env, artifacts)
            ui.shot("history")
            ui.run("windowfocus", "--sync", ui.window)
            time.sleep(1)
            exercise_workflows(ui, repository, template, root_hash, log, config)
            with X11() as x11:
                x11.close_window(ui.window)
            app.wait(timeout=10)
            if app.returncode != 0:
                raise RuntimeError(log.read_text())
        print(log.read_text())
        print(f"GUI smoke passed; own-window artifacts: {artifacts}")


if __name__ == "__main__":
    main()
