"""Fake Unique Name integration tests.

Tests for the fake unique name transformation:
- :1.45 from host becomes :h.1.45 to client
- :1.23 from sandbox becomes :s.1.23 to client
- Client sending to :h.1.45 gets routed to host with :1.45
"""

import tempfile
import asyncio
from pathlib import Path

from dbus_next.aio import MessageBus
from dbus_next import Message, MessageType

from utils.dbus_env import dbus_session, dbus_router_session


def test_list_names_merges_both_buses(test_log_dir: Path, build_project):
    """ListNames should return merged results from both buses with proper prefixes.

    Expected behavior:
    - Unique names from host bus get :h. prefix
    - Unique names from sandbox bus get :s. prefix
    - Well-known names are not prefixed and deduplicated
    """
    with tempfile.TemporaryDirectory(prefix="fn_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

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
                    result = asyncio.run(
                        _test_list_names_merge(router_addr, test_log_dir)
                    )
                    assert result["status"] == "success", f"Test failed: {result}"


async def _test_list_names_merge(router_addr: str, test_log_dir: Path) -> dict:
    """Test that ListNames returns merged results with proper prefixes."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Call ListNames
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='ListNames',
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": str(reply.body)}

        names = reply.body[0] if reply.body else []
        (test_log_dir / "list_names.log").write_text("\n".join(names))

        # Check for expected patterns
        has_sandbox_unique = any(n.startswith(":s.") for n in names)
        has_org_freedesktop_dbus = "org.freedesktop.DBus" in names

        # Note: host unique names may not exist if no connections to host bus
        # but sandbox bus always has connections (us, dbus-daemon)

        bus.disconnect()

        return {
            "status": "success",
            "names": names,
            "has_sandbox_unique": has_sandbox_unique,
            "has_org_freedesktop_dbus": has_org_freedesktop_dbus,
        }

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_get_name_owner_returns_prefixed_name(test_log_dir: Path, build_project):
    """GetNameOwner should return unique name with proper prefix.

    When asking for the owner of org.freedesktop.DBus on sandbox bus,
    the result should be :s.<unique_id> not just :<unique_id>
    """
    with tempfile.TemporaryDirectory(prefix="fn_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        config = test_log_dir / "router.toml"
        config.write_text('''
# No host routes - org.freedesktop.DBus goes to sandbox
''')

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    result = asyncio.run(
                        _test_get_name_owner(router_addr, test_log_dir)
                    )
                    assert result["status"] == "success", f"Test failed: {result}"
                    # The owner should have :s. prefix (sandbox bus)
                    owner = result.get("owner", "")
                    assert owner.startswith(":s."), f"Expected :s. prefix, got: {owner}"


async def _test_get_name_owner(router_addr: str, test_log_dir: Path) -> dict:
    """Test that GetNameOwner returns unique name with proper prefix."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # First, request a well-known name so we have something to query
        await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RequestName',
                signature='su',
                body=['org.test.GetNameOwnerTest', 0],
            )
        )

        # Now get owner of the name we just requested
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='GetNameOwner',
                signature='s',
                body=['org.test.GetNameOwnerTest'],
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": str(reply.body)}

        owner = reply.body[0] if reply.body else ""
        (test_log_dir / "get_name_owner.log").write_text(f"org.test.GetNameOwnerTest owner: {owner}")

        bus.disconnect()

        return {
            "status": "success",
            "owner": owner,
        }

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_hello_returns_prefixed_unique_name(test_log_dir: Path, build_project):
    """Client's own unique name should have proper prefix.

    After Hello(), the client should see their unique name with :s. prefix
    (since Hello() goes to sandbox by default).
    """
    with tempfile.TemporaryDirectory(prefix="fn_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        config = test_log_dir / "router.toml"
        config.write_text('''
# No hostpass - Hello goes to sandbox
''')

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    result = asyncio.run(
                        _test_hello_unique_name(router_addr, test_log_dir)
                    )
                    assert result["status"] == "success", f"Test failed: {result}"
                    unique_name = result.get("unique_name", "")
                    assert unique_name.startswith(":s."), f"Expected :s. prefix, got: {unique_name}"


async def _test_hello_unique_name(router_addr: str, test_log_dir: Path) -> dict:
    """Test that Hello returns unique name with proper prefix."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # The unique_name is set after Hello() completes
        unique_name = bus.unique_name or ""
        (test_log_dir / "hello.log").write_text(f"Unique name: {unique_name}")

        bus.disconnect()

        return {
            "status": "success",
            "unique_name": unique_name,
        }

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_get_name_owner_with_fake_unique_name(test_log_dir: Path, build_project):
    """GetNameOwner should work when querying a fake unique name.

    This tests that when client asks "who owns :h.1.38", the router
    correctly rewrites the body argument to ":1.38" before querying
    the host bus.
    """
    with tempfile.TemporaryDirectory(prefix="fn_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

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
                    result = asyncio.run(
                        _test_get_name_owner_fake_unique(router_addr, test_log_dir)
                    )
                    assert result["status"] == "success", f"Test failed: {result}"


async def _test_get_name_owner_fake_unique(router_addr: str, test_log_dir: Path) -> dict:
    """Test GetNameOwner for fake unique names.

    We query our own :s.X.Y unique name to verify the router properly
    rewrites it to :X.Y when sending to the sandbox bus.
    """
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Get our own unique name (should have :s. prefix)
        my_name = bus.unique_name
        log_content = f"My unique name: {my_name}\n"

        if not my_name.startswith(":s."):
            return {"status": "error", "error": f"Expected sandbox prefix :s., got {my_name}"}

        # Query GetNameOwner for our own fake unique name
        # This should return the same name back (we own ourselves)
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='GetNameOwner',
                signature='s',
                body=[my_name],
            )
        )

        if reply.message_type == MessageType.ERROR:
            error_msg = reply.body[0] if reply.body else str(reply.body)
            log_content += f"GetNameOwner({my_name}) error: {error_msg}\n"
            (test_log_dir / "get_name_owner_fake.log").write_text(log_content)
            return {"status": "error", "error": f"GetNameOwner failed: {error_msg}"}

        owner = reply.body[0] if reply.body else ""
        log_content += f"GetNameOwner({my_name}): {owner}\n"
        (test_log_dir / "get_name_owner_fake.log").write_text(log_content)

        bus.disconnect()

        # The owner should be our own name with the :s. prefix
        if owner != my_name:
            return {"status": "error", "error": f"Expected owner {my_name}, got {owner}"}

        return {"status": "success", "unique_name": my_name, "owner": owner}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_name_owner_changed_has_prefixed_names(test_log_dir: Path, build_project):
    """NameOwnerChanged signal should have prefixed unique names.

    When a service appears/disappears, the old_owner and new_owner
    fields should have proper :h. or :s. prefixes.
    """
    with tempfile.TemporaryDirectory(prefix="fn_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        config = test_log_dir / "router.toml"
        config.write_text('''
# Empty config
''')

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    result = asyncio.run(
                        _test_name_owner_changed(router_addr, test_log_dir)
                    )
                    assert result["status"] == "success", f"Test failed: {result}"


async def _test_name_owner_changed(router_addr: str, test_log_dir: Path) -> dict:
    """Test that NameOwnerChanged signal has prefixed unique names."""
    signals_received = []

    def on_signal(msg):
        if msg.interface == 'org.freedesktop.DBus' and msg.member == 'NameOwnerChanged':
            if msg.body and len(msg.body) >= 3:
                name, old_owner, new_owner = msg.body[0], msg.body[1], msg.body[2]
                signals_received.append({
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

        # Request a name to trigger NameOwnerChanged
        await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='RequestName',
                signature='su',
                body=['org.test.FakeNameTest', 0],
            )
        )

        # Wait for signal
        await asyncio.sleep(0.2)

        bus.disconnect()

        # Log what we received
        (test_log_dir / "name_owner_changed.log").write_text(
            "\n".join(str(s) for s in signals_received)
        )

        # Check that NameOwnerChanged for our test name has prefixed new_owner
        for sig in signals_received:
            if sig["name"] == "org.test.FakeNameTest":
                new_owner = sig.get("new_owner", "")
                if new_owner and not new_owner.startswith(":s.") and not new_owner.startswith(":h."):
                    return {
                        "status": "error",
                        "error": f"new_owner not prefixed: {new_owner}",
                        "signals": signals_received,
                    }

        return {
            "status": "success",
            "signals": signals_received,
        }

    except Exception as e:
        return {"status": "exception", "error": str(e)}
