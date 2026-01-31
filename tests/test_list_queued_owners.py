"""ListQueuedOwners integration tests.

Tests for ListQueuedOwners method which returns the list of connections
queued to own a name. The unique names should be rewritten with appropriate
prefixes (:s. for sandbox, :h. for host).
"""

import asyncio
from pathlib import Path

from dbus_next.aio import MessageBus
from dbus_next import Message, MessageType

# D-Bus spec values (dbus_next enum may differ)
DBUS_REQUEST_NAME_REPLY_PRIMARY_OWNER = 1
DBUS_REQUEST_NAME_REPLY_IN_QUEUE = 2
DBUS_REQUEST_NAME_REPLY_EXISTS = 3
DBUS_REQUEST_NAME_REPLY_ALREADY_OWNER = 4

EMPTY_CONFIG = ""


def test_list_queued_owners_single_owner(test_log_dir: Path, router_env):
    """ListQueuedOwners should return single owner with correct prefix."""
    with router_env(EMPTY_CONFIG, socket_prefix="lqo_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_list_queued_owners_single(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_list_queued_owners_single(router_addr: str, test_log_dir: Path) -> dict:
    """Test ListQueuedOwners with a single owner."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Request a name
        test_name = "org.test.QueuedOwners"
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RequestName',
                signature='su',
                body=[test_name, 0],  # 0 = no flags
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"RequestName failed: {reply.body}"}

        # Now query the queued owners
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='ListQueuedOwners',
                signature='s',
                body=[test_name],
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"ListQueuedOwners failed: {reply.body}"}

        owners = reply.body[0] if reply.body else []
        log_content = f"ListQueuedOwners({test_name}): {owners}\n"
        (test_log_dir / "queued_owners_single.log").write_text(log_content)

        if len(owners) != 1:
            return {"status": "error", "error": f"Expected 1 owner, got {len(owners)}"}

        # Verify the owner has :s. prefix (sandbox bus)
        owner = owners[0]
        if not owner.startswith(":s."):
            return {"status": "error", "error": f"Expected :s. prefix, got {owner}"}

        bus.disconnect()
        return {"status": "success", "owners": owners}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_list_queued_owners_multiple(test_log_dir: Path, router_env):
    """ListQueuedOwners should return multiple queued owners with correct prefixes."""
    with router_env(EMPTY_CONFIG, socket_prefix="lqo_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_list_queued_owners_multiple(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_list_queued_owners_multiple(router_addr: str, test_log_dir: Path) -> dict:
    """Test ListQueuedOwners with multiple queued connections."""
    try:
        # Connect first client and request name
        bus1 = await MessageBus(bus_address=router_addr).connect()
        bus2 = await MessageBus(bus_address=router_addr).connect()

        test_name = "org.test.QueuedMultiple"

        # First client requests name (will be primary owner)
        # DBUS_NAME_FLAG_ALLOW_REPLACEMENT = 0x1
        reply = await bus1.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RequestName',
                signature='su',
                body=[test_name, 0x1],  # ALLOW_REPLACEMENT
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"RequestName (bus1) failed: {reply.body}"}

        # Second client requests same name (will be queued)
        # DBUS_NAME_FLAG_DO_NOT_QUEUE = 0x4 (NOT set, so it will queue)
        reply = await bus2.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RequestName',
                signature='su',
                body=[test_name, 0],  # No flags, will queue
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"RequestName (bus2) failed: {reply.body}"}

        # Should get IN_QUEUE (2) reply
        request_reply = reply.body[0] if reply.body else 0
        if request_reply != DBUS_REQUEST_NAME_REPLY_IN_QUEUE:
            return {"status": "error", "error": f"Expected IN_QUEUE (2), got {request_reply}"}

        # Now query the queued owners
        reply = await bus1.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='ListQueuedOwners',
                signature='s',
                body=[test_name],
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"ListQueuedOwners failed: {reply.body}"}

        owners = reply.body[0] if reply.body else []
        log_content = f"ListQueuedOwners({test_name}): {owners}\n"
        log_content += f"bus1.unique_name: {bus1.unique_name}\n"
        log_content += f"bus2.unique_name: {bus2.unique_name}\n"
        (test_log_dir / "queued_owners_multiple.log").write_text(log_content)

        if len(owners) != 2:
            return {"status": "error", "error": f"Expected 2 owners, got {len(owners)}: {owners}"}

        # Verify all owners have :s. prefix (sandbox bus)
        for owner in owners:
            if not owner.startswith(":s."):
                return {"status": "error", "error": f"Expected :s. prefix, got {owner}"}

        # Verify our unique names are in the list
        if bus1.unique_name not in owners:
            return {"status": "error", "error": f"{bus1.unique_name} not in owners: {owners}"}
        if bus2.unique_name not in owners:
            return {"status": "error", "error": f"{bus2.unique_name} not in owners: {owners}"}

        bus1.disconnect()
        bus2.disconnect()
        return {"status": "success", "owners": owners}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_list_queued_owners_nonexistent(test_log_dir: Path, router_env):
    """ListQueuedOwners for non-existent name should return error."""
    with router_env(EMPTY_CONFIG, socket_prefix="lqo_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_list_queued_owners_nonexistent(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_list_queued_owners_nonexistent(router_addr: str, test_log_dir: Path) -> dict:
    """Test ListQueuedOwners for a name that doesn't exist."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Query non-existent name
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='ListQueuedOwners',
                signature='s',
                body=['org.test.NonExistent'],
            )
        )

        # Should get an error (name doesn't exist)
        if reply.message_type != MessageType.ERROR:
            return {"status": "error", "error": f"Expected error, got: {reply.body}"}

        bus.disconnect()
        return {"status": "success", "error_name": reply.error_name}

    except Exception as e:
        return {"status": "exception", "error": str(e)}
