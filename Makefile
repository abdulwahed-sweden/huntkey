.PHONY: build release check test run dry

# ── Rust bot ────────────────────────────────────────────────────────────────
check:
	cargo check

build:
	cargo build

release:
	cargo build --release

test:
	cargo test -- --nocapture

run:
	RUST_LOG=huntloan=info cargo run

dry:
	DRY_RUN=true RUST_LOG=huntloan=info cargo run

# ── Solidity (Foundry) ──────────────────────────────────────────────────────
sol-build:
	forge build

sol-test:
	forge test -vvv

deploy-dry:
	forge script script/Deploy.s.sol --rpc-url $$BASE_RPC_URL --dry-run

deploy:
	forge script script/Deploy.s.sol --rpc-url $$BASE_RPC_URL --broadcast --private-key $$PRIVATE_KEY

# ── Utility ─────────────────────────────────────────────────────────────────
clean:
	cargo clean && forge clean 2>/dev/null || true
