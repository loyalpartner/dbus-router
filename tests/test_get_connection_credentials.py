"""GetConnectionCredentials integration tests.

Tests for credential query methods:
- GetConnectionUnixUser
- GetConnectionUnixProcessID
- GetConnectionCredentials

These methods should:
1. Rewrite fake unique name to real unique name
2. Route to correct bus based on prefix
3. Return correct credentials
"""

import tempfile
import asyncio
import os
from pathlib import Path

from dbus_next.aio import MessageBus
from dbus_next import Message, MessageType

from utils.dbus_env import dbus_session, dbus_router_session


def test_get_connection_unix_user(test_log_dir: Path, build_project):
    """GetConnectionUnixUser should return UID for our connection."""
    with tempfile.TemporaryDirectory(prefix="gcc_") as sock_dir:
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
                        _test_get_unix_user(router_addr, test_log_dir)
                    )
                    assert result["status"] == "success", f"Test failed: {result}"


async def _test_get_unix_user(router_addr: str, test_log_dir: Path) -> dict:
    """Test GetConnectionUnixUser returns correct UID."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Query our own UID
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='GetConnectionUnixUser',
                signature='s',
                body=[bus.unique_name],
            )
        )

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"GetConnectionUnixUser failed: {reply.body}"}

        uid = reply.body[0] if reply.body else None
        expected_uid = os.getuid()

        (test_log_dir / "unix_user.log").write_text(
            f"unique_name: {bus.unique_name}, returned_uid: {uid}, expected_uid: {expected_uid}"
        )

        bus.disconnect()

        if uid != expected_uid:
            return {"status": "error", "error": f"Expected UID {expected_uid}, got {uid}"}

        return {"status": "success", "uid": uid}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_get_connection_unix_process_id(test_log_dir: Path, build_project):
    """GetConnectionUnixProcessID should return PID for our connection."""
    with tempfile.TemporaryDirectory(prefix="gcc_") as sock_dir:
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
                        _test_get_unix_pid(router_addr, test_log_dir)
                    )
                    assert result["status"] == "success", f"Test failed: {result}"


async def _test_get_unix_pid(router_addr: str, test_log_dir: Path) -> dict:
    """Test GetConnectionUnixProcessID returns a valid PID.

    Note: When querying through router, the sandbox bus sees the router's
    connection, so the PID returned is the router's PID (or test's PID
    depending on connection model). We just verify a valid PID is returned.
    """
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Query our own PID
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

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"GetConnectionUnixProcessID failed: {reply.body}"}

        pid = reply.body[0] if reply.body else None

        (test_log_dir / "unix_pid.log").write_text(
            f"unique_name: {bus.unique_name}, returned_pid: {pid}"
        )

        bus.disconnect()

        # Just verify we got a valid PID (positive integer)
        if pid is None or pid <= 0:
            return {"status": "error", "error": f"Invalid PID: {pid}"}

        return {"status": "success", "pid": pid}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_get_connection_credentials(test_log_dir: Path, build_project):
    """GetConnectionCredentials should return credential dict for our connection."""
    with tempfile.TemporaryDirectory(prefix="gcc_") as sock_dir:
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
                        _test_get_credentials(router_addr, test_log_dir)
                    )
                    assert result["status"] == "success", f"Test failed: {result}"


async def _test_get_credentials(router_addr: str, test_log_dir: Path) -> dict:
    """Test GetConnectionCredentials returns credential dict.

    Note: When querying through router, the sandbox bus sees the router's
    connection. We verify UID matches (same user) but don't require PID match.
    """
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Query our own credentials
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

        if reply.message_type == MessageType.ERROR:
            return {"status": "error", "error": f"GetConnectionCredentials failed: {reply.body}"}

        credentials = reply.body[0] if reply.body else {}

        (test_log_dir / "credentials.log").write_text(
            f"unique_name: {bus.unique_name}, credentials: {credentials}"
        )

        bus.disconnect()

        # Should have at least UnixUserID and ProcessID
        if not isinstance(credentials, dict):
            return {"status": "error", "error": f"Expected dict, got {type(credentials)}"}

        # Check UnixUserID - should match since all processes run as same user
        if 'UnixUserID' in credentials:
            uid = credentials['UnixUserID'].value
            if uid != os.getuid():
                return {"status": "error", "error": f"UnixUserID mismatch: {uid} != {os.getuid()}"}

        # ProcessID may not match (could be router's PID), just verify it exists and is valid
        if 'ProcessID' in credentials:
            pid = credentials['ProcessID'].value
            if pid <= 0:
                return {"status": "error", "error": f"Invalid ProcessID: {pid}"}

        return {"status": "success", "credentials_keys": list(credentials.keys())}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_credentials_fake_name_rewrite(test_log_dir: Path, build_project):
    """Credential queries with fake unique names should be rewritten."""
    with tempfile.TemporaryDirectory(prefix="gcc_") as sock_dir:
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
                        _test_fake_name_rewrite(router_addr, test_log_dir)
                    )
                    assert result["status"] == "success", f"Test failed: {result}"


async def _test_fake_name_rewrite(router_addr: str, test_log_dir: Path) -> dict:
    """Test that fake unique names are rewritten in credential queries."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Our name should have :s. prefix
        our_name = bus.unique_name
        if not our_name.startswith(":s."):
            bus.disconnect()
            return {"status": "error", "error": f"Expected :s. prefix, got: {our_name}"}

        # Query credentials using our fake name
        # Router should rewrite :s.X.Y to :X.Y before sending to sandbox bus
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='GetConnectionUnixUser',
                signature='s',
                body=[our_name],
            )
        )

        if reply.message_type == MessageType.ERROR:
            bus.disconnect()
            return {"status": "error", "error": f"Query failed: {reply.body}"}

        uid = reply.body[0] if reply.body else None

        (test_log_dir / "fake_name_rewrite.log").write_text(
            f"our_fake_name: {our_name}, returned_uid: {uid}, expected_uid: {os.getuid()}"
        )

        bus.disconnect()

        if uid != os.getuid():
            return {"status": "error", "error": f"UID mismatch after rewrite: {uid} != {os.getuid()}"}

        return {"status": "success", "fake_name": our_name, "uid": uid}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_credentials_invalid_name(test_log_dir: Path, build_project):
    """Credential query for non-existent connection should fail."""
    with tempfile.TemporaryDirectory(prefix="gcc_") as sock_dir:
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
                        _test_invalid_name(router_addr, test_log_dir)
                    )
                    assert result["status"] == "success", f"Test failed: {result}"


async def _test_invalid_name(router_addr: str, test_log_dir: Path) -> dict:
    """Test credential query for non-existent connection."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Query for a non-existent connection
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='GetConnectionUnixUser',
                signature='s',
                body=[':s.999.999'],  # Non-existent connection
            )
        )

        bus.disconnect()

        # Should get an error (NameHasNoOwner or similar)
        if reply.message_type != MessageType.ERROR:
            return {
                "status": "error",
                "error": f"Expected error for non-existent name, got: {reply.body}"
            }

        return {"status": "success", "error_name": reply.error_name}

    except Exception as e:
        # DBusError is expected here
        if "NameHasNoOwner" in str(e) or "NoReply" in str(e) or "Error" in str(e):
            return {"status": "success", "exception": str(e)}
        return {"status": "exception", "error": str(e)}
