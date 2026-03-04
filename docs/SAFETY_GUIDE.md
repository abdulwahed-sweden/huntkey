# HuntLoan — Safety Guide

This document covers safe operation of the HuntLoan execution engine,
including the mandatory pre-launch sequence and monitoring procedures.

---

## Nonce Stability Check Before Live Execution

### Why it matters

HuntLoan and the old Bitcoin-Sentinel bot share the same private key and operator wallet.
If the old bot is still running while HuntLoan starts broadcasting, both systems will read
the on-chain nonce independently and attempt to use the same value. The result:

- One transaction succeeds
- The other gets rejected: `nonce too low`
- Gas is wasted on the rejected tx
- The nonce cache inside HuntLoan's executor gets corrupted, requiring a restart

A nonce stability test gives you a 30-second window to confirm that **no external process
is sending transactions from the operator wallet** before you enable live execution.

### When to run it

Run `make nonce-check` at these points:
1. After completing `docs/DECOMMISSION_OLD_SYSTEM.md` (mandatory gate)
2. Before every `DRY_RUN=false` session if the system has been idle > 1 hour
3. Whenever you see unexpected `nonce too low` errors in HuntLoan logs

### How to run it

```bash
# Standard check (30-second gap between readings):
make nonce-check

# Longer gap if you want more confidence:
make nonce-check WAIT=60

# Continuous watcher (runs until Ctrl+C, warns on any nonce change):
make nonce-watch

# Or poll every 5 seconds:
make nonce-watch POLL=5
```

These commands load `RPC_URL` automatically from your `.env` file.
No flags needed. No localhost assumed.

### What the correct output looks like

```
╔══════════════════════════════════════════════════════╗
║  HuntLoan — Nonce Stability Check                    ║
╚══════════════════════════════════════════════════════╝

  Wallet  : 0x3011BfD673a9D09f9761203A7fFCca757Af22587
  RPC     : https://base-mainnet.g.alchemy.com/v2/...
  Wait    : 30s between readings

[18:34:11] Check 1 — nonce = 6
  Sleeping 30s..............................
[18:34:41] Check 2 — nonce = 6

┌─────────────────────────────────────────────────────┐
│  PASS — nonce stable: 6 → 6                         │
│  No external transactions detected.                  │
│  Safe to proceed to the next pre-launch gate.        │
└─────────────────────────────────────────────────────┘
```

### How to interpret results

**PASS (nonce identical both times):**
The operator wallet sent zero transactions during the observation window.
No external process is active. Safe to continue to the next gate.

**FAIL (nonce changed):**
```
FAIL — nonce changed: 6 → 8 (+2 tx)
An external process sent 2 transaction(s).
DO NOT set DRY_RUN=false until this is resolved.
```

Run these debugging commands to find the source:

```bash
# 1. Local machine — check for any Node.js processes
ps aux | grep 'node\|monitor_base' | grep -v grep

# 2. VPS — check PM2 (ssh root@159.89.21.106)
pm2 list
pm2 status mev-bot

# 3. Check scheduled tasks
crontab -l
cat /etc/cron.d/* 2>/dev/null | grep -i mev

# 4. Watch in real time (run nonce_watch in one terminal while investigating)
make nonce-watch POLL=5

# 5. Find all established outbound connections
lsof -i TCP | grep ESTABLISHED | grep -v grep
```

**Do not proceed to live execution until `make nonce-check` shows PASS.**

### Common error: missing `--rpc-url`

If you see this error when running `cast nonce` manually:

```
Warning: Found unknown `rpc_endpoints` config for profile `default` defined in foundry.toml.
Error: Failed to get resolver from the ENS registry: error sending request for url (http://localhost:8545/)
```

**Root cause:** `cast nonce` was called without `--rpc-url`. Foundry defaults to
`http://localhost:8545` when no RPC endpoint is given. With no local node running,
the connection fails and surfaces as an ENS resolver error (a misleading wrapper
around the underlying connection failure).

**Fix — always supply `--rpc-url` explicitly:**

```bash
# Step 1
cast nonce 0x3011BfD673a9D09f9761203A7fFCca757Af22587 --rpc-url $RPC_URL

# Step 2
sleep 30

# Step 3
cast nonce 0x3011BfD673a9D09f9761203A7fFCca757Af22587 --rpc-url $RPC_URL
```

Or just use `make nonce-check` which handles all of this automatically.

---

## STOP — Read Before Going Live

Before switching `DRY_RUN=false`, complete every step in order:

### Step 0 — Decommission the old system (MANDATORY)

The old Bitcoin-Sentinel MEV bot shares your private key. If it runs concurrently with
HuntLoan in live mode, both systems will try to send transactions from the same wallet,
causing nonce conflicts and wasted gas.

See: `docs/DECOMMISSION_OLD_SYSTEM.md`

Do not proceed until:
```bash
# On VPS (ssh root@159.89.21.106):
pm2 list     # must show no mev-bot process
pm2 status   # must show no mev-bot process
```

### Step 1 — Verify wallet balance

```bash
cast balance 0x3011BfD673a9D09f9761203A7fFCca757Af22587 --rpc-url $RPC_URL --ether
```

Minimum safe: **0.1 ETH** (covers ~12 liquidation attempts at 800K gas × 0.008 ETH cap).

The hard floor in code is `MIN_WALLET_ETH = 0.005 ETH`. The engine does NOT check
balance before executing — this is a manual check.

### Step 2 — Run 24h in DRY_RUN=true

```bash
DRY_RUN=true cargo run --release
```

Review logs for:
- `Scan complete candidates=N opportunities=M` — confirms scanner is working
- `eth_call simulation reverted` — expected for positions already liquidated
- `DRY_RUN — tx NOT sent` — confirms the guard is active
- No `ERROR` lines from WS subscription or Multicall3

### Step 3 — Switch to live mode

Edit `.env`:
```
DRY_RUN=false
```

Then restart:
```bash
cargo run --release
```

Watch the first 10 minutes closely. Expected first-execution log:
```
INFO Broadcasting liquidation tx attempt=1 borrower=0x... max_fee_gwei=...
INFO Tx submitted — waiting for receipt tx_hash=0x...
INFO Tx confirmed tx_hash=0x... block=... gas_used=...
INFO Liquidation complete tx_hash=0x... total_ms=...
```

---

## Environment Variables Reference

| Variable | Required | Default | Description |
|---|---|---|---|
| `RPC_URL` | Yes | — | Base mainnet HTTPS RPC endpoint |
| `WS_RPC_URL` | Yes | — | Base mainnet WebSocket RPC endpoint |
| `PRIVATE_KEY` | Yes | — | Operator wallet private key (hex, with 0x prefix) |
| `HUNTLOAN_CONTRACT` | Yes | — | HuntLoanFlashReceiver contract address |
| `DRY_RUN` | No | `true` | Set to `false` to enable live tx broadcasting |
| `MIN_PROFIT_USD` | No | `10` | Minimum net profit to attempt execution |
| `WATCHLIST_PATH` | No | `watchlist.json` | Path to borrower watchlist JSON file |
| `AAVE_POOL` | No | hardcoded | Override Aave V3 Pool address |
| `AAVE_ADDRESSES_PROVIDER` | No | hardcoded | Override PoolAddressesProvider |
| `MAX_GAS_COST_WEI` | No | `8000000000000000` | Hard cap on gas cost per tx (0.008 ETH) |
| `MAX_BRIBE_WEI` | No | `500000000000000000` | Operational cap on sequencer bribe (0.5 ETH) |
| `PRIVATE_RPC_URL` | No | same as RPC_URL | Private RPC for MEV-protected tx submission |
| `TELEGRAM_BOT_TOKEN` | No | — | Telegram bot token for alerts |
| `TELEGRAM_CHAT_ID` | No | — | Telegram chat ID to send alerts |
| `RUST_LOG` | No | `huntloan=info` | Logging level |
| `ALERT_RATE_LIMIT_SECONDS` | No | `60` | Min seconds between FAILED/CIRCUIT alerts |
| `SUMMARY_INTERVAL_SECONDS` | No | `3600` | Seconds between hourly summary alerts |
| `STRONG_HF_THRESHOLD` | No | `1.0` | Skip if HF > this (1.0=disabled, 0.90=strict) |
| `MIN_MARGIN_BPS` | No | `50` | Min profit margin (profit/debt×10000 bps) |
| `MAX_DAILY_GAS_WEI` | No | disabled | Daily gas spend cap in wei |
| `MAX_DAILY_BRIBE_WEI` | No | disabled | Daily bribe spend cap in wei |
| `MAX_PARALLEL_SIMS` | No | `4` | Concurrent on-chain simulation calls |
| `TARGET_COOLDOWN_SECONDS` | No | `300` | Blacklist duration for failed targets |

---

## Financial Safety Caps

### Per-transaction caps (enforced by executor + gas.rs)

| Parameter | Cap | Env override |
|---|---|---|
| Gas cost per tx | 0.008 ETH | `MAX_GAS_COST_WEI` |
| Sequencer bribe | 0.5 ETH | `MAX_BRIBE_WEI` |
| Minimum net profit | $10 USD | `MIN_PROFIT_USD` |
| Minimum wallet ETH | 0.005 ETH | `MIN_WALLET_ETH` (constants) |

### Session caps — Phase 2 (enforced by engine.rs)

| Parameter | Default | Env override |
|---|---|---|
| Daily gas budget | disabled | `MAX_DAILY_GAS_WEI` |
| Daily bribe budget | disabled | `MAX_DAILY_BRIBE_WEI` |
| Strong HF threshold | 1.0 (off) | `STRONG_HF_THRESHOLD` |
| Min profit margin | 50 bps | `MIN_MARGIN_BPS` |
| Target cooldown | 300 s | `TARGET_COOLDOWN_SECONDS` |

**Recommended production caps:**
```bash
MAX_DAILY_GAS_WEI=50000000000000000    # 0.05 ETH/day gas cap
MAX_DAILY_BRIBE_WEI=200000000000000000 # 0.2 ETH/day bribe cap
STRONG_HF_THRESHOLD=0.95               # skip barely-underwater positions
MIN_MARGIN_BPS=100                     # require 1% margin minimum
```

To tighten per-tx caps:
```bash
MAX_GAS_COST_WEI=4000000000000000   # 0.004 ETH (tighter)
MAX_BRIBE_WEI=100000000000000000    # 0.1 ETH (tighter)
```

---

## HF Tier Thresholds

| Threshold | Value | Meaning |
|---|---|---|
| `HF_COLD` | 1.50 | Discovery tracking starts |
| `HF_WARM` | 1.15 | Warm-zone scan + VelocityEngine tracking |
| `HF_HOT` | 1.07 | High-priority monitoring |
| `HF_CRITICAL` | 1.04 | Imminent liquidation |
| `HF < 1.0` | Liquidatable | Scan returns as `Opportunity` |

---

## Gas Tier Behaviour

| HF Range | ETA | Tier selected | Notes |
|---|---|---|---|
| < 1.002 | any | KILL | Maximum aggression |
| < 1.010 | < 30min | STRIKE | Competitive |
| any | < 5min | KILL | Time pressure overrides HF |
| all others | > 30min | PROBE | Conservative, probe mempool |

For high-conviction (>= $15K estimated profit): **STRIKE + KILL fired in parallel**.

---

## Process Supervision

The engine exits if the WebSocket stream terminates (RPC node restart, network blip).
Run under a supervisor for continuous operation:

**Using systemd:**
```ini
[Unit]
Description=HuntLoan MEV Engine
After=network.target

[Service]
Type=simple
WorkingDirectory=/path/to/huntloan
EnvironmentFile=/path/to/huntloan/.env
ExecStart=/path/to/huntloan/target/release/huntloan
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

**Using PM2 (Node.js hosts):**
```bash
pm2 start target/release/huntloan --name huntloan --no-autorestart
pm2 save
```

---

## Monitoring Checklist (Daily)

- [ ] Operator wallet ETH balance > 0.05 ETH
- [ ] No `ERROR` lines in logs (WS disconnect, RPC errors)
- [ ] Telegram alerts receiving at expected frequency
- [ ] `totalProfit` growing on contract (use BaseScan or cast call)
- [ ] Nonce not stuck (cast nonce on operator address)

**Check contract profit:**
```bash
cast call 0x60d0C491dF2d35E4C95D98dF37897f908b04b46f \
  "totalProfit()(uint256)" \
  --rpc-url $RPC_URL
```

---

## Emergency Stop

If the engine is broadcasting unexpected transactions:

```bash
# 1. Kill the process immediately
pkill -f huntloan

# 2. Verify no pending txs (nonce should stabilise)
watch -n 2 "cast nonce 0x3011BfD673a9D09f9761203A7fFCca757Af22587 --rpc-url $RPC_URL"

# 3. To cancel any stuck pending tx (replace with 0-value self-tx):
cast send 0x3011BfD673a9D09f9761203A7fFCca757Af22587 \
  --value 0 \
  --nonce <STUCK_NONCE> \
  --max-fee-per-gas <HIGH_GWEI> \
  --private-key $PRIVATE_KEY \
  --rpc-url $RPC_URL
```

---

## Key Contacts and Links

| Resource | Link |
|---|---|
| HuntLoanFlashReceiver on BaseScan | `https://basescan.org/address/0x60d0C491dF2d35E4C95D98dF37897f908b04b46f` |
| Operator wallet on BaseScan | `https://basescan.org/address/0x3011BfD673a9D09f9761203A7fFCca757Af22587` |
| Aave V3 Base Pool | `https://basescan.org/address/0xA238Dd80C259a72e81d7e4664a9801593F98d1c5` |
| Architecture doc | `docs/ARCHITECTURE.md` |
| Production readiness report | `docs/PRODUCTION_READY.md` |
| Decommission guide | `docs/DECOMMISSION_OLD_SYSTEM.md` |
