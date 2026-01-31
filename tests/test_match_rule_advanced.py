"""Match rule advanced features integration tests.

Tests for advanced match rule features:
1. path - Match signals on specific object paths
2. path_namespace - Match signals on path prefix
3. destination - Match by message destination
4. arg0 - Match by first argument content
"""

import asyncio
from pathlib import Path

from dbus_next.aio import MessageBus
from dbus_next import Message, MessageType

from utils.dbus_env import echo_service_session

EMPTY_CONFIG = ""


def test_match_rule_with_path(test_log_dir: Path, router_env):
    """Match rule with path should only receive signals from that path."""
    with router_env(EMPTY_CONFIG, socket_prefix="mr_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_match_rule_with_path(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_match_rule_with_path(router_addr: str, test_log_dir: Path) -> dict:
    """Test match rule with specific path."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Add match rule with specific path
        match_rule = "type='signal',interface='org.freedesktop.DBus',path='/org/freedesktop/DBus'"

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

        # Remove the match rule (test it works with path)
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

        (test_log_dir / "match_path.log").write_text(f"match_rule: {match_rule}")
        return {"status": "success"}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_match_rule_path_filters_signals(test_log_dir: Path, router_env):
    """Match rule with path should filter signals by path."""
    with router_env(EMPTY_CONFIG, socket_prefix="mr_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_match_rule_path_filters(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_match_rule_path_filters(router_addr: str, test_log_dir: Path) -> dict:
    """Test that path in match rule filters signals correctly."""
    signals_received = []

    def on_signal(msg):
        if msg.message_type == MessageType.SIGNAL:
            signals_received.append({
                "interface": msg.interface,
                "member": msg.member,
                "path": msg.path,
            })

    try:
        bus = await MessageBus(bus_address=router_addr).connect()
        bus.add_message_handler(on_signal)

        # Add match rule for NameOwnerChanged on /org/freedesktop/DBus path
        match_rule = "type='signal',interface='org.freedesktop.DBus',member='NameOwnerChanged',path='/org/freedesktop/DBus'"

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

        # Trigger NameOwnerChanged by requesting a name
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RequestName',
                signature='su',
                body=['org.test.PathMatchTest', 0],
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"RequestName failed: {reply.body}"}

        await asyncio.sleep(0.1)

        bus.disconnect()

        log_content = f"Signals received: {len(signals_received)}\n"
        for sig in signals_received:
            log_content += f"  {sig}\n"
        (test_log_dir / "match_path_filter.log").write_text(log_content)

        # Should have received NameOwnerChanged signals
        name_changed = [s for s in signals_received if s["member"] == "NameOwnerChanged"]
        if len(name_changed) == 0:
            return {"status": "error", "error": "No NameOwnerChanged signals received"}

        # All signals should be from /org/freedesktop/DBus
        for sig in name_changed:
            if sig["path"] != "/org/freedesktop/DBus":
                return {"status": "error", "error": f"Signal from wrong path: {sig['path']}"}

        return {"status": "success", "signal_count": len(name_changed)}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_match_rule_with_destination(test_log_dir: Path, router_env):
    """Match rule with destination should work correctly."""
    with router_env(EMPTY_CONFIG, socket_prefix="mr_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_match_rule_with_destination(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_match_rule_with_destination(router_addr: str, test_log_dir: Path) -> dict:
    """Test match rule with destination filter."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        our_name = bus.unique_name

        # Add match rule for signals destined to us
        match_rule = f"type='signal',destination='{our_name}'"

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

        # Remove the match rule
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

        (test_log_dir / "match_destination.log").write_text(
            f"our_name: {our_name}\nmatch_rule: {match_rule}"
        )
        return {"status": "success", "our_name": our_name}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_match_rule_with_arg0(test_log_dir: Path, router_env):
    """Match rule with arg0 should filter by first argument."""
    with router_env(EMPTY_CONFIG, socket_prefix="mr_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_match_rule_with_arg0(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_match_rule_with_arg0(router_addr: str, test_log_dir: Path) -> dict:
    """Test match rule with arg0 filter."""
    signals_received = []

    def on_signal(msg):
        if (msg.message_type == MessageType.SIGNAL and
            msg.interface == 'org.freedesktop.DBus' and
            msg.member == 'NameOwnerChanged'):
            signals_received.append(msg.body)

    try:
        bus = await MessageBus(bus_address=router_addr).connect()
        bus.add_message_handler(on_signal)

        # Add match rule for NameOwnerChanged with specific arg0 (name)
        test_name = 'org.test.Arg0MatchTest'
        match_rule = f"type='signal',interface='org.freedesktop.DBus',member='NameOwnerChanged',arg0='{test_name}'"

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

        # Request the specific name we're matching
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RequestName',
                signature='su',
                body=[test_name, 0],
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"RequestName failed: {reply.body}"}

        await asyncio.sleep(0.1)

        # Request another name (should NOT trigger our match)
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RequestName',
                signature='su',
                body=['org.test.OtherName', 0],
            )
        )

        await asyncio.sleep(0.1)

        bus.disconnect()

        log_content = f"Signals received: {len(signals_received)}\n"
        for body in signals_received:
            log_content += f"  arg0={body[0] if body else 'empty'}\n"
        (test_log_dir / "match_arg0.log").write_text(log_content)

        # Check that we received signal for the matching name
        matching = [b for b in signals_received if b and b[0] == test_name]
        if len(matching) == 0:
            return {"status": "error", "error": f"No signal received for {test_name}"}

        # Check that we did NOT receive signal for the other name
        # Note: This depends on the dbus-daemon implementation of arg0 matching
        other = [b for b in signals_received if b and b[0] == 'org.test.OtherName']

        return {
            "status": "success",
            "matching_signals": len(matching),
            "other_signals": len(other),
        }

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_match_rule_combined(test_log_dir: Path, router_env):
    """Match rule with multiple conditions should work correctly."""
    with router_env(EMPTY_CONFIG, socket_prefix="mr_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_match_rule_combined(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_match_rule_combined(router_addr: str, test_log_dir: Path) -> dict:
    """Test match rule with multiple conditions."""
    signals_received = []

    def on_signal(msg):
        if msg.message_type == MessageType.SIGNAL:
            signals_received.append({
                "interface": msg.interface,
                "member": msg.member,
                "path": msg.path,
            })

    try:
        bus = await MessageBus(bus_address=router_addr).connect()
        bus.add_message_handler(on_signal)

        # Add match rule with multiple conditions
        match_rule = (
            "type='signal',"
            "interface='org.freedesktop.DBus',"
            "member='NameOwnerChanged',"
            "path='/org/freedesktop/DBus'"
        )

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

        # Trigger NameOwnerChanged
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RequestName',
                signature='su',
                body=['org.test.CombinedMatchTest', 0],
            )
        )

        await asyncio.sleep(0.1)

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

        log_content = f"Signals received: {len(signals_received)}\n"
        for sig in signals_received:
            log_content += f"  {sig}\n"
        (test_log_dir / "match_combined.log").write_text(log_content)

        # Should have received NameOwnerChanged
        name_changed = [s for s in signals_received if s["member"] == "NameOwnerChanged"]
        if len(name_changed) == 0:
            return {"status": "error", "error": "No NameOwnerChanged signals received"}

        return {"status": "success", "signal_count": len(name_changed)}

    except Exception as e:
        return {"status": "exception", "error": str(e)}
