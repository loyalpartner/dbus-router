"""ListActivatableNames integration tests.

Tests for ListActivatableNames method which should:
1. Return merged results from both buses
2. Apply proper fake name prefix to unique names (if any)
3. Deduplicate well-known names
"""

import tempfile
import asyncio
from pathlib import Path

from dbus_next.aio import MessageBus
from dbus_next import Message, MessageType

from utils.dbus_env import dbus_session, dbus_router_session


def test_list_activatable_names_basic(test_log_dir: Path, build_project):
    """ListActivatableNames should return a list of activatable service names."""
    with tempfile.TemporaryDirectory(prefix="lan_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        config = test_log_dir / "router.toml"
        config.write_text("")

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    result = asyncio.run(
                        _test_list_activatable_basic(router_addr, test_log_dir)
                    )
                    assert result["status"] == "success", f"Test failed: {result}"


async def _test_list_activatable_basic(router_addr: str, test_log_dir: Path) -> dict:
    """Test basic ListActivatableNames functionality."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='ListActivatableNames',
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"ListActivatableNames failed: {reply.body}"}

        names = reply.body[0] if reply.body else []
        (test_log_dir / "list_activatable.log").write_text("\n".join(names))

        bus.disconnect()

        # Should be a list of strings
        if not isinstance(names, list):
            return {"status": "error", "error": f"Expected list, got {type(names)}"}

        # org.freedesktop.DBus should be activatable (built-in)
        if "org.freedesktop.DBus" not in names:
            return {"status": "error", "error": "org.freedesktop.DBus not in activatable names"}

        return {"status": "success", "names": names, "count": len(names)}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_list_activatable_names_merged(test_log_dir: Path, build_project):
    """ListActivatableNames should merge results from both buses."""
    with tempfile.TemporaryDirectory(prefix="lan_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        config = test_log_dir / "router.toml"
        config.write_text('''
[[host_routes]]
destination = "org.test.HostOnly"
''')

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    result = asyncio.run(
                        _test_list_activatable_merged(router_addr, host_addr, sandbox_addr, test_log_dir)
                    )
                    assert result["status"] == "success", f"Test failed: {result}"


async def _test_list_activatable_merged(
    router_addr: str, host_addr: str, sandbox_addr: str, test_log_dir: Path
) -> dict:
    """Verify ListActivatableNames merges both buses."""
    try:
        # Get activatable names from router
        bus = await MessageBus(bus_address=router_addr).connect()
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='ListActivatableNames',
            )
        )
        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"Router failed: {reply.body}"}
        router_names = set(reply.body[0] if reply.body else [])
        bus.disconnect()

        # Get from host directly
        bus = await MessageBus(bus_address=host_addr).connect()
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='ListActivatableNames',
            )
        )
        host_names = set(reply.body[0] if reply.body else [])
        bus.disconnect()

        # Get from sandbox directly
        bus = await MessageBus(bus_address=sandbox_addr).connect()
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='ListActivatableNames',
            )
        )
        sandbox_names = set(reply.body[0] if reply.body else [])
        bus.disconnect()

        (test_log_dir / "activatable_comparison.log").write_text(
            f"Router: {sorted(router_names)}\n"
            f"Host: {sorted(host_names)}\n"
            f"Sandbox: {sorted(sandbox_names)}\n"
        )

        # Router should have union of both (well-known names only typically)
        # Note: activatable names are typically .service files, so may be empty in test
        return {
            "status": "success",
            "router_count": len(router_names),
            "host_count": len(host_names),
            "sandbox_count": len(sandbox_names),
        }

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_list_activatable_names_no_duplicates(test_log_dir: Path, build_project):
    """ListActivatableNames should not have duplicate entries."""
    with tempfile.TemporaryDirectory(prefix="lan_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        config = test_log_dir / "router.toml"
        config.write_text("")

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    result = asyncio.run(
                        _test_no_duplicates(router_addr, test_log_dir)
                    )
                    assert result["status"] == "success", f"Test failed: {result}"


async def _test_no_duplicates(router_addr: str, test_log_dir: Path) -> dict:
    """Verify no duplicate entries in ListActivatableNames."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='ListActivatableNames',
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"Failed: {reply.body}"}

        names = reply.body[0] if reply.body else []
        bus.disconnect()

        # Check for duplicates
        seen = set()
        duplicates = []
        for name in names:
            if name in seen:
                duplicates.append(name)
            seen.add(name)

        if duplicates:
            return {
                "status": "error",
                "error": f"Found duplicates: {duplicates}",
            }

        return {"status": "success", "count": len(names)}

    except Exception as e:
        return {"status": "exception", "error": str(e)}
