.PHONY: fmt fmt-check lint test check doc pre-commit publish-dry publish clean

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

lint:
	cargo clippy --all-targets --all-features -- -D warnings
	@command -v cargo-deny >/dev/null 2>&1 && cargo deny check || echo "cargo-deny not installed, skipping (cargo install cargo-deny)"

test:
	cargo test --all-features

check:
	cargo check --all-targets --all-features

doc:
	cargo doc --no-deps --all-features --document-private-items

pre-commit: fmt-check lint test doc
	@echo "All checks passed"

# Publish in dependency order (core first, then build, then rest, then openapi)
publish-dry:
	cargo publish --dry-run -p tonic-rest-core
	cargo publish --dry-run -p tonic-rest-build
	cargo publish --dry-run -p tonic-rest
	cargo publish --dry-run -p tonic-rest-openapi

publish:
	cargo publish -p tonic-rest-core
	@echo "Waiting for crates.io index to update..."
	@sleep 30
	cargo publish -p tonic-rest-build
	@echo "Waiting for crates.io index to update..."
	@sleep 30
	cargo publish -p tonic-rest
	@echo "Waiting for crates.io index to update..."
	@sleep 30
	cargo publish -p tonic-rest-openapi

clean:
	cargo clean
