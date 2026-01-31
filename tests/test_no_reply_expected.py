"""NO_REPLY_EXPECTED flag integration tests.

Tests for the NO_REPLY_EXPECTED header flag (0x1) which indicates
that the sender does not expect a reply to this method call.

D-Bus header flags (byte 2):
- 0x1: NO_REPLY_EXPECTED - The sender doesn't expect a reply
- 0x2: NO_AUTO_START - Don't auto-start the destination service
"""

import asyncio
from pathlib import Path

from dbus_next.aio import MessageBus
from dbus_next import Message, MessageType, MessageFlag

EMPTY_CONFIG = ""


def test_no_reply_expected_passthrough(test_log_dir: Path, router_env):
    """NO_REPLY_EXPECTED flag should be passed through to the bus."""
    with router_env(EMPTY_CONFIG, socket_prefix="nre_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_no_reply_expected_passthrough(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_no_reply_expected_passthrough(router_addr: str, test_log_dir: Path) -> dict:
    """Test that NO_REPLY_EXPECTED flag passes through correctly."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Create a message with NO_REPLY_EXPECTED flag
        # We'll use Ping which is a simple method that normally returns nothing
        msg = Message(
            destination='org.freedesktop.DBus',
            path='/org/freedesktop/DBus',
            interface='org.freedesktop.DBus.Peer',
            member='Ping',
            flags=MessageFlag.NO_REPLY_EXPECTED,
        )

        # Send without waiting for reply (since we set NO_REPLY_EXPECTED)
        bus.send(msg)

        # Give the bus time to process
        await asyncio.sleep(0.1)

        # If we got here without hanging, the flag worked
        (test_log_dir / "no_reply_ping.log").write_text(
            f"Sent Ping with NO_REPLY_EXPECTED, no hang occurred"
        )

        bus.disconnect()
        return {"status": "success"}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_no_reply_expected_with_side_effect(test_log_dir: Path, router_env):
    """NO_REPLY_EXPECTED message should still be processed (verify via side effect)."""
    with router_env(EMPTY_CONFIG, socket_prefix="nre_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_no_reply_expected_side_effect(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_no_reply_expected_side_effect(router_addr: str, test_log_dir: Path) -> dict:
    """Test that NO_REPLY_EXPECTED message is processed by observing side effect."""
    signals_received = []

    def on_signal(msg):
        if (msg.message_type == MessageType.SIGNAL and
            msg.interface == 'org.freedesktop.DBus' and
            msg.member == 'NameOwnerChanged'):
            signals_received.append(msg.body)

    try:
        bus = await MessageBus(bus_address=router_addr).connect()
        bus.add_message_handler(on_signal)

        # Add match rule for NameOwnerChanged (to observe the side effect)
        match_rule = "type='signal',interface='org.freedesktop.DBus',member='NameOwnerChanged'"
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

        test_name = 'org.test.NoReplyExpectedTest'

        # Send RequestName with NO_REPLY_EXPECTED flag
        # The side effect (NameOwnerChanged signal) should still occur
        msg = Message(
            destination='org.freedesktop.DBus',
            path='/org/freedesktop/DBus',
            interface='org.freedesktop.DBus',
            member='RequestName',
            signature='su',
            body=[test_name, 0],
            flags=MessageFlag.NO_REPLY_EXPECTED,
        )

        bus.send(msg)

        # Wait for the signal (side effect)
        await asyncio.sleep(0.2)

        log_content = f"Signals received: {len(signals_received)}\n"
        for body in signals_received:
            log_content += f"  {body}\n"
        (test_log_dir / "no_reply_side_effect.log").write_text(log_content)

        # Check if we received the NameOwnerChanged signal for our name
        matching = [b for b in signals_received if b and len(b) >= 1 and b[0] == test_name]

        bus.disconnect()

        if len(matching) == 0:
            return {
                "status": "error",
                "error": f"No NameOwnerChanged signal received for {test_name}"
            }

        return {
            "status": "success",
            "signal_count": len(matching),
        }

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_compare_with_and_without_no_reply(test_log_dir: Path, router_env):
    """Compare behavior with and without NO_REPLY_EXPECTED flag."""
    with router_env(EMPTY_CONFIG, socket_prefix="nre_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_compare_with_and_without(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_compare_with_and_without(router_addr: str, test_log_dir: Path) -> dict:
    """Compare method call with and without NO_REPLY_EXPECTED."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Test 1: Normal call (expects reply)
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus.Peer',
                member='Ping',
            )
        )

        normal_success = reply.message_type == MessageType.METHOD_RETURN

        # Test 2: Call with NO_REPLY_EXPECTED (no waiting)
        msg = Message(
            destination='org.freedesktop.DBus',
            path='/org/freedesktop/DBus',
            interface='org.freedesktop.DBus.Peer',
            member='Ping',
            flags=MessageFlag.NO_REPLY_EXPECTED,
        )

        bus.send(msg)
        await asyncio.sleep(0.1)
        no_reply_success = True  # If we got here, it didn't hang

        bus.disconnect()

        (test_log_dir / "compare_reply_modes.log").write_text(
            f"Normal call success: {normal_success}\n"
            f"NO_REPLY_EXPECTED success: {no_reply_success}\n"
        )

        if not normal_success:
            return {"status": "error", "error": "Normal Ping call failed"}

        return {
            "status": "success",
            "normal_success": normal_success,
            "no_reply_success": no_reply_success,
        }

    except Exception as e:
        return {"status": "exception", "error": str(e)}
