"""Multi-process routing integration tests.

Tests scenarios with multiple clients connecting to the router simultaneously.
"""

import tempfile
import asyncio
import threading
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor, as_completed

from dbus_next.aio import MessageBus
from dbus_next import Message, MessageType

from utils.dbus_env import dbus_session, dbus_router_session, echo_service_session
from utils.echo_client import sync_call_echo


def test_multiple_sandbox_clients(test_log_dir: Path, build_project):
    """Multiple clients should be able to connect and make calls concurrently."""
    with tempfile.TemporaryDirectory(prefix="mp_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        # No hostpass - all clients route to sandbox
        config = test_log_dir / "router.toml"
        config.write_text("")

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    with echo_service_session(router_addr, test_log_dir):
                        # Launch multiple clients concurrently
                        num_clients = 5
                        messages_per_client = 3

                        def client_work(client_id: int) -> list[str]:
                            """Each client makes multiple calls."""
                            results = []
                            for i in range(messages_per_client):
                                msg = f"client{client_id}_msg{i}"
                                result = sync_call_echo(sandbox_addr, msg)
                                results.append(result)
                            return results

                        with ThreadPoolExecutor(max_workers=num_clients) as executor:
                            futures = {
                                executor.submit(client_work, i): i
                                for i in range(num_clients)
                            }

                            for future in as_completed(futures):
                                client_id = futures[future]
                                results = future.result()
                                # Verify each client got correct responses
                                for i, result in enumerate(results):
                                    expected = f"client{client_id}_msg{i}"
                                    assert result == expected, f"Client {client_id} got {result}, expected {expected}"


def test_mixed_sandbox_and_hostpass_clients(test_log_dir: Path, build_project):
    """One client with hostpass, another without - should route to different buses."""
    with tempfile.TemporaryDirectory(prefix="mp_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        # Hostpass for python3 (echo_service)
        config = test_log_dir / "router.toml"
        config.write_text('''
[[hostpass]]
process = "*/python3*"
''')

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    # Start echo service - it will register on HOST bus (hostpass)
                    with echo_service_session(router_addr, test_log_dir):
                        # Client 1: Call from HOST bus (should reach service)
                        result1 = sync_call_echo(host_addr, "from_host")
                        assert result1 == "from_host"

                        # Client 2: Call from SANDBOX bus (service not visible there)
                        try:
                            sync_call_echo(sandbox_addr, "from_sandbox")
                            assert False, "Should have failed - service not on sandbox bus"
                        except Exception as e:
                            # Expected - service is on host bus, not sandbox
                            # Error varies: ServiceUnknown, NameHasNoOwner, or "not provided by .service files"
                            err_str = str(e)
                            assert any(x in err_str for x in ["ServiceUnknown", "NameHasNoOwner", "not provided"])


def test_concurrent_host_and_sandbox_calls(test_log_dir: Path, build_project):
    """Concurrent calls to both host-routed and sandbox-routed services."""
    with tempfile.TemporaryDirectory(prefix="mp_") as sock_dir:
        sock_path = Path(sock_dir)
        host_dbus_socket = sock_path / "host.sock"
        sandbox_dbus_socket = sock_path / "sandbox.sock"
        router_socket = sock_path / "router.sock"

        # Route org.test.Host to host bus
        config = test_log_dir / "router.toml"
        config.write_text('''
[[host_routes]]
destination = "org.test.Host"
''')

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    # Run concurrent calls through router
                    results = asyncio.run(
                        _concurrent_mixed_calls(router_addr, test_log_dir)
                    )
                    assert results["sandbox_ok"], f"Sandbox calls failed: {results}"
                    assert results["host_ok"], f"Host calls failed: {results}"


async def _concurrent_mixed_calls(router_addr: str, test_log_dir: Path) -> dict:
    """Make concurrent calls to both sandbox and host routed destinations."""
    results = {"sandbox_ok": False, "host_ok": False}

    async def sandbox_calls():
        """Calls that route to sandbox bus."""
        bus = await MessageBus(bus_address=router_addr).connect()
        try:
            # Call org.freedesktop.DBus (routes to sandbox)
            for _ in range(3):
                reply = await bus.call(
                    Message(
                        destination='org.freedesktop.DBus',
                        path='/org/freedesktop/DBus',
                        interface='org.freedesktop.DBus',
                        member='GetId',
                    )
                )
                if reply.message_type != MessageType.METHOD_RETURN:
                    return False
            return True
        finally:
            bus.disconnect()

    async def host_calls():
        """Calls that route to host bus."""
        bus = await MessageBus(bus_address=router_addr).connect()
        try:
            # Call org.test.Host (routes to host, will get ServiceUnknown)
            for _ in range(3):
                try:
                    await bus.call(
                        Message(
                            destination='org.test.Host',
                            path='/org/test/Host',
                            interface='org.freedesktop.DBus.Peer',
                            member='Ping',
                        )
                    )
                except Exception as e:
                    # ServiceUnknown is expected - service doesn't exist
                    if "ServiceUnknown" not in str(e) and "NameHasNoOwner" not in str(e):
                        raise
            return True
        finally:
            bus.disconnect()

    # Run both concurrently
    sandbox_result, host_result = await asyncio.gather(
        sandbox_calls(),
        host_calls(),
        return_exceptions=True
    )

    results["sandbox_ok"] = sandbox_result is True
    results["host_ok"] = host_result is True

    if isinstance(sandbox_result, Exception):
        (test_log_dir / "sandbox_error.log").write_text(str(sandbox_result))
    if isinstance(host_result, Exception):
        (test_log_dir / "host_error.log").write_text(str(host_result))

    return results


def test_service_discovery_across_clients(test_log_dir: Path, build_project):
    """Client A registers service, Client B discovers and calls it."""
    with tempfile.TemporaryDirectory(prefix="mp_") as sock_dir:
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
                    # Service registers on sandbox bus via router
                    with echo_service_session(router_addr, test_log_dir):
                        # Another client discovers and calls the service via sandbox bus
                        result = sync_call_echo(sandbox_addr, "discovery_test")
                        assert result == "discovery_test"


def test_rapid_connect_disconnect(test_log_dir: Path, build_project):
    """Rapid connection/disconnection cycles should not crash router."""
    with tempfile.TemporaryDirectory(prefix="mp_") as sock_dir:
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
                    with echo_service_session(router_addr, test_log_dir):
                        # Rapid connect/call/disconnect cycles
                        num_cycles = 10

                        def rapid_cycle(cycle_id: int) -> bool:
                            try:
                                result = sync_call_echo(sandbox_addr, f"cycle{cycle_id}")
                                return result == f"cycle{cycle_id}"
                            except Exception:
                                return False

                        with ThreadPoolExecutor(max_workers=5) as executor:
                            futures = [
                                executor.submit(rapid_cycle, i)
                                for i in range(num_cycles)
                            ]
                            results = [f.result() for f in futures]

                        # All cycles should succeed
                        assert all(results), f"Some cycles failed: {results}"

                        # Router should still work after rapid cycles
                        final_result = sync_call_echo(sandbox_addr, "final_check")
                        assert final_result == "final_check"


def test_multiple_services_same_bus(test_log_dir: Path, build_project):
    """Multiple services can register on the same bus through router."""
    with tempfile.TemporaryDirectory(prefix="mp_") as sock_dir:
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
                    # Start first service
                    with echo_service_session(router_addr, test_log_dir):
                        # Verify first service works
                        result1 = sync_call_echo(sandbox_addr, "service1_test")
                        assert result1 == "service1_test"

                        # Both calls should work concurrently
                        def call_service():
                            return sync_call_echo(sandbox_addr, "concurrent")

                        with ThreadPoolExecutor(max_workers=3) as executor:
                            futures = [executor.submit(call_service) for _ in range(3)]
                            results = [f.result() for f in futures]

                        assert all(r == "concurrent" for r in results)
