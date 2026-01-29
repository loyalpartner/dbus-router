"""X server environment management for integration tests."""

import os
import subprocess
import time
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path

from .display import (
    get_host_display_for_worker,
    get_sandbox_display_for_worker,
    is_headless_mode,
)


@dataclass
class XorgEnv:
    """X server environment information."""

    host_display: str
    sandbox_display: str


@contextmanager
def xvfb_session(display: str, log_dir: Path, name: str = "xvfb"):
    """Start Xvfb virtual framebuffer.

    Args:
        display: X display number (e.g., ":149")
        log_dir: Directory for log files
        name: Log file prefix
    """
    stdout = open(log_dir / f"{name}.stdout", "w")
    stderr = open(log_dir / f"{name}.stderr", "w")
    proc = subprocess.Popen(
        ["Xvfb", display, "-screen", "0", "1920x1080x24", "-ac"],
        stdout=stdout,
        stderr=stderr,
    )
    time.sleep(0.3)
    try:
        yield proc
    finally:
        proc.terminate()
        proc.wait()
        stdout.close()
        stderr.close()


@contextmanager
def xephyr_session(display: str, host_display: str, log_dir: Path, name: str = "xephyr"):
    """Start Xephyr nested X server.

    Args:
        display: Target display number for Xephyr (e.g., ":99")
        host_display: Host display to run Xephyr on
        log_dir: Directory for log files
        name: Log file prefix
    """
    env = os.environ.copy()
    env["DISPLAY"] = host_display
    stdout = open(log_dir / f"{name}.stdout", "w")
    stderr = open(log_dir / f"{name}.stderr", "w")
    proc = subprocess.Popen(
        ["Xephyr", display, "-ac", "-br", "-screen", "1920x1080", "-host-cursor"],
        env=env,
        stdout=stdout,
        stderr=stderr,
    )
    time.sleep(0.5)
    try:
        yield proc
    finally:
        proc.terminate()
        proc.wait()
        stdout.close()
        stderr.close()


@contextmanager
def xorg_test_env(log_dir: Path):
    """High-level X environment setup with separate host and sandbox displays.

    Architecture:
    - Host Xvfb: Simulates the real host X server
    - Sandbox Xvfb: Isolated environment where sandboxed apps run

    In headless mode:
        Host Xvfb (:149) - host environment
        Sandbox Xvfb (:99) - sandbox environment

    In visual mode:
        Uses real host display for host
        Xephyr (:99) on host display for sandbox (visible window)

    Args:
        log_dir: Directory for log files

    Yields:
        XorgEnv with host_display and sandbox_display
    """
    host_display = get_host_display_for_worker()
    sandbox_display = get_sandbox_display_for_worker()

    if is_headless_mode():
        # Headless: two separate Xvfb instances
        with xvfb_session(host_display, log_dir, "xvfb-host"):
            with xvfb_session(sandbox_display, log_dir, "xvfb-sandbox"):
                yield XorgEnv(host_display=host_display, sandbox_display=sandbox_display)
    else:
        # Visual: use real host display, Xephyr for sandbox visibility
        real_host_display = os.environ.get("DISPLAY", ":0")
        with xephyr_session(sandbox_display, real_host_display, log_dir, "xephyr-sandbox"):
            yield XorgEnv(host_display=real_host_display, sandbox_display=sandbox_display)
