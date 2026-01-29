"""D-Bus daemon and router management for integration tests."""

import os
import subprocess
import time
from contextlib import contextmanager
from pathlib import Path


@contextmanager
def dbus_session(socket_path: Path, log_dir: Path, name: str = "dbus"):
    """Start a dbus-daemon session.

    Args:
        socket_path: Path for the D-Bus socket
        log_dir: Directory for log and config files
        name: Name prefix for log files

    Yields:
        D-Bus address string (e.g., "unix:path=/tmp/dbus.sock")
    """
    config = f"""<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={socket_path}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*"/>
    <allow receive_sender="*"/>
    <allow own="*"/>
  </policy>
</busconfig>
"""
    config_path = log_dir / f"{name}.conf"
    config_path.write_text(config)

    stdout = open(log_dir / f"{name}.stdout", "w")
    stderr = open(log_dir / f"{name}.stderr", "w")

    proc = subprocess.Popen(
        ["dbus-daemon", "--config-file", str(config_path), "--nofork"],
        stdout=stdout,
        stderr=stderr,
    )
    time.sleep(0.3)
    try:
        yield f"unix:path={socket_path}"
    finally:
        proc.terminate()
        proc.wait()
        stdout.close()
        stderr.close()


# Backwards compatibility alias
sandbox_dbus_session = dbus_session


@contextmanager
def dbus_router_session(
    listen_path: Path,
    host_addr: str,
    sandbox_addr: str,
    config_path: Path,
    log_dir: Path,
):
    """Start the dbus-router under test.

    Args:
        listen_path: Path for the router's listen socket
        host_addr: D-Bus address for the host bus
        sandbox_addr: D-Bus address for the sandbox bus
        config_path: Path to the router configuration file
        log_dir: Directory for log files

    Yields:
        D-Bus address string for the router
    """
    env = os.environ.copy()
    env["RUST_LOG"] = "debug"

    stdout = open(log_dir / "router.stdout", "w")
    stderr = open(log_dir / "router.stderr", "w")

    proc = subprocess.Popen(
        [
            "cargo",
            "run",
            "--release",
            "--",
            "--listen",
            str(listen_path),
            "--host",
            host_addr,
            "--sandbox",
            sandbox_addr,
            "--config",
            str(config_path),
        ],
        stdout=stdout,
        stderr=stderr,
        env=env,
    )
    time.sleep(0.5)
    try:
        yield f"unix:path={listen_path}"
    finally:
        proc.terminate()
        proc.wait()
        stdout.close()
        stderr.close()
