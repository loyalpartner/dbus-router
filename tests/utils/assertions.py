"""D-Bus specific test assertions."""

import subprocess
from pathlib import Path


def assert_dbus_service_exists(dbus_addr: str, service_name: str):
    """Assert that a D-Bus service is registered on the bus.

    Args:
        dbus_addr: D-Bus address to connect to
        service_name: Service name to check (e.g., "org.freedesktop.Notifications")
    """
    result = subprocess.run(
        ["dbus-send", "--print-reply", f"--address={dbus_addr}",
         "--dest=org.freedesktop.DBus", "/org/freedesktop/DBus",
         "org.freedesktop.DBus.ListNames"],
        capture_output=True, text=True
    )
    assert result.returncode == 0, f"Failed to list D-Bus names: {result.stderr}"
    assert service_name in result.stdout, f"Service {service_name} not found on bus"


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
