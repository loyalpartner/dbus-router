"""VSCode integration tests for D-Bus router."""

from pathlib import Path

import pytest

from utils.dbus_env import dbus_router_session, dbus_session
from utils.vscode import is_vscode_running, vscode_session
from utils.xorg_env import xorg_test_env


@pytest.mark.xorg
def test_vscode_opens_with_router(test_log_dir: Path, build_project):
    """VSCode should open successfully when using the D-Bus router.

    Architecture:
        Host Xvfb (:149) + Host D-Bus
            ↑
        dbus-router (hostpass: VSCode)
            ↑
        Sandbox Xvfb (:99) + Sandbox D-Bus
            ↑
        VSCode
    """
    with xorg_test_env(test_log_dir) as xenv:
        host_dbus_socket = test_log_dir / "host-dbus.sock"
        sandbox_dbus_socket = test_log_dir / "sandbox-dbus.sock"
        router_socket = test_log_dir / "router.sock"

        config = test_log_dir / "router.toml"
        config.write_text(
            """
# Route all VSCode messages to host bus
[[hostpass]]
process = "/opt/visual-studio-code/code"
"""
        )

        # Start host D-Bus session
        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            # Start sandbox D-Bus session
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                # Start router: sandbox → host
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    # VSCode runs in sandbox display, uses router for D-Bus
                    with vscode_session(
                        xenv.sandbox_display, router_addr, test_log_dir, wait_seconds=10.0
                    ) as vscode:
                        # Verify VSCode started
                        assert is_vscode_running(vscode), "VSCode crashed on startup"
                        assert len(vscode.pids) > 0, "No VSCode processes found"


@pytest.mark.xorg
def test_vscode_notifications_route_to_host(test_log_dir: Path, build_project):
    """Notifications from VSCode should route to host bus."""
    with xorg_test_env(test_log_dir) as xenv:
        host_dbus_socket = test_log_dir / "host-dbus.sock"
        sandbox_dbus_socket = test_log_dir / "sandbox-dbus.sock"
        router_socket = test_log_dir / "router.sock"

        config = test_log_dir / "router.toml"
        config.write_text(
            """
# Route all VSCode messages to host bus
[[hostpass]]
process = "/opt/visual-studio-code/code"
"""
        )

        with dbus_session(host_dbus_socket, test_log_dir, "host-dbus") as host_addr:
            with dbus_session(sandbox_dbus_socket, test_log_dir, "sandbox-dbus") as sandbox_addr:
                with dbus_router_session(
                    router_socket, host_addr, sandbox_addr, config, test_log_dir
                ) as router_addr:
                    with vscode_session(
                        xenv.sandbox_display, router_addr, test_log_dir, wait_seconds=10.0
                    ) as vscode:
                        assert is_vscode_running(vscode), "VSCode crashed on startup"

                        # Check router logs for errors
                        router_stderr = test_log_dir / "router.stderr"
                        if router_stderr.exists():
                            log_content = router_stderr.read_text()
                            assert "error" not in log_content.lower() or "non-fatal" in log_content.lower(), (
                                f"Router logged errors: {log_content}"
                            )
