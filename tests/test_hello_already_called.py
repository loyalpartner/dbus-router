"""Test for the 'Hello() already called' bug fix.

This test reproduces the issue where hostpass clients would get a
"Hello() already called" error from the host bus.

Root cause:
- D-Bus requires every connection to call Hello() before sending other messages
- If the router calls Hello() on the host bus, and then a hostpass client's
  Hello() is forwarded to the same host bus connection, it fails with
  "Hello() already called"

Fix:
- For hostpass clients: Router skips Hello() on host bus (client's Hello()
  will be forwarded)
- For non-hostpass clients: Router sends Hello() (needed for host_routes)
"""

import asyncio
from pathlib import Path

import pytest
from dbus_next import Message, MessageType
from dbus_next.aio import MessageBus
from dbus_next.errors import DBusError

HOSTPASS_CONFIG = '''
[[hostpass]]
process = "*/python3*"
'''

HOST_ROUTE_CONFIG = '''
[[host_routes]]
destination = "org.test.HostService"
'''


async def _connect_and_get_unique_name(router_addr: str) -> str:
    """Connect to router and return unique name (tests Hello() works)."""
    bus = await MessageBus(bus_address=router_addr).connect()
    unique_name = bus.unique_name
    bus.disconnect()
    return unique_name


async def _call_list_names(router_addr: str) -> list:
    """Call ListNames on org.freedesktop.DBus."""
    bus = await MessageBus(bus_address=router_addr).connect()
    reply = await bus.call(
        Message(
            destination="org.freedesktop.DBus",
            path="/org/freedesktop/DBus",
            interface="org.freedesktop.DBus",
            member="ListNames",
        )
    )
    bus.disconnect()
    if reply.message_type == MessageType.METHOD_RETURN:
        return reply.body[0]
    raise Exception(f"Unexpected reply: {reply}")


async def _connect_multiple_clients(router_addr: str, count: int) -> list:
    """Connect multiple clients and return their unique names."""
    unique_names = []
    buses = []
    try:
        for _ in range(count):
            bus = await MessageBus(bus_address=router_addr).connect()
            buses.append(bus)
            unique_names.append(bus.unique_name)
    finally:
        for bus in buses:
            bus.disconnect()
    return unique_names


def test_hostpass_hello_not_duplicated(test_log_dir: Path, router_env):
    """Hostpass client should not get 'Hello() already called' error.

    This test verifies that when a hostpass client connects to the router
    and sends Hello(), it doesn't get an error because the router should
    NOT have sent Hello() on the host bus for this client.
    """
    with router_env(HOSTPASS_CONFIG, socket_prefix="hello_") as env:
        _, _, router_addr = env
        # Connect as a hostpass client - this should work
        # If the bug exists, this would fail with "Hello() already called"
        try:
            unique_name = asyncio.run(
                _connect_and_get_unique_name(router_addr)
            )
            # If we get here, Hello() succeeded
            assert unique_name is not None
            # Check for the "Already handled" error message in unique_name
            # (dbus-next may return error text as unique_name in some cases)
            if "already" in unique_name.lower() or "hello" in unique_name.lower():
                pytest.fail(
                    f"Got 'Hello() already called' error - "
                    f"router should not send Hello() for hostpass clients: {unique_name}"
                )
            assert unique_name.startswith(":"), f"Expected unique name starting with ':', got: {unique_name}"
        except DBusError as e:
            # This is the bug we're testing for
            if "already" in str(e).lower() or "hello" in str(e).lower():
                pytest.fail(
                    f"Got 'Hello() already called' error - "
                    f"router should not send Hello() for hostpass clients: {e}"
                )
            raise


def test_non_hostpass_can_use_host_routes(test_log_dir: Path, router_env):
    """Non-hostpass client should be able to call host_routes services.

    This test verifies that for non-hostpass clients, the router sends
    Hello() on the host bus so that host_routes messages can be routed.
    """
    config_text = '''
[[host_routes]]
destination = "org.freedesktop.DBus"
'''

    with router_env(config_text, socket_prefix="hello_") as env:
        _, _, router_addr = env
        # Connect as non-hostpass client and call host bus
        # If router didn't send Hello(), host bus would disconnect
        try:
            names = asyncio.run(_call_list_names(router_addr))
            assert isinstance(names, list)
        except Exception as e:
            pytest.fail(
                f"Failed to call host bus - router should have sent Hello(): {e}"
            )


def test_multiple_hostpass_clients(test_log_dir: Path, router_env):
    """Multiple hostpass clients should each get their own connection.

    Each hostpass client connection should work independently, with
    each client's Hello() being forwarded to the host bus.
    """
    with router_env(HOSTPASS_CONFIG, socket_prefix="hello_") as env:
        _, _, router_addr = env
        # Connect multiple clients
        try:
            unique_names = asyncio.run(
                _connect_multiple_clients(router_addr, 3)
            )
            # Each client should have a unique name
            assert len(set(unique_names)) == 3
        except DBusError as e:
            if "already called" in str(e).lower():
                pytest.fail(
                    f"Got 'Hello() already called' error: {e}"
                )
            raise
