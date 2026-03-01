# HuntLoan — GO LIVE Checklist

Complete every gate IN ORDER. Do not skip or rush.
This checklist covers the full path from zero to stable controlled-live operation.

---

## PRE-FLIGHT (one time)

### System

- [ ] Old bot fully decommissioned: `pm2 list` shows nothing
- [ ] No stray node processes: `ps aux | grep node | grep -v grep` is empty
- [ ] Crontab empty: `crontab -l` shows nothing MEV-related
- [ ] Binary compiled: `ls -lh target/release/huntloan` (≥ 5MB)
- [ ] Rust edition 2024 + alloy v1 confirmed in Cargo.toml

### Config

- [ ] `.env` exists and is not git-tracked (`git status` shows nothing in src/)
- [ ] `RPC_URL` reachable: `cast block-number --rpc-url $RPC_URL`
- [ ] `WS_RPC_URL` set and valid (must start with `wss://`)
- [ ] `PRIVATE_KEY` valid: boot log shows `operator = 0x3011...`
- [ ] `HUNTLOAN_CONTRACT` set and has bytecode: `cast code $HUNTLOAN_CONTRACT --rpc-url $RPC_URL | wc -c` > 100
- [ ] `TELEGRAM_BOT_TOKEN` + `TELEGRAM_CHAT_ID` set (or both empty for no alerts)
- [ ] `DRY_RUN=true` — default state before any test

### Wallet

- [ ] Balance ≥ 0.1 ETH: `cast balance $OPERATOR_ADDR --rpc-url $RPC_URL --ether`
- [ ] Nonce stable (two readings, 30s apart):
  ```bash
  N1=$(cast nonce $OPERATOR_ADDR --rpc-url $RPC_URL); sleep 30
  N2=$(cast nonce $OPERATOR_ADDR --rpc-url $RPC_URL)
  echo "$N1 == $N2 → PASS (or FAIL if different)"
  ```

### Tests

- [ ] All 13 tests pass: `cargo test 2>&1 | tail -5`

---

## GATE 1 — DRY_RUN Validation (≥ 30 min)

**Config:** `DRY_RUN=true`  
**Command:** `make dry` or `RUST_LOG=huntloan=info cargo run --release`

Required log patterns (all must appear):

- [ ] `Config loaded` with correct addresses
- [ ] `Loading Aave V3 reserve cache` — reserves found (≥ 1)
- [ ] `Subscribing to new block headers` — WS connected
- [ ] `[discovery] Watchlist written count=N` where N > 1000
- [ ] `Scan complete candidates=44924 opportunities=N` (opportunities > 0)
- [ ] `Best opportunity selected` at least once
- [ ] `DRY_RUN — tx NOT sent` confirming no broadcast
- [ ] No `CIRCUIT_BREAKER` lines
- [ ] No `ERROR` lines from scanner or WS

**Abort if:**
- `count=0` from discovery → subgraph issue
- `opportunities=0` every block → scanner misconfiguration
- `ERROR WebSocket` → WS endpoint problem

---

## GATE 2 — SOFT_LIVE Preview (≥ 10 min)

**Config:** `DRY_RUN=false`, `SOFT_LIVE=true`  
**Command:** `make soft`

Required log patterns:

- [ ] `SOFT_LIVE — full tx preview (NOT broadcast)` appears
- [ ] Preview shows valid: `chain_id=8453`, `nonce=N`, `max_fee_wei > 0`
- [ ] `calldata_bytes=132` (requestFlashLiquidation is always 4+4×32=132 bytes)
- [ ] `borrower` address in preview matches a known Aave borrower
- [ ] Nonce does NOT advance (check `cast nonce` before and after)
- [ ] No Telegram alert fired (SOFT_LIVE is preview-only)

---

## GATE 3 — LIVE_CONTROLLED (first session, ≥ 2h)

**Config:** Tight caps  
**Command:** `make live-controlled`

Caps enforced:
```
DRY_RUN=false
SOFT_LIVE=false
MAX_GAS_COST_WEI=2000000000000000    # 0.002 ETH per tx
MAX_BRIBE_WEI=5000000000000000       # 0.005 ETH per tx
MIN_PROFIT_USD=20                    # $20 minimum
MAX_PARALLEL_SIMS=4
TARGET_COOLDOWN_SECONDS=300
```

Success criteria:

- [ ] At least 1 tx confirmed with `status=1`
- [ ] `logs/trades.csv` created with ≥ 1 row
- [ ] Telegram 🐺🔥 EXECUTED alert received on Telegram
- [ ] `consecutive_reverts` counter never reached `max_consecutive_reverts`
- [ ] Circuit breaker NOT triggered
- [ ] Wallet balance decreased by ≤ estimated gas cost (no unexpected spend)
- [ ] After session: `cast call $HUNTLOAN_CONTRACT "totalProfit()(uint256)" --rpc-url $RPC_URL` increased

---

## GATE 4 — LIVE Escalation

Only after Gate 3 runs stably for ≥ 24h with ≥ 3 confirmed txs.

**Config changes from LIVE_CONTROLLED:**
```
MAX_GAS_COST_WEI=8000000000000000    # 0.008 ETH (default)
MAX_BRIBE_WEI=50000000000000000      # 0.05 ETH (default)
MIN_PROFIT_USD=10                    # $10 minimum
MAX_DAILY_GAS_WEI=50000000000000000  # 0.05 ETH/day hard cap
MAX_DAILY_BRIBE_WEI=200000000000000000 # 0.2 ETH/day
```

- [ ] Nonce check re-run before starting
- [ ] Wallet balance ≥ 0.5 ETH
- [ ] Telegram configured and receiving
- [ ] `tmux` or `systemd` supervision confirmed
- [ ] SSH disconnect does NOT kill the process

---

## MONITORING (ongoing)

### Every 24h

- [ ] `cast balance $OPERATOR_ADDR --rpc-url $RPC_URL --ether` (> 0.05 ETH)
- [ ] `cast call $HUNTLOAN_CONTRACT "totalProfit()(uint256)" --rpc-url $RPC_URL` (increasing)
- [ ] No `CIRCUIT_BREAKER` in logs
- [ ] Telegram hourly summary (📊) received
- [ ] `logs/trades.csv` line count increasing: `wc -l logs/trades.csv`

### Win-rate health

Acceptable: win_rate ≥ 60%  
Investigate: win_rate < 40% (raise MIN_PROFIT_USD or investigate revert patterns)  
Stop: win_rate < 20% (circuit breaker or systematic issue)

```bash
# Win rate from trades.csv:
awk -F, 'NR>1 {t++; if($13==1) w++} END {printf "Win rate: %.1f%%\n", w/t*100}' logs/trades.csv
```

---

## EMERGENCY STOP

```bash
# 1. Immediate kill
pkill -f huntloan

# 2. Verify process gone
ps aux | grep huntloan | grep -v grep

# 3. Check for pending txs
cast nonce $OPERATOR_ADDR --rpc-url $RPC_URL
# If nonce is still moving, cancel with:
cast send $OPERATOR_ADDR --value 0 \
  --nonce <STUCK_NONCE> \
  --max-fee-per-gas 2000000000 \  # 2 gwei (high enough to replace)
  --private-key $PRIVATE_KEY \
  --rpc-url $RPC_URL

# 4. Telegram will fire 🚨 circuit breaker alert if bot exits cleanly
#    If killed hard (pkill), send manual notification.
```

---

## Phase 8+ Upgrades (next iteration)

After stable 7-day live operation:

- [ ] G-1: Reduce Multicall3 batches from sequential to concurrent (JoinSet for scan stage 1)
- [ ] G-4: Try alternate Uniswap V3 fee tiers on `SwapFailed` to recover missed positions
- [ ] G-7: Add per-asset analytics (which assets generate most profit)
- [ ] G-10: JSON log output for Grafana/Datadog
- [ ] Midnight daily budget reset (currently resets on bot restart)
- [ ] Private RPC integration (sendPrivateTransaction endpoint)
