"""Test hostpass service survives sandbox bus disconnection.

When the sandbox D-Bus daemon disconnects, hostpass clients should continue
to work because they are registered on the host bus.

Bug: Currently when sandbox bus disconnects, the entire session ends,
causing hostpass services to become unreachable even though they are
registered on the host bus.
"""

import tempfile
import time
from pathlib import Path

from utils.echo_client import sync_call_echo
from utils.assertions import assert_dbus_service_exists
from utils.dbus_env import dbus_session_with_process, dbus_router_session, echo_service_session


HOSTPASS_CONFIG = '''
[[hostpass]]
process = "*/python3*"
'''


def test_hostpass_survives_sandbox_disconnect(test_log_dir: Path, router_binary):
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
        config.write_text(HOSTPASS_CONFIG)

        with dbus_session_with_process(
            host_dbus_socket, test_log_dir, "host-dbus"
        ) as (host_addr, _):
            with dbus_session_with_process(
                sandbox_dbus_socket, test_log_dir, "sandbox-dbus"
            ) as (sandbox_addr, sandbox_proc):
                with dbus_router_session(
                    router_socket,
                    host_addr,
                    sandbox_addr,
                    config,
                    test_log_dir,
                    router_binary=router_binary,
                ) as router_addr:
                    with echo_service_session(router_addr, test_log_dir):
                        # Step 1: Verify service is registered on host bus
                        assert_dbus_service_exists(host_addr, "org.test.Echo")
                        (test_log_dir / "step1_service_exists.log").write_text("OK")

                        # Step 2: Verify service works before sandbox disconnect
                        result = sync_call_echo(host_addr, "before disconnect")
                        assert result == "before disconnect", (
                            f"Expected 'before disconnect', got '{result}'"
                        )
                        (test_log_dir / "step2_call_before.log").write_text(
                            f"OK: {result}"
                        )

                        # Step 3: Kill sandbox dbus daemon
                        sandbox_proc.terminate()
                        sandbox_proc.wait()
                        time.sleep(0.5)  # Give router time to detect disconnection
                        (test_log_dir / "step3_sandbox_killed.log").write_text(
                            "Sandbox dbus terminated"
                        )

                        # Step 4: Verify service STILL works after sandbox disconnect
                        # This is the bug - currently the session ends and service becomes unreachable
                        result = sync_call_echo(host_addr, "after disconnect")
                        assert result == "after disconnect", (
                            f"Expected 'after disconnect', got '{result}'"
                        )
                        (test_log_dir / "step4_call_after.log").write_text(
                            f"OK: {result}"
                        )
