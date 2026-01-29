.PHONY: build test test-headless test-parallel clean

# Build the project in release mode
build:
	cargo build --release

# Run visual tests with Xephyr on host display
test: build
	cd tests && uv run pytest -v -m xorg

# Run headless tests with Xvfb
test-headless: build
	cd tests && HSDBUS_HEADLESS=1 uv run pytest -v -m xorg

# Run parallel headless tests
test-parallel: build
	cd tests && HSDBUS_HEADLESS=1 uv run pytest -v -m xorg -n auto --dist loadfile

# Run unit tests only (Rust)
test-unit:
	cargo test

# Clean build artifacts
clean:
	cargo clean
	rm -rf tests/__pycache__ tests/utils/__pycache__
	rm -rf /tmp/hsdbus_tests_*
