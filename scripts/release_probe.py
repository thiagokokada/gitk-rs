#!/usr/bin/env python3
"""Observe X11 startup and memory use; this is not a statistical benchmark.

Requires git, xdotool, xwininfo, ImageMagick import and X11/XRes libraries.
Only windows whose XRes client PID matches the child process are touched.
"""

import argparse
import json
import os
import re
import subprocess
import tempfile
import time
from pathlib import Path

from native_tools import X11, application, require_commands

TRIAL_SECONDS = 8
SAMPLE_SECONDS = 0.025


def memory(pid: int) -> dict[str, int]:
    """Read KiB values; a process disappearing between samples is normal."""
    try:
        status = Path(f"/proc/{pid}/status").read_text()
    except OSError:
        status = ""
    fields = dict(re.findall(r"^(VmRSS|VmHWM):\s+(\d+)", status, re.MULTILINE))
    return {key: int(fields.get(key, 0)) for key in ("VmRSS", "VmHWM")}


def children(pid: int) -> set[int]:
    """Include children created by any thread, without counting them twice."""
    descendants = set()
    pending = [pid]
    while pending:
        parent = pending.pop()
        try:
            threads = list(Path(f"/proc/{parent}/task").iterdir())
        except OSError:
            continue
        for thread in threads:
            try:
                direct = {
                    int(value) for value in (thread / "children").read_text().split()
                }
            except OSError:
                continue
            unseen = direct - descendants - {pid}
            descendants.update(unseen)
            pending.extend(unseen)
    return descendants


def find_window(label: str, pid: int, x11: X11, env: dict[str, str]) -> str | None:
    title = "gitk-rs" if label == "rust" else "gitk-go"
    found = subprocess.run(
        ["xdotool", "search", "--all", "--onlyvisible", "--name", f"^{title}$"],
        env=env,
        capture_output=True,
        text=True,
        timeout=5,
        check=False,
    )
    return next(
        (
            window
            for window in found.stdout.splitlines()
            if x11.owns_window(window, pid)
        ),
        None,
    )


def observe_trial(
    arguments: list[str],
    label: str,
    trial: str,
    root: Path,
    env: dict[str, str],
    x11: X11,
) -> dict:
    log = root / f"{label}-{trial}.log"
    marker = "phase=HistoryPainted" if label == "rust" else "reloadCommitsAsync loaded"
    started = time.monotonic()
    child_peak = 0
    app_peak = 0
    first_history = None
    window = None
    with application(arguments, env, log) as app:
        while time.monotonic() - started < TRIAL_SECONDS:
            if app.poll() is not None:
                raise RuntimeError(
                    f"{label} exited {app.returncode}: {log.read_text()[-2000:]}"
                )
            if window is None and time.monotonic() - started > 0.5:
                window = find_window(label, app.pid, x11, env)
                if window is not None:
                    subprocess.run(
                        ["xdotool", "windowsize", "--sync", window, "1920", "1332"],
                        env=env,
                        timeout=5,
                        check=True,
                    )
            observed = memory(app.pid)
            app_peak = max(app_peak, observed["VmHWM"])
            child_peak = max(
                child_peak, sum(memory(pid)["VmRSS"] for pid in children(app.pid))
            )
            if first_history is None and marker in log.read_text():
                first_history = round((time.monotonic() - started) * 1000, 1)
            time.sleep(SAMPLE_SECONDS)

        if window is None:
            raise RuntimeError("Own window not found")
        info = subprocess.check_output(
            ["xwininfo", "-id", window], text=True, env=env, timeout=5
        )
        dimensions = dict(re.findall(r"(Width|Height): (\d+)", info))
        subprocess.run(
            ["import", "-window", window, str(root / f"{label}-{trial}.png")],
            env=env,
            timeout=10,
            check=True,
        )
        return {
            "app": label,
            "trial": trial,
            "duration_s": TRIAL_SECONDS,
            "width": int(dimensions["Width"]),
            "height": int(dimensions["Height"]),
            "rss_kib_at_end": observed["VmRSS"],
            "app_vm_hwm_kib": app_peak,
            "sampled_children_peak_total_rss_kib": child_peak,
            "first_history_log_observed_ms": first_history,
            "first_history_event": (
                "CPU paint callback"
                if label == "rust"
                else "Go controller batch applied"
            ),
            "log": str(log),
        }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary")
    parser.add_argument("app", choices=("go", "rust"))
    parser.add_argument("repository", type=Path)
    parser.add_argument(
        "output_directory",
        type=Path,
        nargs="?",
        default=Path("/tmp/gitk-rs-release-observations"),
    )
    options = parser.parse_args()
    require_commands("git", "xdotool", "xwininfo", "import")
    root = options.output_directory
    root.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.pop("WAYLAND_DISPLAY", None)
    # Retain these directories with the observations; reuse both for the warm trial.
    env["XDG_CACHE_HOME"] = tempfile.mkdtemp(prefix=f"{options.app}-cache-", dir=root)
    env["XDG_CONFIG_HOME"] = tempfile.mkdtemp(prefix=f"{options.app}-config-", dir=root)
    arguments = [
        options.binary,
        "--limit",
        "192",
        "--mode",
        "dark",
        "--nowatch",
        "--verbose",
    ]
    if options.app == "rust":
        arguments.append("--timings")
    arguments.append(str(options.repository))
    records = []
    with X11() as x11:
        for trial in ("first-launch", "warm-reopen"):
            record = observe_trial(arguments, options.app, trial, root, env, x11)
            records.append(record)
            print(json.dumps(record), flush=True)
    (root / f"{options.app}.json").write_text(json.dumps(records, indent=2) + "\n")


if __name__ == "__main__":
    main()
