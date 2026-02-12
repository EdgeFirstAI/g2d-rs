# g2d-rs Makefile

.PHONY: help format lint check test bench clean

help:
	@echo "Available targets:"
	@echo "  make format    - Format code with rustfmt"
	@echo "  make lint      - Run clippy"
	@echo "  make check     - Run cargo check"
	@echo "  make test      - Run tests"
	@echo "  make bench     - Run benchmarks"
	@echo "  make clean     - Clean build artifacts"

format:
	@echo "Formatting Rust code..."
	@cargo fmt --all
	@echo "✓ Formatting complete"

lint:
	@echo "Running clippy..."
	@cargo clippy --workspace --all-targets -- -D warnings
	@echo "✓ Clippy passed"

check:
	@echo "Running cargo check..."
	@cargo check --workspace
	@echo "✓ Check passed"

test:
	@echo "Running tests..."
	@cargo test --workspace
	@echo "✓ Tests passed"

bench:
	@echo "Running benchmarks..."
	@cargo bench --workspace
	@echo "✓ Benchmarks complete"

clean:
	@echo "Cleaning..."
	@cargo clean
	@echo "✓ Clean complete"
