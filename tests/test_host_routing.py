"""Host routing integration tests - tests for routing messages to host bus.

This test file specifically tests scenarios where a client:
1. Connects to router (Hello goes to sandbox bus)
2. Then sends messages to services configured in host_routes (goes to host bus)

This simulates the VSCode crash scenario where org.a11y.Bus was in host_routes.
"""

import tempfile
import asyncio
from pathlib import Path

from dbus_next.aio import MessageBus
from dbus_next import Message, MessageType

from utils.dbus_env import dbus_session, dbus_router_session


def test_host_route_after_sandbox_hello(test_log_dir: Path, build_project):
    """Client connects via router, then calls a host-routed service.

    This reproduces the VSCode crash scenario:
    1. Client connects to router
    2. Hello() is sent - routes to sandbox bus (default)
    3. Client calls org.test.HostService - routes to host bus
    4. Host bus rejects because it never received Hello() from router
    """
    with tempfile.TemporaryDirectory(prefix="hr_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        # Configure host_routes for org.test.HostService
        # No hostpass - so Hello() will go to sandbox by default
        config = test_log_dir / "router.toml"
        config.write_text('''
[[host_routes]]
destination = "org.test.HostService"
''')

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    # Run async test
                    result = asyncio.run(
                        _test_host_route_call(router_addr, test_log_dir)
                    )
                    assert result == "success", f"Test failed: {result}"


async def _test_host_route_call(router_addr: str, test_log_dir: Path) -> str:
    """Async test: connect to router and call a host-routed service."""
    try:
        # Connect to router
        bus = await MessageBus(bus_address=router_addr).connect()

        # At this point, Hello() has been sent automatically by dbus_next
        # It routes to sandbox bus (no hostpass configured)

        # Now try to call a service that routes to host bus
        # This should fail if host bus didn't receive Hello()
        reply = await bus.call(
            Message(
                destination='org.test.HostService',
                path='/org/test/HostService',
                interface='org.freedesktop.DBus.Peer',
                member='Ping',
            )
        )

        # If we get here, the call succeeded (service doesn't exist, but bus accepted it)
        # We expect an error like "service unknown", not a disconnect
        bus.disconnect()
        return "success"

    except Exception as e:
        error_str = str(e)
        # Log the error for debugging
        (test_log_dir / "test_error.log").write_text(f"Error: {error_str}\n")

        # Check if it's a "service not found" error (expected) vs connection error (bug)
        if "org.freedesktop.DBus.Error.ServiceUnknown" in error_str:
            # This is expected - service doesn't exist, but the call was routed correctly
            return "success"
        elif "disconnect" in error_str.lower() or "connection" in error_str.lower():
            # This is the bug - host bus disconnected
            return f"connection_error: {error_str}"
        else:
            return f"unexpected_error: {error_str}"


def test_mixed_sandbox_and_host_calls(test_log_dir: Path, build_project):
    """Client makes calls to both sandbox and host routed services.

    This is a more comprehensive test:
    1. Client connects to router
    2. Makes a call to sandbox bus (org.freedesktop.DBus)
    3. Makes a call to host-routed service
    4. Makes another call to sandbox bus

    All calls should succeed (or return service-not-found, not disconnect).
    """
    with tempfile.TemporaryDirectory(prefix="hr_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        config = test_log_dir / "router.toml"
        config.write_text('''
[[host_routes]]
destination = "org.a11y.Bus"

[[host_routes]]
destination = "org.test.HostOnly"
''')

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    result = asyncio.run(
                        _test_mixed_calls(router_addr, test_log_dir)
                    )
                    assert result == "success", f"Test failed: {result}"


async def _test_mixed_calls(router_addr: str, test_log_dir: Path) -> str:
    """Test mixed sandbox and host calls."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Call 1: Sandbox bus (org.freedesktop.DBus.ListNames)
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='ListNames',
            )
        )
        if reply.message_type == MessageType.ERROR:
            return f"sandbox_call_1_error: {reply.body}"

        # Call 2: Host-routed service (org.a11y.Bus)
        # This mimics what VSCode does
        try:
            reply = await bus.call(
                Message(
                    destination='org.a11y.Bus',
                    path='/org/a11y/bus',
                    interface='org.freedesktop.DBus.Peer',
                    member='Ping',
                )
            )
        except Exception as e:
            error_str = str(e)
            error_type = type(e).__name__
            if "ServiceUnknown" in error_str or "NameHasNoOwner" in error_str:
                # Expected - service doesn't exist on host bus
                pass
            else:
                return f"host_call_error: {error_type}: {error_str}"

        # Call 3: Back to sandbox bus - should still work
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='GetId',
            )
        )
        if reply.message_type == MessageType.ERROR:
            return f"sandbox_call_2_error: {reply.body}"

        bus.disconnect()
        return "success"

    except Exception as e:
        error_str = str(e)
        (test_log_dir / "mixed_test_error.log").write_text(f"Error: {error_str}\n")
        return f"unexpected_error: {error_str}"
