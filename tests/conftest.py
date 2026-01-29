"""Global pytest fixtures for hsdbus integration tests."""

import subprocess
from datetime import datetime
from pathlib import Path

import pytest


@pytest.fixture(scope="session")
def build_project():
    """Build the Rust project in release mode."""
    result = subprocess.run(
        ["cargo", "build", "--release"],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, f"Build failed: {result.stderr}"


@pytest.fixture(scope="session")
def router_binary(build_project) -> Path:
    """Path to the built router binary."""
    return Path("target/release/dbus-router")


@pytest.fixture(scope="session")
def test_run_dir() -> Path:
    """Create timestamped log directory for this test run."""
    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    path = Path(f"/tmp/hsdbus_tests_{timestamp}")
    path.mkdir(parents=True, exist_ok=True)
    return path


@pytest.fixture(scope="module")
def module_log_dir(test_run_dir, request) -> Path:
    """Per-module log directory."""
    module_name = request.module.__name__.split(".")[-1]
    path = test_run_dir / module_name
    path.mkdir(parents=True, exist_ok=True)
    return path


@pytest.fixture
def test_log_dir(module_log_dir, request) -> Path:
    """Per-test log directory."""
    test_name = request.node.name
    path = module_log_dir / test_name
    path.mkdir(parents=True, exist_ok=True)
    return path


def pytest_runtest_logreport(report):
    """Print log directory on test failure."""
    if report.failed:
        print(f"\nLogs available at: /tmp/hsdbus_tests_*/")
