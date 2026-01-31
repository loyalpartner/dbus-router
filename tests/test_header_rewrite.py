"""Header field rewriting integration tests.

Tests for D-Bus message header field parsing and rewriting:
- Correct handling of various D-Bus type signatures in header fields
- SENDER and DESTINATION rewriting with fake unique names
- No warnings/errors when parsing messages with standard header fields
"""

import asyncio
from pathlib import Path

from dbus_next.aio import MessageBus
from dbus_next import Message, MessageType

EMPTY_CONFIG = """
# Empty config - sandbox routing by default
"""


def test_sender_rewrite_in_method_return(test_log_dir: Path, router_env):
    """SENDER field in method return should be properly rewritten.

    When calling a method through the router, the reply's SENDER
    should have the appropriate prefix (:h. or :s.).
    """
    with router_env(EMPTY_CONFIG, socket_prefix="hr_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_sender_in_reply(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"
        # Check router logs for any warnings about header parsing
        router_log = test_log_dir / "router.stderr"
        if router_log.exists():
            log_content = router_log.read_text()
            # Should not have warnings about unknown header field types
            assert "Unknown header field type" not in log_content, \
                f"Found warning in router logs:\n{log_content}"


async def _test_sender_in_reply(router_addr: str, test_log_dir: Path) -> dict:
    """Test that SENDER in method reply is properly rewritten."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Call GetId which returns a string
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='GetId',
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": str(reply.body)}

        # The reply should have a sender with proper prefix
        sender = reply.sender or ""
        (test_log_dir / "get_id_sender.log").write_text(f"GetId reply sender: {sender}")

        bus.disconnect()

        # Sender should have :s. or :h. prefix (for unique names)
        # or be a well-known name like org.freedesktop.DBus
        if sender.startswith(":") and not (sender.startswith(":s.") or sender.startswith(":h.")):
            return {
                "status": "error",
                "error": f"Reply sender not prefixed: {sender}",
            }

        return {
            "status": "success",
            "sender": sender,
        }

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_multiple_header_fields_parsing(test_log_dir: Path, router_env):
    """Messages with multiple header fields should be parsed correctly.

    Standard D-Bus messages have multiple header fields:
    - PATH (o)
    - INTERFACE (s)
    - MEMBER (s)
    - DESTINATION (s)
    - SIGNATURE (g) - when body is present
    - REPLY_SERIAL (u) - for replies
    - SENDER (s) - added by daemon

    The router should handle all these types when scanning for SENDER/DESTINATION.
    """
    with router_env(EMPTY_CONFIG, socket_prefix="hr_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_various_methods(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_various_methods(router_addr: str, test_log_dir: Path) -> dict:
    """Call various D-Bus methods to exercise header field parsing."""
    results = []

    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Test 1: GetId - simple string return
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='GetId',
            )
        )
        results.append(("GetId", reply.message_type == MessageType.METHOD_RETURN))

        # Test 2: ListNames - returns array of strings
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='ListNames',
            )
        )
        results.append(("ListNames", reply.message_type == MessageType.METHOD_RETURN))

        # Test 3: NameHasOwner - takes string, returns boolean
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
        results.append(("NameHasOwner", reply.message_type == MessageType.METHOD_RETURN))

        # Test 4: GetConnectionUnixProcessID - returns uint32
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='GetConnectionUnixProcessID',
                signature='s',
                body=[bus.unique_name],
            )
        )
        results.append(("GetConnectionUnixProcessID", reply.message_type == MessageType.METHOD_RETURN))

        # Test 5: GetConnectionCredentials - returns a{sv} (dict with variants)
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='GetConnectionCredentials',
                signature='s',
                body=[bus.unique_name],
            )
        )
        results.append(("GetConnectionCredentials", reply.message_type == MessageType.METHOD_RETURN))

        bus.disconnect()

        # Log results
        log_lines = [f"{name}: {'OK' if ok else 'FAILED'}" for name, ok in results]
        (test_log_dir / "various_methods.log").write_text("\n".join(log_lines))

        # Check all passed
        failed = [name for name, ok in results if not ok]
        if failed:
            return {"status": "error", "error": f"Failed methods: {failed}"}

        return {"status": "success", "results": results}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_signal_sender_rewrite(test_log_dir: Path, router_env):
    """Signals should have their SENDER field properly rewritten.

    When a signal is emitted from the sandbox bus, the SENDER
    should have :s. prefix. When from host bus, :h. prefix.
    """
    with router_env(EMPTY_CONFIG, socket_prefix="hr_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_signal_sender(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_signal_sender(router_addr: str, test_log_dir: Path) -> dict:
    """Test that signal SENDER fields are properly prefixed."""
    signals = []

    def on_signal(msg):
        if msg.message_type == MessageType.SIGNAL:
            signals.append({
                "sender": msg.sender,
                "interface": msg.interface,
                "member": msg.member,
            })

    try:
        bus = await MessageBus(bus_address=router_addr).connect()
        bus.add_message_handler(on_signal)

        # Subscribe to NameOwnerChanged signals
        await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='AddMatch',
                signature='s',
                body=["type='signal',interface='org.freedesktop.DBus'"],
            )
        )

        # Trigger a signal by requesting a name
        await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RequestName',
                signature='su',
                body=['org.test.HeaderRewrite.Signal', 0],
            )
        )

        # Wait for signals
        await asyncio.sleep(0.3)

        bus.disconnect()

        # Log signals
        log_lines = [f"{s['interface']}.{s['member']} from {s['sender']}" for s in signals]
        (test_log_dir / "signals.log").write_text("\n".join(log_lines))

        # Check that signals from org.freedesktop.DBus have prefixed sender
        for sig in signals:
            sender = sig.get("sender", "")
            if sender and sender.startswith(":") and not (sender.startswith(":s.") or sender.startswith(":h.")):
                return {
                    "status": "error",
                    "error": f"Signal sender not prefixed: {sender}",
                    "signals": signals,
                }

        return {"status": "success", "signals": signals}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_introspect_with_complex_return(test_log_dir: Path, router_env):
    """Introspect returns large XML string, testing string handling.

    This exercises the router's ability to handle messages with
    large string payloads and verify header rewriting works correctly.
    """
    with router_env(EMPTY_CONFIG, socket_prefix="hr_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_introspect(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_introspect(router_addr: str, test_log_dir: Path) -> dict:
    """Test introspection which returns large XML strings."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Introspect the D-Bus daemon
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus.Introspectable',
                member='Introspect',
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": str(reply.body)}

        xml_data = reply.body[0] if reply.body else ""
        sender = reply.sender or ""

        (test_log_dir / "introspect.xml").write_text(xml_data[:1000] + "...")
        (test_log_dir / "introspect_sender.log").write_text(f"Sender: {sender}")

        bus.disconnect()

        # Verify sender is prefixed
        if sender and sender.startswith(":") and not (sender.startswith(":s.") or sender.startswith(":h.")):
            return {
                "status": "error",
                "error": f"Reply sender not prefixed: {sender}",
            }

        # Verify we got valid XML
        if not xml_data.startswith("<!DOCTYPE"):
            return {
                "status": "error",
                "error": "Invalid introspection XML",
            }

        return {
            "status": "success",
            "sender": sender,
            "xml_length": len(xml_data),
        }

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_no_header_parse_warnings(test_log_dir: Path, router_env):
    """Router should not emit warnings about unknown header field types.

    After the header parsing fix, standard D-Bus messages should
    be processed without warnings in the router logs.
    """
    config_text = '''
# Test various routing scenarios
[[host_routes]]
destination = "org.test.HostService"
'''

    with router_env(config_text, socket_prefix="hr_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_many_operations(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"

        # Check router logs for warnings
        router_log = test_log_dir / "router.stderr"
        if router_log.exists():
            log_content = router_log.read_text()
            # Should not have WARN level messages about header parsing
            warn_lines = [
                line for line in log_content.split("\n")
                if "WARN" in line and "header field" in line.lower()
            ]
            assert not warn_lines, \
                f"Found header field warnings in logs:\n" + "\n".join(warn_lines)


async def _test_many_operations(router_addr: str, test_log_dir: Path) -> dict:
    """Perform many D-Bus operations to exercise header parsing."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        operations = []

        # Various method calls with different signatures
        methods = [
            ("Hello", None, None),  # Already called by connect()
            ("GetId", None, None),
            ("ListNames", None, None),
            ("ListActivatableNames", None, None),
            ("NameHasOwner", "s", ["org.freedesktop.DBus"]),
            ("GetNameOwner", "s", ["org.freedesktop.DBus"]),
            ("GetConnectionUnixUser", "s", [bus.unique_name]),
            ("GetConnectionUnixProcessID", "s", [bus.unique_name]),
            ("GetConnectionCredentials", "s", [bus.unique_name]),
        ]

        for member, sig, body in methods:
            try:
                msg = Message(
                    destination='org.freedesktop.DBus',
                    path='/org/freedesktop/DBus',
                    interface='org.freedesktop.DBus',
                    member=member,
                )
                if sig:
                    msg = Message(
                        destination='org.freedesktop.DBus',
                        path='/org/freedesktop/DBus',
                        interface='org.freedesktop.DBus',
                        member=member,
                        signature=sig,
                        body=body,
                    )
                reply = await bus.call(msg)
                operations.append((member, reply.message_type == MessageType.METHOD_RETURN))
            except Exception as e:
                operations.append((member, False, str(e)))

        # Request and release a name (more header field variations)
        await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RequestName',
                signature='su',
                body=['org.test.HeaderParse.Test', 0],
            )
        )
        operations.append(("RequestName", True))

        await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='ReleaseName',
                signature='s',
                body=['org.test.HeaderParse.Test'],
            )
        )
        operations.append(("ReleaseName", True))

        bus.disconnect()

        # Log results
        log_lines = [f"{op[0]}: {'OK' if op[1] else 'FAILED'}" for op in operations]
        (test_log_dir / "operations.log").write_text("\n".join(log_lines))

        return {"status": "success", "operations": operations}

    except Exception as e:
        return {"status": "exception", "error": str(e)}
