# HuntLoan — System Architecture

## Overview

HuntLoan is an automated Aave V3 flash-loan liquidation engine deployed on Base mainnet.
It monitors undercollateralised borrowing positions, simulates liquidation profitability
on-chain, and executes flash-loan liquidations atomically when profitable.

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         HuntLoan System                                 │
│                                                                         │
│  ┌──────────┐   block   ┌──────────────────────────────────────────┐   │
│  │  Base    │ ────────► │           HuntLoanEngine                  │   │
│  │  WS RPC  │  headers  │  (engine.rs — pipeline coordinator)       │   │
│  └──────────┘           └──┬────────────┬────────────────────┬─────┘   │
│                            │            │                    │          │
│                    ┌───────▼──┐  ┌──────▼──────┐  ┌─────────▼──────┐  │
│                    │ scanner  │  │  simulator   │  │   executor     │  │
│                    │ (Stage1) │  │  (Stage 2)   │  │  (Stage 3)     │  │
│                    └───────┬──┘  └──────┬───────┘  └─────────┬──────┘  │
│                            │            │                    │          │
│                    ┌───────▼──────────────────────────────────▼──────┐  │
│                    │         Supporting Modules                       │  │
│                    │  reserves | gas | math | oracle | velocity      │  │
│                    │  discovery | alerts | config | constants        │  │
│                    └───────────────────────────────────────────────┘  │
│                                                                         │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │                    Base Mainnet                                   │  │
│  │  HuntLoanFlashReceiver.sol  ·  Aave V3  ·  Uniswap V3/Aerodrome  │  │
│  └──────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Execution Pipeline (per block)

```
WebSocket block header received
  └─► engine.rs::process_block()
        │
        ├─ 1. oracle::fetch_eth_price_usd()     [Chainlink + Binance fallback]
        │
        ├─ 2. scanner::scan()                   [Multicall3 batch → reserve resolve]
        │       ├─ Stage 1: getUserAccountData × 500 per RPC call
        │       ├─ Stage 2: reserve resolution (collateral/debt assets)
        │       └─ Stage 3: delta-neutral filter + profit pre-screen
        │
        ├─ 3. scanner::scan_warm()              [HF 1.07–1.15 → VelocityEngine]
        │
        ├─ 4. [P2] blacklist filter + STRONG_HF_THRESHOLD filter
        │
        ├─ 5. [P3] score-sort candidates (1/HF × bonus × log-debt)
        │
        ├─ 6. [P3] simulator × N (parallel JoinSet, MAX_PARALLEL_SIMS=4)
        │       └─ margin filter (MIN_MARGIN_BPS)
        │
        ├─ 7. [P2] daily budget pre-check (gas + bribe caps)
        │
        └─ 8. executor::{execute | execute_parallel}
                ├─ DRY_RUN=true  → log + return (no tx sent)
                ├─ SOFT_LIVE     → sign + print full preview + return
                └─ LIVE          → EIP-1559 tx → Base mempool → receipt
                      ├─ on success: fmt_executed alert + trades.csv row
                      └─ on failure: blacklist target + fmt_failed_exec alert
```

**Timing targets (Base, 2-second blocks):**

| Stage | Target | Notes |
|---|---|---|
| Scan (500 candidates) | < 200ms | Multicall3 batch |
| Simulation (eth_call) | < 100ms | Single RPC call |
| Execution (to mempool) | < 50ms | Private RPC preferred |
| Total block-to-mempool | < 400ms | Well within 2-second block window |

---

## Module Reference

### `src/engine.rs` — Pipeline coordinator
- Manages WebSocket block subscription
- Loads `ReserveCache` at startup
- Runs periodic discovery refresh (every 300 blocks ≈ 10 min)
- Coordinates scan → simulate → execute flow
- Fires Telegram alerts at 4 points (critical, strike, profit, failed)

### `src/scanner.rs` — Liquidation opportunity scanner
- `scan()` — 3-stage pipeline returning `Vec<Opportunity>`
- `scan_warm()` — cheap HF range scan for velocity tracking (no reserve resolution)
- Batches 500 addresses per Multicall3 call
- Goldilocks filter: $5K–$500K debt only
- Delta-neutral filter: skips positions where collateral/debt are same price family

### `src/simulator.rs` — On-chain simulation
- `simulate_on_chain()` — `eth_call` to verify no revert before broadcast
- Gas estimation via `eth_estimateGas`
- Profitability check using `math::simulate`
- Returns `SimOutput { passes, estimated_gas, net_profit_usd, revert_reason }`

### `src/executor.rs` — Transaction broadcast
- `execute()` — single-shot with up to 3 retries, +15% fee bump per retry
- `execute_parallel()` — dual-shot (STRIKE + KILL) with adjacent nonces for high-conviction targets
- EIP-1559 fee computation via `gas::compute_gas_tier()`
- Private RPC submission when `PRIVATE_RPC_URL` is set
- Optimistic nonce caching with chain resync on error

### `src/reserves.rs` — Aave V3 reserve resolution
- `ReserveCache::load()` — fetches all Aave V3 reserves at startup
- `resolve_positions()` — identifies actual collateral/debt token per borrower
- Determines `is_delta_neutral` flag and `bonus_bps` (liquidation bonus)

### `src/velocity.rs` — HF trend engine
- `VelocityEngine::record(addr, hf)` — accumulates HF observations per borrower
- `eta_minutes(addr) -> Option<f64>` — linear regression to predict ETA until HF = 1.0
- Garbage collects observations older than 1 hour
- Powers the ETA field in Telegram critical alerts

### `src/discovery.rs` — Watchlist population
- Queries Goldsky subgraph (Aave V3 Base) for active borrowers
- Cursor-based pagination, 1000 borrowers per page
- `refresh_watchlist(path)` — overwrites `watchlist.json`
- Runs at startup + every 300 blocks (background task)

### `src/gas.rs` — EIP-1559 fee strategy
- Three tiers: `Probe | Strike | Kill`
- Three regimes: `Stable | Volatile | Crash`
- `select_tier(hf, eta_min)` — picks tier from urgency
- `compute_gas_tier(base_fee, priority_fee, tier, regime)` — final fee params
- `compute_bribe_wei(gross_profit_wei, fraction)` — bribe calc, hard-capped at 0.05 ETH

### `src/math.rs` — Profit simulation
- `simulate(debt, collateral_usd, bonus_bps, base_fee, eth_price)` → `SimResult`
- Gross profit = collateral seized × bonus − debt repaid
- Net profit = gross − flash loan premium (0.05%) − estimated gas cost

### `src/oracle.rs` — ETH/USD price feed
- Primary: Chainlink AggregatorV3 on Base
- Fallback: Binance REST API (`api.binance.com/api/v3/ticker/price`)
- Returns cached value (< 60s old) or fetches fresh

### `src/alerts.rs` — Telegram notifications (Phase 1)
- **4 alert classes only**: `fmt_executed`, `fmt_failed_exec`, `fmt_circuit_breaker`, `fmt_summary`
- Arabic titles + icon prefix per class (🐺🔥 / 🐺❌ / 🚨 / 📊)
- `AlertStats` global singleton (`OnceLock`) tracks blocks, opps, sims, execs, PnL
- Per-category rate limiter via `throttle_guard()` (keyed on category + target)
- Hourly summary fired from `engine.run()` loop every `SUMMARY_INTERVAL_SECONDS`
- All alerts are `tokio::spawn`ed; never blocks the pipeline

### `src/trades.rs` — CSV trade log (Phase 5)
- Appends one row to `logs/trades.csv` per confirmed broadcast
- Auto-creates `logs/` directory and CSV header on first write
- 16 columns: timestamp, tx_hash, target, assets, financials, latencies

### `src/config.rs` — Runtime configuration
- Reads from environment variables (`.env` file)
- Legacy variable fallbacks: `BASE_RPC_URL` → `RPC_URL`, `EXECUTOR_ADDRESS` → `HUNTLOAN_CONTRACT`
- Key fields: `rpc_http`, `rpc_ws`, `huntloan_addr`, `dry_run`, `min_profit_usd`, `watchlist_path`

---

## Smart Contract: `HuntLoanFlashReceiver.sol`

**Deployed:** `0x0A0fE1f59D56716aF5c4C9D7688df742EE5949D3` (Base mainnet)

### Entry Point

```
operator wallet
  └─► requestFlashLiquidation(debtAsset, debtAmount, collateralAsset, borrower)
        └─► Aave V3: POOL.flashLoanSimple(this, debtAsset, debtAmount, ...)
              └─► executeOperation() [Aave callback]
                    ├─ 1. Approve POOL to pull debt token
                    ├─ 2. POOL.liquidationCall(collateral, debt, borrower, amount, false)
                    ├─ 3. _swapCollateralToDebt(collateral, debt, seized, owed)
                    │       Try: Uniswap V3 (0.05%) → (0.3%) → (1%)
                    │       Try: Aerodrome volatile → stable
                    │       Revert: SwapFailed if no route works
                    ├─ 4. Approve POOL to pull repayment (amount + 0.05% premium)
                    └─ 5. Surplus stays in contract as profit (totalProfit +=)
```

### Security Properties

| Property | Implementation |
|---|---|
| Only operator can call `requestFlashLiquidation` | `if (msg.sender != operator) revert OnlyOperator()` |
| Only Aave pool can call `executeOperation` | `if (msg.sender != address(POOL)) revert OnlyAavePool()` |
| Flash loan always fully repaid | `LiquidationUnprofitable` revert if `received < owed` |
| No reentrancy vector | Single flash loan per call; state cleared after callback |
| Approval minimised | Exact `approve(amount)` only, reset to 0 after each swap |

### Swap Route Priority

1. Uniswap V3 — 0.05% fee (stablecoins, tight pairs)
2. Uniswap V3 — 0.30% fee (ETH-paired assets)
3. Uniswap V3 — 1.00% fee (long-tail assets)
4. Aerodrome — volatile pool
5. Aerodrome — stable pool

If no route returns `>= owed`, reverts with `SwapFailed`.

---

## Key Addresses (Base Mainnet)

| Name | Address |
|---|---|
| HuntLoanFlashReceiver (active) | `0x0A0fE1f59D56716aF5c4C9D7688df742EE5949D3` |
| Aave V3 Pool | `0xA238Dd80C259a72e81d7e4664a9801593F98d1c5` |
| Aave Data Provider | `0x2d8A3C5677189723C4cB8873CfC9C8976FDF38Ac` |
| Aave PoolAddressesProvider | `0xe20fCBdBfFC4Dd138cE8b2E6FBb6CB49777ad64D` |
| Uniswap V3 Router | `0x2626664c2603336E57B271c5C0b26F421741e481` |
| Aerodrome Router | `0xcF77a3Ba9A5CA399B7c97c74d54e5b1Beb874E43` |
| Multicall3 | `0xcA11bde05977b3631167028862bE2a173976CA11` |
| WETH | `0x4200000000000000000000000000000000000006` |
| USDC | `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913` |
| Operator wallet | `0x3011BfD673a9D09f9761203A7fFCca757Af22587` |

---

## Dependency Graph

```
main.rs
  └─► engine.rs
        ├─► scanner.rs ──► reserves.rs
        │                ├─► constants.rs
        │                └─► math.rs
        ├─► simulator.rs ──► math.rs
        ├─► executor.rs ──► gas.rs
        ├─► velocity.rs
        ├─► discovery.rs
        ├─► oracle.rs
        ├─► alerts.rs
        └─► config.rs
```

---

## Design Decisions

**Why flash loans, not capital-based?**
Flash loans require zero pre-positioned capital per transaction. The full liquidation is
self-funded within a single atomic transaction: borrow → liquidate → swap → repay.
Capital risk per transaction = 0 (gas aside).

**Why Rust?**
- Compile-time memory safety eliminates a class of runtime panics
- `tokio` async runtime enables concurrent block processing
- `alloy` provides type-safe ABI encoding/decoding
- Single binary with no runtime dependency

**Why Multicall3 for scanning?**
500 `getUserAccountData` calls in one RPC round-trip vs 500 sequential calls.
Reduces scan time from ~50s to < 200ms.

**Why event-driven (WebSocket) instead of polling?**
The legacy Node.js bot polled every 400ms. Base produces blocks every 2 seconds.
WebSocket subscription receives each block header within milliseconds of finalization,
giving the engine more reaction time within the block window.

**Why parallel dual-shot execution?**
For high-conviction opportunities (>= $15K estimated profit), two transactions are fired
with adjacent nonces (N and N+1). STRIKE tier = competitive fees, KILL tier = aggressive.
Whichever lands first captures the liquidation; the second reverts harmlessly.
The maximum wasted gas is one failed tx (~800K gas ≈ $0.10 on Base).
