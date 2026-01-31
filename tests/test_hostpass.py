"""Hostpass routing integration tests."""

from pathlib import Path

from utils.dbus_env import echo_service_session
from utils.echo_client import (
    sync_call_echo,
    sync_call_raise_error,
    emit_and_wait_for_signal,
)
from utils.assertions import assert_dbus_service_exists


HOSTPASS_CONFIG = '''
[[hostpass]]
process = "*/python3*"
'''


def test_hostpass_service_registration(test_log_dir: Path, router_env):
    """Service in sandbox should register on host bus via router with hostpass."""
    # Use short paths for sockets to avoid Unix socket path length limit (108 chars)
    with router_env(HOSTPASS_CONFIG, socket_prefix="hp_") as env:
        host_addr, sandbox_addr, router_addr = env
        # Echo service connects to router, gets hostpass routing
        with echo_service_session(router_addr, test_log_dir):
            # Verify service is visible on HOST bus (not sandbox)
            assert_dbus_service_exists(host_addr, "org.test.Echo")


def test_hostpass_method_call(test_log_dir: Path, router_env):
    """Host client should be able to call service running in sandbox."""
    with router_env(HOSTPASS_CONFIG, socket_prefix="hp_") as env:
        host_addr, sandbox_addr, router_addr = env
        with echo_service_session(router_addr, test_log_dir):
            # Call from HOST bus, should reach sandbox service via router
            result = sync_call_echo(host_addr, "hello world")
            assert result == "hello world"


def test_hostpass_multiple_calls(test_log_dir: Path, router_env):
    """Multiple method calls should all succeed."""
    with router_env(HOSTPASS_CONFIG, socket_prefix="hp_") as env:
        host_addr, sandbox_addr, router_addr = env
        with echo_service_session(router_addr, test_log_dir):
            for i in range(5):
                result = sync_call_echo(host_addr, f"message {i}")
                assert result == f"message {i}"


def test_hostpass_signal_routing(test_log_dir: Path, router_env):
    """Signal from hostpass service should be visible on host bus."""
    with router_env(HOSTPASS_CONFIG, socket_prefix="hp_") as env:
        host_addr, sandbox_addr, router_addr = env
        with echo_service_session(router_addr, test_log_dir):
            # Emit signal via router and verify it's received on host bus
            signal_received = emit_and_wait_for_signal(
                emit_addr=router_addr,
                listen_addr=host_addr,
                message="test signal",
            )
            assert signal_received == "test signal"


def test_hostpass_error_routing(test_log_dir: Path, router_env):
    """Error from hostpass service should be routed back to host client."""
    with router_env(HOSTPASS_CONFIG, socket_prefix="hp_") as env:
        host_addr, sandbox_addr, router_addr = env
        with echo_service_session(router_addr, test_log_dir):
            # Call from HOST bus, expect error to be routed back
            error_name, error_message = sync_call_raise_error(
                host_addr, "test error message"
            )
            assert error_name == "org.test.Echo.TestError"
            assert error_message == "test error message"
