#!/usr/bin/env python3
"""Own-window X11 smoke test using disposable Git repositories and preferences.

Requires Python 3, git, xdotool, xwininfo and ImageMagick's import. Run within
nix develop so ctypes can load libX11. See docs/VALIDATION.md.
"""
import ctypes as C
import os
from pathlib import Path
import re
import subprocess
import time
import sys
import shutil
import signal

import tempfile
for command in ["git", "xdotool", "xwininfo", "import"]:
    if shutil.which(command) is None:
        raise SystemExit(f"Missing {command}; see docs/VALIDATION.md")
artifacts = Path(sys.argv[2] if len(sys.argv) > 2 else "/tmp/gitk-rs-gui-smoke")
artifacts.mkdir(parents=True, exist_ok=True)
fixture = tempfile.TemporaryDirectory(prefix="gitk-rs-ui-fixture-")
repository = Path(fixture.name) / "repo"
repository.mkdir()
template = Path(fixture.name) / "template"
template.mkdir()
def git(*args):
    return subprocess.check_output(["git", "-C", str(repository), "-c", "user.name=Fixture", "-c", "user.email=fixture@example.test", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", *args], text=True).strip()
git("init", "--template=" + str(template), "-q", "-b", "main")
(repository / "main.rs").write_text("fn main() { println!(\"base\"); }\n")
git("add", "."); git("commit", "-qm", "root")
root_hash = git("rev-parse", "HEAD")
git("checkout", "-qb", "side")
(repository / "main.rs").write_text("fn main() { println!(\"side\"); }\n")
git("add", "."); git("commit", "-qm", "side")
git("checkout", "-q", "main")
(repository / "main.rs").write_text("fn main() { println!(\"unsaved\"); }\n")
(repository / "new.go").write_text("package main\nfunc main() { println(42) }\n")
git("add", "new.go")
(repository / "flake.nix").write_text("{ pkgs }: { answer = 42; package = pkgs.hello; }\n")
env = os.environ.copy()
env.pop("WAYLAND_DISPLAY", None)
env["GIT_CONFIG_NOSYSTEM"] = "1"
env["GIT_CONFIG_GLOBAL"] = "/dev/null"
env["XDG_CONFIG_HOME"] = str(Path(fixture.name) / "config")
log = artifacts / "application.log"
with log.open("w") as output:
    app = subprocess.Popen([sys.argv[1] if len(sys.argv) > 1 else "target/debug/gitk-rs", "--timings", "--verbose"] + [str(repository)], env=env, stderr=output, start_new_session=True)
    try:
        deadline = time.monotonic() + 10
        while "phase=HistoryPainted" not in log.read_text():
            if app.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError(log.read_text())
            time.sleep(0.05)
        tool = shutil.which("xdotool")
        result = subprocess.run(["xwininfo", "-name", "gitk-rs"], env=env, capture_output=True, text=True, check=True)
        window = int(re.search(r"Window id: (0x[0-9a-f]+)", result.stdout)[1], 16)
        subprocess.run(["import", "-window", hex(window), str(artifacts / "history.png")], env=env, timeout=10, check=True)
        owner = subprocess.check_output([tool, "getwindowpid", hex(window)], env=env, text=True).strip()
        if owner != str(app.pid): raise RuntimeError("Refusing to use a window not owned by the test application")
        def assert_focus():
            focused = int(subprocess.check_output([tool, "getwindowfocus"], env=env, text=True).strip())
            if focused != window: raise RuntimeError("Test window lost focus; refusing to send input elsewhere")
        def keys(*values):
            assert_focus()
            subprocess.run([tool, "key", "--clearmodifiers", *values], env=env, check=True)
        subprocess.run([tool, "windowfocus", "--sync", hex(window)], env=env, check=True)
        time.sleep(1)
        def shot(name):
            subprocess.run(["import", "-window", hex(window), str(artifacts / f"{name}.png")], env=env, check=True)
        keys("Home"); time.sleep(1); shot("local-nix")
        keys("Down"); time.sleep(0.5); shot("staged-go")
        keys("Down"); time.sleep(0.5); shot("commit-rust")
        keys("F1"); time.sleep(0.2); shot("help")
        keys("Escape", "ctrl+b"); time.sleep(0.5); shot("branches")
        keys("Down", "Return"); time.sleep(0.7); shot("branch-protected")
        assert git("branch", "--show-current") == "main"
        assert "unsaved" in (repository / "main.rs").read_text()
        keys("Escape")
        (repository / "main.rs").write_text("fn main() { println!(\"base\"); }\n")
        keys("ctrl+b"); time.sleep(0.5)
        assert_focus()
        subprocess.run([tool, "type", "--clearmodifiers", "--delay", "20", "side"], env=env, check=True)
        time.sleep(0.2); shot("branch-filtered"); keys("Return"); time.sleep(1)
        assert git("branch", "--show-current") == "side"
        until = time.monotonic() + 5
        while True:
            branch_generation = re.search(r"history generation=(\d+) branch=side", log.read_text())
            if branch_generation and f"selection generation={branch_generation[1]} commit={root_hash}" in log.read_text():
                break
            assert time.monotonic() < until, "Branch switch did not preserve the selected ancestor commit"
            time.sleep(0.05)
        shot("branch-switched")
        keys("alt+r"); time.sleep(0.2); shot("watch-disabled")
        keys("ctrl+comma"); time.sleep(0.2); shot("preferences")
        keys("ctrl+Return"); time.sleep(0.4)
        assert (Path(fixture.name) / "config/gitk-rs/preferences.json").exists()
        keys("alt+t"); time.sleep(0.4); shot("light-theme")
        keys("alt+t", "slash")
        assert_focus()
        subprocess.run([tool, "type", "--clearmodifiers", "--delay", "20", "root"], env=env, check=True)
        time.sleep(0.3); shot("workflow-filter")
        keys("ctrl+e", "ctrl+u", "Escape", "F5"); time.sleep(0.6); shot("workflow-reloaded")
        alternate = Path(fixture.name) / "alternate"
        alternate.mkdir()
        subprocess.run(["git", "-C", str(alternate), "init", "--template=" + str(template), "-q", "-b", "alternate"], env=env, check=True)
        (alternate / "README.md").write_text("# Alternate repository\n")
        subprocess.run(["git", "-C", str(alternate), "add", "."], env=env, check=True)
        subprocess.run(["git", "-C", str(alternate), "-c", "user.name=Fixture", "-c", "user.email=fixture@example.test", "-c", "commit.gpgsign=false", "commit", "-qm", "alternate repository"], env=env, check=True)
        keys("ctrl+shift+o"); time.sleep(0.2); shot("repository-path-dialog")
        keys("ctrl+a", "ctrl+k")
        assert_focus()
        subprocess.run([tool, "type", "--clearmodifiers", "--delay", "5", str(alternate)], env=env, check=True)
        keys("Return")
        until = time.monotonic() + 5
        while "branch=alternate" not in log.read_text():
            assert time.monotonic() < until, log.read_text()
            time.sleep(0.05)
        time.sleep(0.3); shot("repository-opened")
        x = C.CDLL("libX11.so.6")
        x.XOpenDisplay.restype = C.c_void_p
        display = x.XOpenDisplay(None)
        x.XInternAtom.argtypes = [C.c_void_p, C.c_char_p, C.c_int]
        x.XInternAtom.restype = C.c_ulong
        class Message(C.Structure):
            _fields_ = [("type", C.c_int), ("serial", C.c_ulong), ("send_event", C.c_int), ("display", C.c_void_p), ("window", C.c_ulong), ("message_type", C.c_ulong), ("format", C.c_int), ("data", C.c_long * 5)]
        class Event(C.Union):
            _fields_ = [("message", Message), ("pad", C.c_long * 24)]
        event = Event()
        event.message.type = 33
        event.message.display = display
        event.message.window = window
        event.message.message_type = x.XInternAtom(display, b"WM_PROTOCOLS", 0)
        event.message.format = 32
        event.message.data[0] = x.XInternAtom(display, b"WM_DELETE_WINDOW", 0)
        x.XSendEvent.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_long, C.POINTER(Event)]
        x.XSendEvent(display, window, 0, 0, C.byref(event))
        x.XFlush.argtypes = [C.c_void_p]
        x.XFlush(display)
        app.wait(timeout=10)
        if app.returncode != 0:
            raise RuntimeError(log.read_text())
        print(log.read_text())
        print(f"GUI smoke passed; own-window artifacts: {artifacts}")
    finally:
        if app.poll() is None:
            os.killpg(app.pid, signal.SIGTERM)
            try:
                app.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(app.pid, signal.SIGKILL)
                app.wait()
