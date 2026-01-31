.PHONY: build test test-unit test-integration clean

# Build the project in release mode
build:
	cargo build --release

# pytest args (override as needed, e.g. PYTEST_XDIST="-n 8 --dist=loadscope")
PYTEST_ARGS ?= -v
PYTEST_XDIST ?= -n auto --dist=loadscope

# Run all tests
test: test-unit test-integration

# Run unit tests (Rust)
test-unit:
	cargo test

# Run integration tests (Python)
test-integration: build
	cd tests && uv run pytest $(PYTEST_ARGS) $(PYTEST_XDIST)

# Clean build artifacts
clean:
	cargo clean
	rm -rf tests/__pycache__ tests/utils/__pycache__
	rm -rf /tmp/hsdbus_tests_*
