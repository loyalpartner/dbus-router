"""VSCode launch and verification utilities."""

import os
import signal
import subprocess
import time
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path


@dataclass
class VSCodeSession:
    """VSCode session information."""

    display: str
    pids: list[int]


def find_vscode_pids(display: str) -> list[int]:
    """Find VSCode process IDs by matching display and process name.

    Args:
        display: X display to match

    Returns:
        List of PIDs for VSCode electron processes
    """
    pids = []
    # Find processes with 'code' in cmdline that have matching DISPLAY
    result = subprocess.run(
        ["pgrep", "-f", "code.*--type="],
        capture_output=True,
        text=True,
    )
    if result.returncode == 0 and result.stdout.strip():
        for pid_str in result.stdout.strip().split("\n"):
            try:
                pid = int(pid_str)
                # Check if this process has the right DISPLAY
                environ_path = f"/proc/{pid}/environ"
                if os.path.exists(environ_path):
                    with open(environ_path, "rb") as f:
                        environ = f.read()
                    if f"DISPLAY={display}".encode() in environ:
                        pids.append(pid)
            except (ValueError, OSError):
                continue
    return pids


def find_vscode_main_pid(display: str) -> int | None:
    """Find the main VSCode process PID.

    Args:
        display: X display to match

    Returns:
        Main VSCode PID or None
    """
    result = subprocess.run(
        ["pgrep", "-f", r"code.*--ms-enable-electron-run-as-node"],
        capture_output=True,
        text=True,
    )
    if result.returncode == 0 and result.stdout.strip():
        for pid_str in result.stdout.strip().split("\n"):
            try:
                pid = int(pid_str)
                environ_path = f"/proc/{pid}/environ"
                if os.path.exists(environ_path):
                    with open(environ_path, "rb") as f:
                        environ = f.read()
                    if f"DISPLAY={display}".encode() in environ:
                        return pid
            except (ValueError, OSError):
                continue
    return None


@contextmanager
def vscode_session(
    display: str,
    dbus_addr: str,
    log_dir: Path,
    wait_seconds: float = 5.0,
):
    """Launch VSCode with custom D-Bus address.

    Args:
        display: X display to use
        dbus_addr: D-Bus session bus address
        log_dir: Directory for log files
        wait_seconds: Time to wait for VSCode to start

    Yields:
        VSCodeSession with display and PIDs
    """
    env = os.environ.copy()
    env["DISPLAY"] = display
    env["DBUS_SESSION_BUS_ADDRESS"] = dbus_addr

    stdout = open(log_dir / "vscode.stdout", "w")
    stderr = open(log_dir / "vscode.stderr", "w")

    # The 'code' command is a launcher that exits immediately
    subprocess.Popen(
        ["code", "--new-window", "--disable-gpu"],
        env=env,
        stdout=stdout,
        stderr=stderr,
    )

    time.sleep(wait_seconds)

    # Find the actual VSCode processes
    pids = find_vscode_pids(display)
    session = VSCodeSession(display=display, pids=pids)

    try:
        yield session
    finally:
        # Terminate all VSCode processes for this display
        for pid in find_vscode_pids(display):
            try:
                os.kill(pid, signal.SIGTERM)
            except OSError:
                pass

        # Wait a moment then force kill any remaining
        time.sleep(1)
        for pid in find_vscode_pids(display):
            try:
                os.kill(pid, signal.SIGKILL)
            except OSError:
                pass

        stdout.close()
        stderr.close()


def is_vscode_running(session: VSCodeSession) -> bool:
    """Check if VSCode is still running.

    Args:
        session: VSCodeSession to check

    Returns:
        True if any VSCode processes are running for this display
    """
    pids = find_vscode_pids(session.display)
    return len(pids) > 0


def find_vscode_windows(display: str) -> list[int]:
    """Find VSCode window IDs using xdotool.

    Searches by window class name since VSCode may not set window title immediately.

    Args:
        display: X display to search

    Returns:
        List of window IDs
    """
    env = {"DISPLAY": display, "PATH": os.environ.get("PATH", "")}

    # Try searching by class name first (more reliable)
    result = subprocess.run(
        ["xdotool", "search", "--class", "code"],
        env=env,
        capture_output=True,
        text=True,
    )
    if result.returncode == 0 and result.stdout.strip():
        return [int(wid) for wid in result.stdout.strip().split("\n")]

    # Fallback to window name
    result = subprocess.run(
        ["xdotool", "search", "--name", "Visual Studio Code"],
        env=env,
        capture_output=True,
        text=True,
    )
    if result.returncode == 0 and result.stdout.strip():
        return [int(wid) for wid in result.stdout.strip().split("\n")]

    return []
