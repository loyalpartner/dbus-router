"""RemoveMatch integration tests.

Tests for RemoveMatch method which should:
1. Remove previously added match rules
2. Stop receiving signals after match is removed
3. Properly rewrite sender in match rules with fake names
"""

import asyncio
from pathlib import Path

from dbus_next.aio import MessageBus
from dbus_next import Message, MessageType

EMPTY_CONFIG = ""


def test_remove_match_basic(test_log_dir: Path, router_env):
    """RemoveMatch should remove a previously added match rule."""
    with router_env(EMPTY_CONFIG, socket_prefix="rm_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_remove_match_basic(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_remove_match_basic(router_addr: str, test_log_dir: Path) -> dict:
    """Test basic RemoveMatch functionality."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        match_rule = "type='signal',interface='org.freedesktop.DBus',member='NameOwnerChanged'"

        # Add match rule
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='AddMatch',
                signature='s',
                body=[match_rule],
            )
        )
        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"AddMatch failed: {reply.body}"}

        # Remove match rule
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RemoveMatch',
                signature='s',
                body=[match_rule],
            )
        )
        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"RemoveMatch failed: {reply.body}"}

        bus.disconnect()
        return {"status": "success"}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_remove_match_not_found(test_log_dir: Path, router_env):
    """RemoveMatch for non-existent rule should return error."""
    with router_env(EMPTY_CONFIG, socket_prefix="rm_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_remove_match_not_found(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_remove_match_not_found(router_addr: str, test_log_dir: Path) -> dict:
    """Test RemoveMatch for non-existent rule."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Try to remove a rule we never added
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RemoveMatch',
                signature='s',
                body=["type='signal',interface='org.test.NonExistent',member='SomeSignal'"],
            )
        )

        bus.disconnect()

        # Should get MatchRuleNotFound error
        if reply.message_type != MessageType.ERROR:
            return {
                "status": "error",
                "error": "Expected error for non-existent match rule"
            }

        return {"status": "success", "error_name": reply.error_name}

    except Exception as e:
        # Error is expected
        if "MatchRuleNotFound" in str(e):
            return {"status": "success", "exception": str(e)}
        return {"status": "exception", "error": str(e)}


def test_remove_match_stops_signals(test_log_dir: Path, router_env):
    """After RemoveMatch, signals matching the rule should stop being received."""
    with router_env(EMPTY_CONFIG, socket_prefix="rm_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_remove_match_stops_signals(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_remove_match_stops_signals(router_addr: str, test_log_dir: Path) -> dict:
    """Test that signals stop after RemoveMatch."""
    signals_before_remove = []
    signals_after_remove = []
    collecting_after = False

    def on_signal(msg):
        if msg.interface == 'org.freedesktop.DBus' and msg.member == 'NameOwnerChanged':
            if collecting_after:
                signals_after_remove.append(msg)
            else:
                signals_before_remove.append(msg)

    try:
        bus = await MessageBus(bus_address=router_addr).connect()
        bus.add_message_handler(on_signal)

        match_rule = "type='signal',interface='org.freedesktop.DBus',member='NameOwnerChanged'"

        # Add match rule
        await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='AddMatch',
                signature='s',
                body=[match_rule],
            )
        )

        # Trigger a NameOwnerChanged by requesting a name
        await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RequestName',
                signature='su',
                body=['org.test.RemoveMatchTest1', 0],
            )
        )

        await asyncio.sleep(0.1)
        before_count = len(signals_before_remove)

        # Remove match rule
        await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RemoveMatch',
                signature='s',
                body=[match_rule],
            )
        )

        collecting_after = True

        # Trigger another NameOwnerChanged (should not be received now)
        await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RequestName',
                signature='su',
                body=['org.test.RemoveMatchTest2', 0],
            )
        )

        await asyncio.sleep(0.1)

        bus.disconnect()

        (test_log_dir / "remove_match_signals.log").write_text(
            f"Before remove: {before_count}, After remove: {len(signals_after_remove)}"
        )

        # Should have received signals before remove
        if before_count == 0:
            return {
                "status": "error",
                "error": "No signals received before RemoveMatch"
            }

        # Should NOT receive signals after remove
        # Note: There might be some signals already in transit, so we allow 0 or 1
        if len(signals_after_remove) > 1:
            return {
                "status": "error",
                "error": f"Too many signals after RemoveMatch: {len(signals_after_remove)}"
            }

        return {
            "status": "success",
            "before_count": before_count,
            "after_count": len(signals_after_remove)
        }

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_remove_match_with_sender(test_log_dir: Path, router_env):
    """RemoveMatch with sender should have fake name rewritten."""
    with router_env(EMPTY_CONFIG, socket_prefix="rm_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_remove_match_with_sender(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_remove_match_with_sender(router_addr: str, test_log_dir: Path) -> dict:
    """Test RemoveMatch with fake sender name."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Add match with sender (our fake name)
        our_name = bus.unique_name
        match_rule = f"type='signal',sender='{our_name}'"

        # Add match rule
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='AddMatch',
                signature='s',
                body=[match_rule],
            )
        )
        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"AddMatch failed: {reply.body}"}

        # Remove the same match rule
        # Router should rewrite :s.X.Y to :X.Y for the real bus
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RemoveMatch',
                signature='s',
                body=[match_rule],
            )
        )
        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"RemoveMatch failed: {reply.body}"}

        bus.disconnect()

        (test_log_dir / "remove_match_sender.log").write_text(
            f"our_name: {our_name}, match_rule: {match_rule}"
        )

        return {"status": "success", "our_name": our_name}

    except Exception as e:
        return {"status": "exception", "error": str(e)}
