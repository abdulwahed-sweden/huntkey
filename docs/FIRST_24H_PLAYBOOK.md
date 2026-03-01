# HuntLoan — First 24 Hours Playbook

Step-by-step runbook for the transition from DRY_RUN to controlled live operation.
Complete every phase in order. Do NOT skip or parallelize.

---

## Pre-Launch Gate (T-2 hours before going live)

Run these checks in sequence. All must PASS.

```bash
# 1. Old bot is dead (run on VPS)
ssh root@159.89.21.106 "pm2 list && ps aux | grep monitor_base | grep -v grep"
# PASS: pm2 shows no mev-bot, ps shows no monitor_base

# 2. Nonce stability (run on local machine, twice 30s apart)
WALLET=0x3011BfD673a9D09f9761203A7fFCca757Af22587
RPC=https://base-mainnet.g.alchemy.com/v2/rPnh_aKOfxs07PhgeIJJX
cast nonce $WALLET --rpc-url $RPC ; sleep 30 ; cast nonce $WALLET --rpc-url $RPC
# PASS: both numbers are identical

# 3. Wallet ETH balance
cast balance $WALLET --rpc-url $RPC --ether
# PASS: balance > 0.1 ETH

# 4. Test suite
cargo test
# PASS: "13 passed; 0 failed"

# 5. .env has correct contract
grep HUNTLOAN_CONTRACT .env
# PASS: 0x0A0fE1f59D56716aF5c4C9D7688df742EE5949D3

# 6. .env has DRY_RUN=false (only for controlled live)
grep DRY_RUN .env
# PASS: DRY_RUN=false
```

**BLOCKER:** Any FAIL above stops the launch. Fix the specific issue before proceeding.

---

## Hour 0 — Dry Run Baseline (T+0:00)

Run the full pipeline in DRY_RUN mode for at least 30 minutes.

```bash
make dry
```

### What to look for

```
# Expected output pattern:
INFO  huntloan: Config loaded mode=DRY_RUN rpc=... contract=0x0A0fe...
INFO  huntloan: [HuntLoanEngine] Pipeline active — DRY_RUN=true
INFO  huntloan: Scan complete candidates=N rpc_batches=M liquidatable=K
INFO  huntloan: Best opportunity selected borrower=0x... hf=0.97 net_profit_usd=234
INFO  huntloan: DRY_RUN — tx NOT sent borrower=0x...
```

**PASS criteria:**
- Engine connects WS within 10 seconds (`[HuntLoanEngine] Subscribing to new block headers`)
- Scan completes every block without ERROR lines
- At least one "Best opportunity selected" seen within 30 minutes (confirms watchlist is live)
- "DRY_RUN — tx NOT sent" appears for every opportunity — **no tx broadcast**

**FAIL responses:**

| Log line | Meaning | Action |
|---|---|---|
| `WebSocket connection failed` | Bad WS_RPC_URL | Fix WS_RPC_URL in .env |
| `Failed to load reserve cache` | Aave V3 RPC call failed | Check RPC_URL, rate limits |
| `Cannot read watchlist` | Missing watchlist.json | Run `echo '[]' > watchlist.json`, engine will refresh via Goldsky |
| `HUNTLOAN_CONTRACT is not set` | Zero address in env | Fix HUNTLOAN_CONTRACT in .env |
| Scan shows 0 opportunities for > 1 hour | Watchlist empty or HF regime stable | Check Goldsky fetch, review candidates count |

---

## Hour 1 — Soft-Live Preview (T+1:00)

Switch to SOFT_LIVE mode. The engine resolves the real nonce and encodes the full calldata
but does NOT broadcast. Confirm the tx parameters look sane.

```bash
make soft
```

### What to look for

```
INFO  huntloan: Config loaded mode=SOFT_LIVE
INFO  huntloan: SOFT_LIVE — full tx preview (NOT broadcast)
  mode=SOFT_LIVE
  to=0x0A0fE1f59D56716aF5c4C9D7688df742EE5949D3
  chain_id=8453
  nonce=42
  max_fee_gwei=2
  max_priority_gwei=1
  gas_limit=960000
  value_wei=0
  calldata_bytes=132
  calldata=0x...
  borrower=0x...
  debt_to_repay=50000
  collateral=0x...
  debt_asset=0x...
  estimated_profit=234
```

**PASS criteria:**
- `to` = `0x0A0fE1f59D56716aF5c4C9D7688df742EE5949D3` (correct contract)
- `chain_id` = `8453`
- `nonce` is a reasonable number (matches `cast nonce $WALLET --rpc-url $RPC`)
- `calldata_bytes` = 132 (standard `requestFlashLiquidation` ABI = 4 selector + 4×32)
- `value_wei` = 0 (no ETH sent)
- `max_fee_gwei` > 0

**FAIL responses:**

| Issue | Action |
|---|---|
| `to` is 0x000...0 | HUNTLOAN_CONTRACT not set — STOP, fix .env |
| `chain_id` != 8453 | Wrong network — STOP |
| `calldata_bytes` != 132 | ABI mismatch — check sol! interface matches deployed contract |
| `nonce` = 18446744073709551615 (u64::MAX) | RPC nonce fetch failed — check RPC_URL connectivity |

---

## Hour 2 — Controlled Live Trial (T+2:00)

**Only proceed if Hours 0 and 1 both PASS.**

Start with tight caps: 0.002 ETH max gas, 0.005 ETH max bribe, $20 minimum profit.

```bash
# Run in foreground, watch logs directly
make live-controlled
```

For background + log file:
```bash
make live-controlled >> logs/huntloan-$(date +%Y%m%d).log 2>&1 &
HUNTLOAN_PID=$!
echo "HuntLoan PID: $HUNTLOAN_PID"
```

### What a successful first execution looks like

```
INFO  huntloan: Broadcasting liquidation tx attempt=1 borrower=0x... max_fee_gwei=2 nonce=42
INFO  huntloan: Tx submitted — waiting for receipt tx_hash=0xabc...
INFO  huntloan: Tx confirmed tx_hash=0xabc... block=12345678 gas_used=650000 send_latency_ms=1200
INFO  huntloan: Liquidation complete tx_hash=0xabc... scan_ms=180 sim_ms=90 exec_ms=1200 total_ms=1470
```

### What to monitor every 30 minutes

```bash
# Check wallet ETH balance
cast balance 0x3011BfD673a9D09f9761203A7fFCca757Af22587 \
  --rpc-url $RPC_URL --ether

# Check contract profit accumulation (USDC, 6-dec)
cast call 0x0A0fE1f59D56716aF5c4C9D7688df742EE5949D3 \
  "totalProfit()(uint256)" --rpc-url $RPC_URL

# Count recent confirmations
grep "Tx confirmed" logs/huntloan-$(date +%Y%m%d).log | tail -5

# Count errors
grep "ERROR\|WARN" logs/huntloan-$(date +%Y%m%d).log | tail -20
```

---

## PASS / FAIL Criteria for First 24 Hours

### Financial health

| Metric | PASS | FAIL → Action |
|---|---|---|
| Gas used per tx | < 800K units | > 1M units → review gas_limit calc, check for revert patterns |
| Actual profit vs estimated | within ±20% | > 20% deviation → review math::simulate and bonus_bps values |
| Wallet ETH balance | > 0.05 ETH | < 0.05 ETH → STOP, refund wallet immediately |
| Any tx with status=0 (revert) | 0 reverts | Any revert → see revert response below |

### Operational health

| Metric | PASS | FAIL → Action |
|---|---|---|
| WS reconnections | 0 in 24h | > 0 → add reconnect logic or switch RPC provider |
| Blocks processed | > 95% of emitted blocks | < 95% → check scan_ms, may need multicall chunk size reduction |
| Telegram alerts received | Boot + each execution | Silent > 30 min → check Telegram credentials |

---

## Revert Response Procedure

If a tx confirms with `status=0` (on-chain revert):

```bash
# 1. Find the tx hash from logs
grep "Tx confirmed" logs/huntloan-$(date +%Y%m%d).log | tail -1

# 2. STOP the engine immediately
kill $HUNTLOAN_PID   # or: systemctl stop huntloan

# 3. Decode the revert reason
cast run <TX_HASH> --rpc-url $RPC_URL 2>&1 | grep -i revert
# OR:
cast call 0x0A0fE1f59D56716aF5c4C9D7688df742EE5949D3 \
  "requestFlashLiquidation(address,uint256,address,address)" \
  <DEBT_ASSET> <DEBT_AMOUNT> <COLLATERAL> <BORROWER> \
  --rpc-url $RPC_URL 2>&1

# 4. Diagnose by revert reason:
```

| Revert | Root cause | Fix |
|---|---|---|
| `SwapFailed` | No liquid swap route for this asset pair | eth_call should have caught this — check simulator swap validation |
| `OnlyOperator` | msg.sender != operator wallet | Contract HUNTLOAN_CONTRACT mismatch in .env |
| `LiquidationUnprofitable` | Swap slippage higher than expected at execution | Tighten MIN_PROFIT_USD, wait for lower slippage periods |
| `OnlyAavePool` | Reentrancy attempt or wrong callback chain | Should never happen — contact team immediately |
| `ContractSettled` | settle() was called | Check maturityTime — 6 months from deploy (Sept 2026) |
| Generic revert | Position already liquidated by competitor | Normal MEV race — no action needed |

```bash
# 5. Only restart after root cause is confirmed and fixed
make live-controlled
```

---

## Gas Spike Response

If Base L2 base fee spikes (unusual, but possible during L1 congestion):

```bash
# Check current base fee
cast rpc eth_getBlockByNumber "latest" false --rpc-url $RPC_URL | \
  python3 -c "import sys,json; b=json.load(sys.stdin); print(int(b['baseFeePerGas'],16)/1e9,'gwei')"

# If base fee > 0.1 gwei (extremely high for Base):
# Engine will self-limit via MAX_GAS_COST_WEI cap (0.002 ETH in controlled mode)
# Watch for "gas X > cap Y" log lines — these are self-protection, not errors
```

---

## Emergency Stop Commands

```bash
# Option 1: Kill by PID (foreground process)
kill $HUNTLOAN_PID

# Option 2: Kill by name (background)
pkill -f "huntloan"

# Option 3: systemd (if deployed as service)
systemctl stop huntloan

# Verify stopped
ps aux | grep huntloan | grep -v grep
# Expected: no output

# Verify nonce is not still incrementing
cast nonce 0x3011BfD673a9D09f9761203A7fFCca757Af22587 --rpc-url $RPC_URL
sleep 10
cast nonce 0x3011BfD673a9D09f9761203A7fFCca757Af22587 --rpc-url $RPC_URL
# Expected: identical values
```

### Cancel a stuck pending transaction

If the engine sent a tx and then crashed before the receipt was returned:

```bash
# Find the stuck nonce
cast nonce 0x3011BfD673a9D09f9761203A7fFCca757Af22587 --rpc-url $RPC_URL
# This returns the NEXT nonce — subtract 1 to get the stuck tx's nonce

# Send a 0-value replacement tx at the same nonce with higher fee
cast send 0x3011BfD673a9D09f9761203A7fFCca757Af22587 \
  --value 0 \
  --nonce <STUCK_NONCE> \
  --max-fee-per-gas 20000000000 \
  --private-key $PRIVATE_KEY \
  --rpc-url $RPC_URL
```

---

## Hour 24 — Graduation Decision

After 24 hours of controlled live operation, review:

```bash
# Summary stats
echo "=== Executions ==="
grep "Liquidation complete" logs/huntloan-*.log | wc -l

echo "=== Errors ==="
grep "^ERROR" logs/huntloan-*.log | wc -l

echo "=== Reverts ==="
grep "Tx reverted" logs/huntloan-*.log | wc -l

echo "=== Contract profit (USDC 6-dec) ==="
cast call 0x0A0fE1f59D56716aF5c4C9D7688df742EE5949D3 \
  "totalProfit()(uint256)" --rpc-url $RPC_URL

echo "=== Wallet ETH balance ==="
cast balance 0x3011BfD673a9D09f9761203A7fFCca757Af22587 \
  --rpc-url $RPC_URL --ether
```

**Graduate to full production (`make run`) only when:**
- [ ] 0 reverts in 24 hours
- [ ] Profit within ±20% of simulation estimates
- [ ] Wallet balance > 0.1 ETH
- [ ] No WARN lines about nonce conflicts or RPC errors
- [ ] Telegram alerts received consistently

**If any graduation criteria fails:**
- Return to `make soft` for another 12-hour review cycle
- Do NOT increase caps until root cause of any issue is understood

---

## Escalating to Full Production Caps

Once 24h controlled live passes, switch to full .env caps:

```bash
# Edit .env — remove or increase caps
# MAX_GAS_COST_WEI=8000000000000000   (0.008 ETH — default)
# MAX_BRIBE_WEI=50000000000000000     (0.05 ETH — default)
# MIN_PROFIT_USD=10                   (default)
# Then:
make run
```

---

## Quick Reference Card

| Command | Mode | Sends tx? | When to use |
|---|---|---|---|
| `make test` | Test | No | Always first |
| `make dry` | DRY_RUN | No | Development, verification |
| `make soft` | SOFT_LIVE | No | Pre-live calldata inspection |
| `make live-controlled` | LIVE tight caps | **Yes** | First 24h live trial |
| `make run` | LIVE full caps | **Yes** | After controlled trial passes |

---

## Session Report Template

Append one block per run session to this file. Fill in every field.

```
═══════════════════════════════════════════════════════════════
SESSION REPORT
Date       : YYYY-MM-DD HH:MM UTC
Mode       : DRY_RUN / SOFT_LIVE / LIVE_CONTROLLED / LIVE
Duration   : Xh Ym
Block range: START_BLOCK → END_BLOCK
═══════════════════════════════════════════════════════════════

SCANNER
  Candidates watched  : N
  Blocks scanned      : N
  Opportunities found : N (N unique borrowers)
  Delta-neutral skips : N
  Goldilocks skips    : N

SIMULATOR
  eth_call attempts   : N
  Passed (profitable) : N
  Failed (revert)     : N
  Failure reasons     : [SwapFailed / already liquidated / slippage]

EXECUTOR   (DRY_RUN: fill "N/A")
  Txs broadcast       : N
  Txs confirmed       : N
  Txs reverted        : N
  Circuit breaker hit : YES / NO (count: N)
  Consecutive reverts : max observed = N

FINANCIAL
  Gross profit (USD)  : $N (predicted: $N  delta: N%)
  Gas spent (ETH)     : N ETH
  Flash fees (USD)    : $N
  Net profit (USD)    : $N
  Wallet ETH start    : N ETH
  Wallet ETH end      : N ETH

TIMING (avg per block, ms)
  scan_ms             : N
  sim_ms              : N
  exec_ms             : N
  total_ms            : N

INCIDENTS
  WS disconnections   : N
  RPC error streak    : max = N (circuit breaker at 10)
  Nonce conflicts     : N
  Unexpected warnings : [describe or N/A]

ROOT CAUSE (if any failure occurred)
  Issue               : [describe]
  Root cause          : [slippage / bonus mismatch / competition / RPC / other]
  Fix applied         : [code change / config change / N/A]
  Regression test     : [added / not needed]

NEXT SESSION PLAN
  Mode                : DRY_RUN / SOFT_LIVE / LIVE_CONTROLLED / LIVE
  Cap changes         : [describe or "none"]
  Watchlist size      : [keep / expand to N addresses]
  Other               : [describe or "none"]

SIGNED OFF BY: [your name]
═══════════════════════════════════════════════════════════════
```

---

## Example Completed Session Report

```
═══════════════════════════════════════════════════════════════
SESSION REPORT
Date       : 2026-03-01 18:00 UTC
Mode       : LIVE_CONTROLLED
Duration   : 2h 15m
Block range: 27,450,000 → 27,454,000
═══════════════════════════════════════════════════════════════

SCANNER
  Candidates watched  : 150
  Blocks scanned      : 4,000
  Opportunities found : 3 (3 unique borrowers)
  Delta-neutral skips : 12
  Goldilocks skips    : 5

SIMULATOR
  eth_call attempts   : 3
  Passed (profitable) : 2
  Failed (revert)     : 1
  Failure reasons     : [SwapFailed — no direct WBTC/USDC pool on Aerodrome]

EXECUTOR
  Txs broadcast       : 2
  Txs confirmed       : 2
  Txs reverted        : 0
  Circuit breaker hit : NO
  Consecutive reverts : max observed = 0

FINANCIAL
  Gross profit (USD)  : $312 (predicted: $290  delta: +7.5%)
  Gas spent (ETH)     : 0.0008 ETH
  Flash fees (USD)    : $5
  Net profit (USD)    : $305
  Wallet ETH start    : 0.25 ETH
  Wallet ETH end      : 0.2492 ETH

TIMING (avg per block, ms)
  scan_ms             : 185
  sim_ms              : 92
  exec_ms             : 1240
  total_ms            : 1520

INCIDENTS
  WS disconnections   : 0
  RPC error streak    : max = 0
  Nonce conflicts     : 0
  Unexpected warnings : None

ROOT CAUSE (if any failure occurred)
  Issue               : SwapFailed on WBTC→USDC route
  Root cause          : No direct Uniswap V3 or Aerodrome WBTC/USDC pool on Base
  Fix applied         : None yet — eth_call simulation correctly rejected this opportunity
  Regression test     : N/A (simulation filter works correctly)

NEXT SESSION PLAN
  Mode                : LIVE_CONTROLLED (continue)
  Cap changes         : None — 2 successes, 0 reverts
  Watchlist size      : Expand to 300 addresses (next Goldsky refresh)
  Other               : Monitor WBTC positions — may need multi-hop routing

SIGNED OFF BY: operator
═══════════════════════════════════════════════════════════════
```
