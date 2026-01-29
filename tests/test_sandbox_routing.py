"""Sandbox routing integration tests (default behavior without hostpass)."""

import tempfile
from pathlib import Path

from utils.dbus_env import dbus_session, dbus_router_session, echo_service_session
from utils.echo_client import (
    sync_call_echo,
    sync_call_raise_error,
    emit_and_wait_for_signal,
)
from utils.assertions import assert_dbus_service_exists, assert_dbus_service_not_exists


def test_sandbox_service_registration(test_log_dir: Path, build_project):
    """Service without hostpass should register on sandbox bus (not host)."""
    with tempfile.TemporaryDirectory(prefix="sb_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        # Empty config - no hostpass rules
        config = test_log_dir / "router.toml"
        config.write_text("")

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    with echo_service_session(router_addr, test_log_dir):
                        # Service should be on SANDBOX bus
                        assert_dbus_service_exists(sandbox_addr, "org.test.Echo")
                        # Service should NOT be on host bus
                        assert_dbus_service_not_exists(host_addr, "org.test.Echo")


def test_sandbox_method_call(test_log_dir: Path, build_project):
    """Client on sandbox bus should be able to call sandbox service."""
    with tempfile.TemporaryDirectory(prefix="sb_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        config = test_log_dir / "router.toml"
        config.write_text("")

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    with echo_service_session(router_addr, test_log_dir):
                        # Call from SANDBOX bus should work
                        result = sync_call_echo(sandbox_addr, "sandbox message")
                        assert result == "sandbox message"


def test_sandbox_signal_routing(test_log_dir: Path, build_project):
    """Signal from sandbox service should be visible on sandbox bus."""
    with tempfile.TemporaryDirectory(prefix="sb_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        # Empty config - no hostpass rules
        config = test_log_dir / "router.toml"
        config.write_text("")

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    with echo_service_session(router_addr, test_log_dir):
                        # Emit signal via router and verify it's received on sandbox bus
                        signal_received = emit_and_wait_for_signal(
                            emit_addr=router_addr,
                            listen_addr=sandbox_addr,
                            message="sandbox signal",
                        )
                        assert signal_received == "sandbox signal"


def test_sandbox_error_routing(test_log_dir: Path, build_project):
    """Error from sandbox service should be routed back to sandbox client."""
    with tempfile.TemporaryDirectory(prefix="sb_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        # Empty config - no hostpass rules
        config = test_log_dir / "router.toml"
        config.write_text("")

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    with echo_service_session(router_addr, test_log_dir):
                        # Call from SANDBOX bus, expect error to be routed back
                        error_name, error_message = sync_call_raise_error(
                            sandbox_addr, "sandbox error message"
                        )
                        assert error_name == "org.test.Echo.TestError"
                        assert error_message == "sandbox error message"
