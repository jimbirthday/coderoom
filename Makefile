.PHONY: setup fmt lint test run build

setup:
	@echo "No extra setup yet. Try: cargo build"

fmt:
	cargo fmt

lint:
	cargo clippy --all-targets --all-features -- -D warnings

test:
	cargo test

run:
	cargo run -- --help

build:
	cargo build --release
