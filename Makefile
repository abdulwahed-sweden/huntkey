.PHONY: build release check test run dry sol-build sol-test deploy-dry deploy clean fmt lint

# ── Rust bot ─────────────────────────────────────────────────────────────────
check:
	cargo check

fmt:
	cargo fmt

lint:
	cargo clippy -- -D warnings

build:
	cargo build

release:
	cargo build --release

test:
	cargo test -- --nocapture

# LIVE mode — requires WS_RPC_URL + HUNTLOAN_CONTRACT in .env
run:
	RUST_LOG=huntloan=info cargo run

# Simulation-only mode — no transactions broadcast
dry:
	DRY_RUN=true RUST_LOG=huntloan=info cargo run

# ── Solidity / Foundry ────────────────────────────────────────────────────────
sol-build:
	forge build

sol-test:
	forge test -vvv

# Dry-run deploy: verify calldata without broadcasting
deploy-dry:
	forge script script/Deploy.s.sol \
	  --rpc-url $$RPC_URL \
	  --chain-id 8453 \
	  --dry-run

# Production deploy: broadcast HuntLoanFlashReceiver to Base mainnet
deploy:
	forge script script/Deploy.s.sol \
	  --rpc-url $$RPC_URL \
	  --chain-id 8453 \
	  --broadcast \
	  --private-key $$PRIVATE_KEY

# ── Utility ──────────────────────────────────────────────────────────────────
clean:
	cargo clean && forge clean 2>/dev/null || true
