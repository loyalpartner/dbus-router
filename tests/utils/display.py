"""Display allocation utilities for parallel test execution."""

import os


def is_headless_mode() -> bool:
    """Check if tests should run in headless mode (Xvfb)."""
    return os.environ.get("HSDBUS_HEADLESS", "0") == "1"


def get_host_display_for_worker() -> str:
    """Get host Xvfb display number for current worker.

    Display range: :149, :150, :151, ...
    """
    worker = os.environ.get("PYTEST_XDIST_WORKER", "")
    if worker.startswith("gw"):
        return f":{149 + int(worker[2:]) * 2}"
    return ":149"


def get_sandbox_display_for_worker() -> str:
    """Get sandbox Xvfb display number for current worker.

    Display range: :99, :100, :101, ...
    """
    worker = os.environ.get("PYTEST_XDIST_WORKER", "")
    if worker.startswith("gw"):
        return f":{99 + int(worker[2:]) * 2}"
    return ":99"


# Backwards compatibility
def get_display_for_worker() -> str:
    """Deprecated: use get_sandbox_display_for_worker instead."""
    return get_sandbox_display_for_worker()


def get_xvfb_display_for_worker() -> str:
    """Deprecated: use get_host_display_for_worker instead."""
    return get_host_display_for_worker()
