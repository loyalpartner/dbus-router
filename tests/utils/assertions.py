"""D-Bus specific test assertions."""

import asyncio
import subprocess
from pathlib import Path

from dbus_next.aio import MessageBus


async def _list_names(dbus_addr: str) -> list[str]:
    """List all names on the bus using dbus-next."""
    bus = await MessageBus(bus_address=dbus_addr).connect()
    introspection = await bus.introspect('org.freedesktop.DBus', '/org/freedesktop/DBus')
    proxy = bus.get_proxy_object('org.freedesktop.DBus', '/org/freedesktop/DBus', introspection)
    interface = proxy.get_interface('org.freedesktop.DBus')
    names = await interface.call_list_names()
    bus.disconnect()
    return names


def assert_dbus_service_exists(dbus_addr: str, service_name: str):
    """Assert that a D-Bus service is registered on the bus.

    Args:
        dbus_addr: D-Bus address to connect to
        service_name: Service name to check (e.g., "org.freedesktop.Notifications")
    """
    names = asyncio.run(_list_names(dbus_addr))
    assert service_name in names, f"Service {service_name} not found on bus. Available: {names}"


def assert_dbus_service_not_exists(dbus_addr: str, service_name: str):
    """Assert that a D-Bus service is NOT registered on the bus.

    Args:
        dbus_addr: D-Bus address to connect to
        service_name: Service name that should not exist
    """
    names = asyncio.run(_list_names(dbus_addr))
    assert service_name not in names, f"Service {service_name} should not be on bus, but was found"


def assert_log_contains(log_file: Path, pattern: str):
    """Assert that a log file contains a specific pattern.

    Args:
        log_file: Path to the log file
        pattern: String pattern to search for
    """
    assert log_file.exists(), f"Log file does not exist: {log_file}"
    content = log_file.read_text()
    assert pattern in content, f"Pattern '{pattern}' not found in {log_file}"


def assert_no_errors_in_log(log_file: Path, ignore_patterns: list[str] | None = None):
    """Assert that a log file contains no error messages.

    Args:
        log_file: Path to the log file
        ignore_patterns: List of error patterns to ignore
    """
    if not log_file.exists():
        return

    ignore_patterns = ignore_patterns or []
    content = log_file.read_text()

    for line in content.splitlines():
        line_lower = line.lower()
        if "error" in line_lower or "fatal" in line_lower:
            should_ignore = any(p in line for p in ignore_patterns)
            if not should_ignore:
                raise AssertionError(f"Error found in log: {line}")
