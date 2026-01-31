"""ReleaseName integration tests.

Tests for:
1. Client releases a name and receives NameLost signal
2. Sandbox service tracking is updated on ReleaseName
3. NameOwnerChanged signal is sent when name is released
"""

import asyncio
from pathlib import Path

from dbus_next.aio import MessageBus
from dbus_next import Message, MessageType

EMPTY_CONFIG = ""


def test_release_name_basic(test_log_dir: Path, router_env):
    """Client can release a name it owns."""
    with router_env(EMPTY_CONFIG, socket_prefix="rn_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_release_name_basic(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_release_name_basic(router_addr: str, test_log_dir: Path) -> dict:
    """Test basic ReleaseName functionality."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Request a name
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RequestName',
                signature='su',
                body=['org.test.ReleaseTest', 0],
            )
        )
        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"RequestName failed: {reply.body}"}

        # Verify we got the name (return code 1 = PRIMARY_OWNER)
        if reply.body[0] != 1:
            return {"status": "error", "error": f"Unexpected RequestName result: {reply.body[0]}"}

        # Release the name
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='ReleaseName',
                signature='s',
                body=['org.test.ReleaseTest'],
            )
        )
        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"ReleaseName failed: {reply.body}"}

        # Return code 1 = RELEASED
        if reply.body[0] != 1:
            return {"status": "error", "error": f"Unexpected ReleaseName result: {reply.body[0]}"}

        bus.disconnect()
        return {"status": "success"}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_release_name_name_lost_signal(test_log_dir: Path, router_env):
    """NameLost signal should be received when releasing a name."""
    with router_env(EMPTY_CONFIG, socket_prefix="rn_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_name_lost_signal(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_name_lost_signal(router_addr: str, test_log_dir: Path) -> dict:
    """Test that NameLost signal is received on release."""
    name_lost_received = []

    def on_signal(msg):
        if msg.interface == 'org.freedesktop.DBus' and msg.member == 'NameLost':
            if msg.body:
                name_lost_received.append(msg.body[0])

    try:
        bus = await MessageBus(bus_address=router_addr).connect()
        bus.add_message_handler(on_signal)

        # Subscribe to NameLost
        await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='AddMatch',
                signature='s',
                body=["type='signal',interface='org.freedesktop.DBus',member='NameLost'"],
            )
        )

        # Request a name
        await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RequestName',
                signature='su',
                body=['org.test.NameLostTest', 0],
            )
        )

        # Release the name
        await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='ReleaseName',
                signature='s',
                body=['org.test.NameLostTest'],
            )
        )

        # Wait for signal
        await asyncio.sleep(0.2)

        bus.disconnect()

        (test_log_dir / "name_lost.log").write_text(
            f"NameLost signals: {name_lost_received}"
        )

        if 'org.test.NameLostTest' not in name_lost_received:
            return {
                "status": "error",
                "error": f"NameLost signal not received, got: {name_lost_received}"
            }

        return {"status": "success", "signals": name_lost_received}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_release_name_owner_changed(test_log_dir: Path, router_env):
    """NameOwnerChanged signal should be sent when name is released."""
    with router_env(EMPTY_CONFIG, socket_prefix="rn_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_release_owner_changed(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_release_owner_changed(router_addr: str, test_log_dir: Path) -> dict:
    """Test NameOwnerChanged on release has correct new_owner (empty)."""
    owner_changed_signals = []

    def on_signal(msg):
        if msg.interface == 'org.freedesktop.DBus' and msg.member == 'NameOwnerChanged':
            if msg.body and len(msg.body) >= 3:
                name, old_owner, new_owner = msg.body[0], msg.body[1], msg.body[2]
                if name == 'org.test.OwnerChangedTest':
                    owner_changed_signals.append({
                        "name": name,
                        "old_owner": old_owner,
                        "new_owner": new_owner,
                    })

    try:
        bus = await MessageBus(bus_address=router_addr).connect()
        bus.add_message_handler(on_signal)

        # Subscribe to NameOwnerChanged
        await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='AddMatch',
                signature='s',
                body=["type='signal',interface='org.freedesktop.DBus',member='NameOwnerChanged'"],
            )
        )

        # Request name
        await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RequestName',
                signature='su',
                body=['org.test.OwnerChangedTest', 0],
            )
        )

        await asyncio.sleep(0.1)

        # Release name
        await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='ReleaseName',
                signature='s',
                body=['org.test.OwnerChangedTest'],
            )
        )

        await asyncio.sleep(0.2)

        bus.disconnect()

        (test_log_dir / "owner_changed_release.log").write_text(
            "\n".join(str(s) for s in owner_changed_signals)
        )

        # Should have 2 signals: acquire (new_owner set) and release (new_owner empty)
        if len(owner_changed_signals) < 2:
            return {
                "status": "error",
                "error": f"Expected 2 NameOwnerChanged signals, got {len(owner_changed_signals)}",
                "signals": owner_changed_signals,
            }

        # Last signal should have empty new_owner (name released)
        last_signal = owner_changed_signals[-1]
        if last_signal["new_owner"] != "":
            return {
                "status": "error",
                "error": f"Release signal should have empty new_owner, got: {last_signal['new_owner']}",
                "signals": owner_changed_signals,
            }

        # old_owner should have prefix
        if not (last_signal["old_owner"].startswith(":s.") or last_signal["old_owner"].startswith(":h.")):
            return {
                "status": "error",
                "error": f"old_owner should have prefix, got: {last_signal['old_owner']}",
            }

        return {"status": "success", "signals": owner_changed_signals}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_release_name_not_owner(test_log_dir: Path, router_env):
    """ReleaseName for name we don't own should return NOT_OWNER."""
    with router_env(EMPTY_CONFIG, socket_prefix="rn_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_release_not_owner(router_addr)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_release_not_owner(router_addr: str) -> dict:
    """Test releasing a name we don't own."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Try to release a name we don't own
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='ReleaseName',
                signature='s',
                body=['org.test.NotOurName'],
            )
        )

        bus.disconnect()

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"Unexpected error: {reply.body}"}

        # Return code 3 = NOT_OWNER (we never owned it)
        # Return code 2 = NON_EXISTENT (name doesn't exist)
        if reply.body[0] not in [2, 3]:
            return {"status": "error", "error": f"Unexpected result: {reply.body[0]}"}

        return {"status": "success", "result": reply.body[0]}

    except Exception as e:
        return {"status": "exception", "error": str(e)}
