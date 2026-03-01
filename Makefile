.PHONY: build release check test run dry soft live-controlled sol-build sol-test deploy-dry deploy clean fmt lint nonce-check nonce-watch

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

# ── Simulation-only mode (default safe state) ─────────────────────────────────
# DRY_RUN=true — no transactions sent, full pipeline logs
# PASS: logs show "DRY_RUN — tx NOT sent" for every opportunity
dry:
	DRY_RUN=true RUST_LOG=huntloan=info cargo run

# ── Soft-live mode (tx preview without broadcast) ─────────────────────────────
# DRY_RUN=false + SOFT_LIVE=true — resolves nonce, encodes calldata, prints full
# tx preview (to, calldata, maxFee, maxPriority, gasLimit, nonce) but does NOT send.
# Use this AFTER decommissioning the old bot and BEFORE the first real broadcast.
# PASS: logs show "SOFT_LIVE — full tx preview (NOT broadcast)"
soft:
	DRY_RUN=false SOFT_LIVE=true RUST_LOG=huntloan=info cargo run

# ── Controlled live mode (very tight caps, minimum risk) ──────────────────────
# DRY_RUN=false, no SOFT_LIVE — actual broadcast enabled.
# Caps set to 0.002 ETH gas / 0.005 ETH bribe (10-25x tighter than defaults).
# Only attempt opportunities with >=  $20 net profit (2x the default floor).
# PREREQUISITE: old bot decommissioned, nonce stable, wallet funded.
live-controlled:
	DRY_RUN=false \
	SOFT_LIVE=false \
	MAX_GAS_COST_WEI=2000000000000000 \
	MAX_BRIBE_WEI=5000000000000000 \
	MIN_PROFIT_USD=20 \
	RUST_LOG=huntloan=info \
	cargo run --release

# ── Full live mode (production caps from .env) ────────────────────────────────
# LIVE mode — requires WS_RPC_URL + HUNTLOAN_CONTRACT in .env
# Use only after 24h of live-controlled without incidents.
run:
	RUST_LOG=huntloan=info cargo run --release

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

# ── Pre-launch safety checks ──────────────────────────────────────────────────
# Nonce stability check — MANDATORY before DRY_RUN=false.
# Reads two nonces 30s apart. PASS = same value. FAIL = external tx detected.
# Loads RPC_URL from .env automatically.
#
# Usage:
#   make nonce-check          # default 30s gap
#   make nonce-check WAIT=60  # 60s gap
nonce-check:
	@bash scripts/nonce_check.sh $(WAIT)

# Continuous nonce watcher — runs until Ctrl+C.
# Logs every poll result; warns loudly if nonce changes between polls.
#
# Usage:
#   make nonce-watch           # poll every 10s (default)
#   make nonce-watch POLL=5    # poll every 5s
nonce-watch:
	@bash scripts/nonce_watch.sh $(POLL)

# ── Utility ──────────────────────────────────────────────────────────────────
clean:
	cargo clean && forge clean 2>/dev/null || true
