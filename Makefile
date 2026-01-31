.PHONY: build test test-unit test-integration clean

# Build the project in release mode
build:
	cargo build --release

# Run all tests
test: test-unit test-integration

# Run unit tests (Rust)
test-unit:
	cargo test

# Run integration tests (Python)
test-integration: build
	cd tests && uv run pytest -v -n auto

# Clean build artifacts
clean:
	cargo clean
	rm -rf tests/__pycache__ tests/utils/__pycache__
	rm -rf /tmp/hsdbus_tests_*
