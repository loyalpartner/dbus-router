"""Hostpass routing integration tests."""

import tempfile
from pathlib import Path

from utils.dbus_env import dbus_session, dbus_router_session, echo_service_session
from utils.echo_client import (
    sync_call_echo,
    sync_call_raise_error,
    emit_and_wait_for_signal,
)
from utils.assertions import assert_dbus_service_exists


def test_hostpass_service_registration(test_log_dir: Path, build_project):
    """Service in sandbox should register on host bus via router with hostpass."""
    # Use short paths for sockets to avoid Unix socket path length limit (108 chars)
    with tempfile.TemporaryDirectory(prefix="hp_") as sock_dir:
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

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    # Echo service connects to router, gets hostpass routing
                    with echo_service_session(router_addr, test_log_dir):
                        # Verify service is visible on HOST bus (not sandbox)
                        assert_dbus_service_exists(host_addr, "org.test.Echo")


def test_hostpass_method_call(test_log_dir: Path, build_project):
    """Host client should be able to call service running in sandbox."""
    with tempfile.TemporaryDirectory(prefix="hp_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        config = test_log_dir / "router.toml"
        config.write_text('''
[[hostpass]]
process = "*/python3*"
''')

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    with echo_service_session(router_addr, test_log_dir):
                        # Call from HOST bus, should reach sandbox service via router
                        result = sync_call_echo(host_addr, "hello world")
                        assert result == "hello world"


def test_hostpass_multiple_calls(test_log_dir: Path, build_project):
    """Multiple method calls should all succeed."""
    with tempfile.TemporaryDirectory(prefix="hp_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        config = test_log_dir / "router.toml"
        config.write_text('''
[[hostpass]]
process = "*/python3*"
''')

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    with echo_service_session(router_addr, test_log_dir):
                        for i in range(5):
                            result = sync_call_echo(host_addr, f"message {i}")
                            assert result == f"message {i}"


def test_hostpass_signal_routing(test_log_dir: Path, build_project):
    """Signal from hostpass service should be visible on host bus."""
    with tempfile.TemporaryDirectory(prefix="hp_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        config = test_log_dir / "router.toml"
        config.write_text('''
[[hostpass]]
process = "*/python3*"
''')

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    with echo_service_session(router_addr, test_log_dir):
                        # Emit signal via router and verify it's received on host bus
                        signal_received = emit_and_wait_for_signal(
                            emit_addr=router_addr,
                            listen_addr=host_addr,
                            message="test signal",
                        )
                        assert signal_received == "test signal"


def test_hostpass_error_routing(test_log_dir: Path, build_project):
    """Error from hostpass service should be routed back to host client."""
    with tempfile.TemporaryDirectory(prefix="hp_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        config = test_log_dir / "router.toml"
        config.write_text('''
[[hostpass]]
process = "*/python3*"
''')

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    with echo_service_session(router_addr, test_log_dir):
                        # Call from HOST bus, expect error to be routed back
                        error_name, error_message = sync_call_raise_error(
                            host_addr, "test error message"
                        )
                        assert error_name == "org.test.Echo.TestError"
                        assert error_message == "test error message"
