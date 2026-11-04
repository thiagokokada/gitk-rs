"""Shared process and X11 helpers for the optional native checks."""

import ctypes
import os
import shutil
import signal
import subprocess
import time
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from pathlib import Path


def require_commands(*commands: str) -> None:
    for command in commands:
        if shutil.which(command) is None:
            raise RuntimeError(f"Missing required command: {command}")


def wait_until(predicate: Callable[[], bool], message: str, timeout: float = 5) -> None:
    deadline = time.monotonic() + timeout
    while not predicate():
        if time.monotonic() >= deadline:
            raise RuntimeError(message)
        time.sleep(0.05)


@contextmanager
def application(
    arguments: list[str], environment: dict[str, str], log: Path
) -> Iterator[subprocess.Popen]:
    """Capture output and always terminate and reap our process group."""
    with log.open("w") as output:
        process = subprocess.Popen(
            arguments,
            env=environment,
            stdout=output,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        try:
            yield process
        finally:
            # Children can outlive their parent, so signal the group even if it exited.
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()


class ClientMessage(ctypes.Structure):
    _fields_ = [
        ("type", ctypes.c_int),
        ("serial", ctypes.c_ulong),
        ("send_event", ctypes.c_int),
        ("display", ctypes.c_void_p),
        ("window", ctypes.c_ulong),
        ("message_type", ctypes.c_ulong),
        ("format", ctypes.c_int),
        ("data", ctypes.c_long * 5),
    ]


class XEvent(ctypes.Union):
    _fields_ = [("message", ClientMessage), ("pad", ctypes.c_long * 24)]


class X11:
    """Keep Xlib signatures and resource ownership in one place."""

    def __init__(self) -> None:
        self.library = ctypes.CDLL("libX11.so.6")
        self.library.XOpenDisplay.argtypes = [ctypes.c_char_p]
        self.library.XOpenDisplay.restype = ctypes.c_void_p
        self.library.XCloseDisplay.argtypes = [ctypes.c_void_p]
        self.library.XInternAtom.argtypes = [
            ctypes.c_void_p,
            ctypes.c_char_p,
            ctypes.c_int,
        ]
        self.library.XInternAtom.restype = ctypes.c_ulong
        self.library.XSendEvent.argtypes = [
            ctypes.c_void_p,
            ctypes.c_ulong,
            ctypes.c_int,
            ctypes.c_long,
            ctypes.POINTER(XEvent),
        ]
        self.library.XFlush.argtypes = [ctypes.c_void_p]
        self.display = self.library.XOpenDisplay(None)
        if not self.display:
            raise RuntimeError("An X11 display is required")

    def __enter__(self) -> "X11":
        return self

    def __exit__(self, *_: object) -> None:
        self.library.XCloseDisplay(self.display)

    def close_window(self, window: str) -> None:
        """Request normal application shutdown through WM_DELETE_WINDOW."""
        event = XEvent()
        event.message.type = 33  # ClientMessage
        event.message.display = self.display
        event.message.window = int(window, 0)
        event.message.message_type = self.library.XInternAtom(
            self.display, b"WM_PROTOCOLS", 0
        )
        event.message.format = 32
        event.message.data[0] = self.library.XInternAtom(
            self.display, b"WM_DELETE_WINDOW", 0
        )
        self.library.XSendEvent(self.display, int(window, 0), 0, 0, ctypes.byref(event))
        self.library.XFlush(self.display)
