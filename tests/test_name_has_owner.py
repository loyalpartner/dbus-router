"""NameHasOwner integration tests.

Tests for NameHasOwner method which should:
1. Route to correct bus based on name
2. Return correct boolean for owned/not owned names
3. Handle fake unique names correctly
"""

import asyncio
from pathlib import Path

from dbus_next.aio import MessageBus
from dbus_next import Message, MessageType

from utils.dbus_env import echo_service_session

EMPTY_CONFIG = ""

HOST_ROUTE_CONFIG = '''
[[host_routes]]
destination = "org.test.HostService"
'''


def test_name_has_owner_well_known(test_log_dir: Path, router_env):
    """NameHasOwner should return True for owned well-known names."""
    with router_env(EMPTY_CONFIG, socket_prefix="nho_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_name_has_owner_well_known(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_name_has_owner_well_known(router_addr: str, test_log_dir: Path) -> dict:
    """Test NameHasOwner for well-known names."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # org.freedesktop.DBus always has an owner
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='NameHasOwner',
                signature='s',
                body=['org.freedesktop.DBus'],
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"NameHasOwner failed: {reply.body}"}

        has_owner = reply.body[0] if reply.body else False
        if not has_owner:
            return {"status": "error", "error": "org.freedesktop.DBus should have owner"}

        # Non-existent name should return False
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='NameHasOwner',
                signature='s',
                body=['org.test.NonExistent'],
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"NameHasOwner failed: {reply.body}"}

        has_owner = reply.body[0] if reply.body else True
        if has_owner:
            return {"status": "error", "error": "Non-existent name should not have owner"}

        bus.disconnect()
        return {"status": "success"}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_name_has_owner_sandbox_service(test_log_dir: Path, router_env):
    """NameHasOwner should return True for sandbox services."""
    with router_env(EMPTY_CONFIG, socket_prefix="nho_") as env:
        _, _, router_addr = env
        with echo_service_session(router_addr, test_log_dir):
            result = asyncio.run(
                _test_name_has_owner_sandbox(router_addr, test_log_dir)
            )
            assert result["status"] == "success", f"Test failed: {result}"


async def _test_name_has_owner_sandbox(router_addr: str, test_log_dir: Path) -> dict:
    """Test NameHasOwner for sandbox services."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Check if our echo service has an owner
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='NameHasOwner',
                signature='s',
                body=['org.test.Echo'],
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"NameHasOwner failed: {reply.body}"}

        has_owner = reply.body[0] if reply.body else False
        bus.disconnect()

        if not has_owner:
            return {"status": "error", "error": "org.test.Echo should have owner"}

        return {"status": "success", "has_owner": has_owner}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_name_has_owner_org_freedesktop_dbus(test_log_dir: Path, router_env):
    """NameHasOwner should return True for org.freedesktop.DBus."""
    with router_env(EMPTY_CONFIG, socket_prefix="nho_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_name_has_owner_dbus(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_name_has_owner_dbus(router_addr: str, test_log_dir: Path) -> dict:
    """Test NameHasOwner for org.freedesktop.DBus."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # org.freedesktop.DBus always has an owner (the bus daemon itself)
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='NameHasOwner',
                signature='s',
                body=['org.freedesktop.DBus'],
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"NameHasOwner failed: {reply.body}"}

        has_owner = reply.body[0] if reply.body else False

        (test_log_dir / "dbus_name_owner.log").write_text(
            f"org.freedesktop.DBus has_owner: {has_owner}"
        )

        bus.disconnect()

        if not has_owner:
            return {"status": "error", "error": "org.freedesktop.DBus should have owner"}

        return {"status": "success", "has_owner": has_owner}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_name_has_owner_host_routed(test_log_dir: Path, router_env):
    """NameHasOwner for host-routed names should query host bus."""
    with router_env(HOST_ROUTE_CONFIG, socket_prefix="nho_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_name_has_owner_host_routed(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_name_has_owner_host_routed(router_addr: str, test_log_dir: Path) -> dict:
    """Test NameHasOwner routes to host bus for host_routes names."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Query host-routed service (doesn't exist, should return False)
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='NameHasOwner',
                signature='s',
                body=['org.test.HostService'],
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"NameHasOwner failed: {reply.body}"}

        has_owner = reply.body[0] if reply.body else True

        bus.disconnect()

        # Service doesn't exist on host bus
        if has_owner:
            return {"status": "error", "error": "org.test.HostService should not have owner"}

        return {"status": "success", "has_owner": has_owner}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_name_has_owner_fake_unique_name(test_log_dir: Path, router_env):
    """NameHasOwner should work for fake unique names (e.g., :s.1.0).

    This tests that the router correctly rewrites the body argument
    from :s.X.Y to :X.Y before querying the sandbox bus.
    """
    with router_env(EMPTY_CONFIG, socket_prefix="nho_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_name_has_owner_fake_unique(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_name_has_owner_fake_unique(router_addr: str, test_log_dir: Path) -> dict:
    """Test NameHasOwner for fake unique names."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Get our own unique name (which should be :s.X.Y from sandbox bus)
        my_name = bus.unique_name

        log_content = f"My unique name: {my_name}\n"

        # Verify our name starts with :s. (sandbox prefix)
        if not my_name.startswith(":s."):
            return {"status": "error", "error": f"Expected sandbox prefix :s., got {my_name}"}

        # Query if our own unique name has an owner
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='NameHasOwner',
                signature='s',
                body=[my_name],
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"NameHasOwner failed: {reply.body}"}

        has_owner = reply.body[0] if reply.body else False
        log_content += f"NameHasOwner({my_name}): {has_owner}\n"
        (test_log_dir / "fake_unique_name.log").write_text(log_content)

        bus.disconnect()

        if not has_owner:
            return {"status": "error", "error": f"{my_name} should have owner"}

        return {"status": "success", "unique_name": my_name, "has_owner": has_owner}

    except Exception as e:
        return {"status": "exception", "error": str(e)}
