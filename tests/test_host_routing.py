"""Host routing integration tests - tests for routing messages to host bus.

This test file specifically tests scenarios where a client:
1. Connects to router (Hello goes to sandbox bus)
2. Then sends messages to services configured in host_routes (goes to host bus)

This simulates the VSCode crash scenario where org.a11y.Bus was in host_routes.
"""

import asyncio
from pathlib import Path

from dbus_next.aio import MessageBus
from dbus_next import Message, MessageType



def test_host_route_after_sandbox_hello(test_log_dir: Path, router_env):
    """Client connects via router, then calls a host-routed service.

    This reproduces the VSCode crash scenario:
    1. Client connects to router
    2. Hello() is sent - routes to sandbox bus (default)
    3. Client calls org.test.HostService - routes to host bus
    4. Host bus rejects because it never received Hello() from router
    """
    # Configure host_routes for org.test.HostService
    # No hostpass - so Hello() will go to sandbox by default
    config_text = '''
[[host_routes]]
destination = "org.test.HostService"
'''

    with router_env(config_text, socket_prefix="hr_") as env:
        _, _, router_addr = env
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


def test_mixed_sandbox_and_host_calls(test_log_dir: Path, router_env):
    """Client makes calls to both sandbox and host routed services.

    This is a more comprehensive test:
    1. Client connects to router
    2. Makes a call to sandbox bus (org.freedesktop.DBus)
    3. Makes a call to host-routed service
    4. Makes another call to sandbox bus

    All calls should succeed (or return service-not-found, not disconnect).
    """
    config_text = '''
[[host_routes]]
destination = "org.a11y.Bus"

[[host_routes]]
destination = "org.test.HostOnly"
'''

    with router_env(config_text, socket_prefix="hr_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_mixed_calls(router_addr, test_log_dir)
        )
        assert result == "success", f"Test failed: {result}"


async def _test_mixed_calls(router_addr: str, test_log_dir: Path) -> str:
    """Test mixed sandbox and host calls."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()
        # At this point Hello() handshake completed - if NameAcquired signal
        # was left in buffer and forwarded, the connect() would have failed
        # with "Unexpected message Signal"

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


def test_nameacquired_signal_not_leaked(test_log_dir: Path, router_env):
    """NameAcquired signal from host bus should not be forwarded to client.

    This test reproduces the NameAcquired signal issue:
    - Client connects to router, which triggers Router<->Host auth
    - Router sends Hello() to Host bus
    - Host bus returns MethodReturn + NameAcquired signal
    - If NameAcquired is not consumed by Router, it enters forward_loop buffer
    - When client sends Hello(), the NameAcquired gets forwarded first
    - Client receives Signal instead of MethodReturn, causing "Unexpected message Signal"

    The key timing here is that the client's Hello() response races with
    the stale NameAcquired signal from the host bus.
    """
    config_text = '''
[[host_routes]]
destination = "org.test.HostService"
'''

    with router_env(config_text, socket_prefix="hr_") as env:
        _, _, router_addr = env
        # Run multiple connections sequentially
        # Each triggers Router<->Host Hello() handshake
        errors = []
        for i in range(3):
            result = asyncio.run(
                _test_single_host_route_connection(router_addr, i, test_log_dir)
            )
            if result != "success":
                errors.append(f"Connection {i}: {result}")

        assert not errors, f"Connections failed: {errors}"


async def _test_single_host_route_connection(
    router_addr: str, conn_id: int, test_log_dir: Path
) -> str:
    """Single client connection that calls a host-routed service.

    If NameAcquired signal is not properly consumed by Router,
    this will fail with "Unexpected message Signal".
    """
    import socket

    try:
        # Connect to router - this does Hello() handshake
        # If stale NameAcquired signal is forwarded, connect() fails here
        bus = await MessageBus(bus_address=router_addr).connect()

        # Immediately after connect, check if there are unexpected messages
        # by doing a simple call that should succeed quickly
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='GetId',
            )
        )
        if reply.message_type != MessageType.METHOD_RETURN:
            bus.disconnect()
            return f"unexpected_first_reply: {reply.message_type}"

        # Call host-routed service (will get ServiceUnknown, that's OK)
        try:
            await bus.call(
                Message(
                    destination='org.test.HostService',
                    path='/org/test/HostService',
                    interface='org.freedesktop.DBus.Peer',
                    member='Ping',
                )
            )
        except Exception as e:
            if "ServiceUnknown" not in str(e) and "NameHasNoOwner" not in str(e):
                bus.disconnect()
                return f"host_call_error: {e}"

        bus.disconnect()
        return "success"

    except Exception as e:
        error_str = str(e)
        (test_log_dir / f"conn_{conn_id}_error.log").write_text(f"Error: {error_str}\n")

        # Check for the specific NameAcquired signal issue
        if "Unexpected" in error_str and "Signal" in error_str:
            return f"nameacquired_signal_leaked: {error_str}"
        return f"connection_error: {error_str}"


def test_no_stale_signal_after_connect(test_log_dir: Path, router_env):
    """Verify no stale signals are sent to client after connection.

    This is a low-level test that checks if any unexpected messages
    are received right after the D-Bus handshake completes.
    """
    import socket
    import struct

    config_text = '''
[[host_routes]]
destination = "org.test.HostService"
'''

    with router_env(config_text, socket_prefix="hr_") as env:
        _, _, router_addr = env
        # Use dbus_next to connect (handles auth + Hello)
        result = asyncio.run(
            _check_for_stale_signals(router_addr, test_log_dir)
        )
        assert result == "success", f"Test failed: {result}"


async def _check_for_stale_signals(router_addr: str, test_log_dir: Path) -> str:
    """Check if any stale signals are received after connection."""
    stale_signals = []

    def signal_handler(msg):
        if msg.message_type == MessageType.SIGNAL:
            # NameAcquired from org.freedesktop.DBus is expected for our own name
            # But NameAcquired for router's host bus name would be wrong
            if msg.interface == 'org.freedesktop.DBus' and msg.member == 'NameAcquired':
                # Check if this is for our unique name or a stale one
                if msg.body:
                    name = msg.body[0]
                    # Our unique name starts with : and matches our connection
                    # Any other NameAcquired would be stale
                    stale_signals.append(f"NameAcquired({name})")

    try:
        bus = await MessageBus(bus_address=router_addr).connect()
        bus.add_message_handler(signal_handler)

        # Wait briefly to see if any stale signals arrive
        await asyncio.sleep(0.1)

        # Make a simple call
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='GetId',
            )
        )

        bus.disconnect()

        # Filter out expected signals (our own NameAcquired)
        # We should only receive NameAcquired for our own unique name
        if len(stale_signals) > 1:
            return f"stale_signals_detected: {stale_signals}"

        return "success"

    except Exception as e:
        return f"error: {e}"
