"""Sandbox routing integration tests (default behavior without hostpass)."""

from pathlib import Path

from utils.dbus_env import echo_service_session
from utils.echo_client import (
    sync_call_echo,
    sync_call_raise_error,
    emit_and_wait_for_signal,
)
from utils.assertions import assert_dbus_service_exists, assert_dbus_service_not_exists


def test_sandbox_service_registration(test_log_dir: Path, router_env):
    """Service without hostpass should register on sandbox bus (not host)."""
    with router_env(socket_prefix="sb_") as env:
        host_addr, sandbox_addr, router_addr = env
        with echo_service_session(router_addr, test_log_dir):
            # Service should be on SANDBOX bus
            assert_dbus_service_exists(sandbox_addr, "org.test.Echo")
            # Service should NOT be on host bus
            assert_dbus_service_not_exists(host_addr, "org.test.Echo")


def test_sandbox_method_call(test_log_dir: Path, router_env):
    """Client on sandbox bus should be able to call sandbox service."""
    with router_env(socket_prefix="sb_") as env:
        host_addr, sandbox_addr, router_addr = env
        with echo_service_session(router_addr, test_log_dir):
            # Call from SANDBOX bus should work
            result = sync_call_echo(sandbox_addr, "sandbox message")
            assert result == "sandbox message"


def test_sandbox_signal_routing(test_log_dir: Path, router_env):
    """Signal from sandbox service should be visible on sandbox bus."""
    with router_env(socket_prefix="sb_") as env:
        host_addr, sandbox_addr, router_addr = env
        with echo_service_session(router_addr, test_log_dir):
            # Emit signal via router and verify it's received on sandbox bus
            signal_received = emit_and_wait_for_signal(
                emit_addr=router_addr,
                listen_addr=sandbox_addr,
                message="sandbox signal",
            )
            assert signal_received == "sandbox signal"


def test_sandbox_error_routing(test_log_dir: Path, router_env):
    """Error from sandbox service should be routed back to sandbox client."""
    with router_env(socket_prefix="sb_") as env:
        host_addr, sandbox_addr, router_addr = env
        with echo_service_session(router_addr, test_log_dir):
            # Call from SANDBOX bus, expect error to be routed back
            error_name, error_message = sync_call_raise_error(
                sandbox_addr, "sandbox error message"
            )
            assert error_name == "org.test.Echo.TestError"
            assert error_message == "sandbox error message"
