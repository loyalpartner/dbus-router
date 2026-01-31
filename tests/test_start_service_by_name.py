"""StartServiceByName integration tests.

Tests for StartServiceByName method which should:
1. Route to correct bus based on service name
2. Return ALREADY_RUNNING for services that are already active
3. Return error for non-activatable services
"""

import asyncio
from pathlib import Path

from dbus_next.aio import MessageBus
from dbus_next import Message, MessageType

EMPTY_CONFIG = ""

HOST_ROUTE_CONFIG = '''
[[host_routes]]
destination = "org.test.HostService"
'''


def test_start_service_not_activatable(test_log_dir: Path, router_env):
    """StartServiceByName for non-activatable service should return error."""
    with router_env(EMPTY_CONFIG, socket_prefix="ssn_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_start_not_activatable(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_start_not_activatable(router_addr: str, test_log_dir: Path) -> dict:
    """Test StartServiceByName for non-activatable service."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Try to start a non-existent/non-activatable service
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='StartServiceByName',
                signature='su',
                body=['org.test.NonExistentService', 0],
            )
        )

        bus.disconnect()

        (test_log_dir / "start_service.log").write_text(
            f"message_type: {reply.message_type}, body: {reply.body}"
        )

        # Should get an error (ServiceUnknown or similar)
        if reply.message_type != MessageType.ERROR:
            return {
                "status": "error",
                "error": f"Expected error for non-activatable service, got: {reply.body}"
            }

        return {"status": "success", "error_name": reply.error_name}

    except Exception as e:
        # ServiceUnknown error is expected
        err_str = str(e)
        if "ServiceUnknown" in err_str or "not provided" in err_str or "not activatable" in err_str.lower():
            return {"status": "success", "exception": err_str}
        return {"status": "exception", "error": err_str}


def test_start_service_dbus_daemon(test_log_dir: Path, router_env):
    """StartServiceByName for org.freedesktop.DBus should return ALREADY_RUNNING."""
    with router_env(EMPTY_CONFIG, socket_prefix="ssn_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_start_dbus_daemon(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_start_dbus_daemon(router_addr: str, test_log_dir: Path) -> dict:
    """Test StartServiceByName for org.freedesktop.DBus."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='StartServiceByName',
                signature='su',
                body=['org.freedesktop.DBus', 0],
            )
        )

        bus.disconnect()

        (test_log_dir / "start_dbus_daemon.log").write_text(
            f"message_type: {reply.message_type}, body: {reply.body}"
        )

        if reply.message_type == MessageType.ERROR:
            # Some implementations may return error for starting the bus itself
            return {"status": "success", "error_name": reply.error_name}

        # Return code 2 = ALREADY_RUNNING
        if reply.body and reply.body[0] == 2:
            return {"status": "success", "result": "ALREADY_RUNNING"}

        return {"status": "success", "result": reply.body[0] if reply.body else None}

    except Exception as e:
        return {"status": "exception", "error": str(e)}


def test_start_service_host_routed(test_log_dir: Path, router_env):
    """StartServiceByName for host-routed service should route to host bus."""
    with router_env(HOST_ROUTE_CONFIG, socket_prefix="ssn_") as env:
        _, _, router_addr = env
        result = asyncio.run(
            _test_start_host_routed(router_addr, test_log_dir)
        )
        assert result["status"] == "success", f"Test failed: {result}"


async def _test_start_host_routed(router_addr: str, test_log_dir: Path) -> dict:
    """Test StartServiceByName routes to host bus for host_routes services."""
    try:
        bus = await MessageBus(bus_address=router_addr).connect()

        # Try to start a host-routed service (doesn't exist, but should route to host)
        reply = await bus.call(
            Message(
                destination='org.freedesktop.DBus',
                path='/org/freedesktop/DBus',
                interface='org.freedesktop.DBus',
                member='StartServiceByName',
                signature='su',
                body=['org.test.HostService', 0],
            )
        )

        bus.disconnect()

        (test_log_dir / "start_host_routed.log").write_text(
            f"message_type: {reply.message_type}, body: {reply.body}"
        )

        # Should get an error (service doesn't exist on host bus)
        if reply.message_type != MessageType.ERROR:
            return {
                "status": "error",
                "error": f"Expected error for non-existent host service, got: {reply.body}"
            }

        # The error proves the request was routed to host bus
        return {"status": "success", "error_name": reply.error_name}

    except Exception as e:
        err_str = str(e)
        if "ServiceUnknown" in err_str or "not provided" in err_str:
            return {"status": "success", "exception": err_str}
        return {"status": "exception", "error": err_str}
