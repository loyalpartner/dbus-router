"""Test hostpass service survives sandbox bus disconnection.

When the sandbox D-Bus daemon disconnects, hostpass clients should continue
to work because they are registered on the host bus.

Bug: Currently when sandbox bus disconnects, the entire session ends,
causing hostpass services to become unreachable even though they are
registered on the host bus.
"""

import os
import subprocess
import tempfile
import time
from pathlib import Path

from utils.echo_client import sync_call_echo
from utils.assertions import assert_dbus_service_exists


def test_hostpass_survives_sandbox_disconnect(test_log_dir: Path, build_project):
    """Hostpass service should remain accessible after sandbox bus disconnects.

    This test:
    1. Starts host dbus, sandbox dbus, and router
    2. Starts echo service (hostpass) which registers on host bus
    3. Verifies the service works
    4. Kills the sandbox dbus daemon
    5. Verifies the service STILL works (currently fails - this is the bug)
    """
    with tempfile.TemporaryDirectory(prefix="hpsd_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        # Configure hostpass for echo_service.py
        config = test_log_dir / "router.toml"
        config.write_text('''
[[hostpass]]
process = "*/python3*"
''')

        # Start host dbus
        host_config = _write_dbus_config(test_log_dir, "host-dbus", host_dbus_socket)
        host_proc = subprocess.Popen(
            ["dbus-daemon", "--config-file", str(host_config), "--nofork"],
            stdout=open(test_log_dir / "host-dbus.stdout", "w"),
            stderr=open(test_log_dir / "host-dbus.stderr", "w"),
        )
        time.sleep(0.3)
        host_addr = f"unix:path={host_dbus_socket}"

        # Start sandbox dbus
        sandbox_config = _write_dbus_config(test_log_dir, "sandbox-dbus", sandbox_dbus_socket)
        sandbox_proc = subprocess.Popen(
            ["dbus-daemon", "--config-file", str(sandbox_config), "--nofork"],
            stdout=open(test_log_dir / "sandbox-dbus.stdout", "w"),
            stderr=open(test_log_dir / "sandbox-dbus.stderr", "w"),
        )
        time.sleep(0.3)
        sandbox_addr = f"unix:path={sandbox_dbus_socket}"

        # Start router
        env = os.environ.copy()
        env["RUST_LOG"] = "debug"
        router_proc = subprocess.Popen(
            [
                "cargo", "run", "--release", "--",
                "--listen", str(router_socket),
                "--host", host_addr,
                "--sandbox", sandbox_addr,
                "--config", str(config),
            ],
            stdout=open(test_log_dir / "router.stdout", "w"),
            stderr=open(test_log_dir / "router.stderr", "w"),
            env=env,
        )
        # Wait for router socket
        for _ in range(20):
            time.sleep(0.1)
            if router_socket.exists():
                break
        assert router_socket.exists(), "Router socket not created"
        router_addr = f"unix:path={router_socket}"

        # Start echo service (hostpass client)
        echo_env = os.environ.copy()
        echo_env["DBUS_SESSION_BUS_ADDRESS"] = router_addr
        echo_script = Path(__file__).parent / "utils" / "echo_service.py"
        echo_proc = subprocess.Popen(
            ["python3", str(echo_script)],
            stdout=open(test_log_dir / "echo-service.stdout", "w"),
            stderr=open(test_log_dir / "echo-service.stderr", "w"),
            env=echo_env,
        )
        time.sleep(0.5)

        try:
            # Step 1: Verify service is registered on host bus
            assert_dbus_service_exists(host_addr, "org.test.Echo")
            (test_log_dir / "step1_service_exists.log").write_text("OK")

            # Step 2: Verify service works before sandbox disconnect
            result = sync_call_echo(host_addr, "before disconnect")
            assert result == "before disconnect", f"Expected 'before disconnect', got '{result}'"
            (test_log_dir / "step2_call_before.log").write_text(f"OK: {result}")

            # Step 3: Kill sandbox dbus daemon
            sandbox_proc.terminate()
            sandbox_proc.wait()
            time.sleep(0.5)  # Give router time to detect disconnection
            (test_log_dir / "step3_sandbox_killed.log").write_text("Sandbox dbus terminated")

            # Step 4: Verify service STILL works after sandbox disconnect
            # This is the bug - currently the session ends and service becomes unreachable
            result = sync_call_echo(host_addr, "after disconnect")
            assert result == "after disconnect", f"Expected 'after disconnect', got '{result}'"
            (test_log_dir / "step4_call_after.log").write_text(f"OK: {result}")

        finally:
            echo_proc.terminate()
            echo_proc.wait()
            router_proc.terminate()
            router_proc.wait()
            host_proc.terminate()
            host_proc.wait()
            if sandbox_proc.poll() is None:
                sandbox_proc.terminate()
                sandbox_proc.wait()


def _write_dbus_config(log_dir: Path, name: str, socket_path: Path) -> Path:
    """Write dbus-daemon config file."""
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
    return config_path
