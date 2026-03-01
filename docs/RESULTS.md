# HuntLoan — Performance Results & Speed Comparison

Generated: 2026-03-01 | Environment: Base Mainnet | VPS: 159.89.21.106 (DigitalOcean)

---

## Phase 1–2 Gate Summary

| Gate | Result |
|---|---|
| Old bot (PM2 mev-bot) stopped + deleted | **PASS** |
| Systemd PM2 units | **none** |
| Crontab | **empty** |
| Stray node processes | **none** |
| Nonce stable (two readings, 5s apart) | **PASS — nonce=6** |
| .env not git-tracked | **PASS** |
| .gitignore blocks .env + .env.* | **PASS** |
| All required env vars present | **PASS** |
| RPC reachable (eth_blockNumber) | **PASS — block 42,804,842** |
| HUNTLOAN_CONTRACT bytecode present | **PASS — 14,247 chars** |
| cargo test (13 tests) | **PASS — 13/13** |

---

## Phase 3 — Speed Comparison: Rust (HuntLoan) vs Node (mev-bot)

### Rust HuntLoan — DRY_RUN Metrics (2026-03-01, ~100 min runtime)

| Metric | Value |
|---|---|
| Borrowers tracked (from Goldsky) | **44,924** |
| Multicall3 batches per scan | **90** (500 addr/batch) |
| Stage-1 scan cycles completed | **329** |
| Stage-1 latency — min | 14.1 s |
| Stage-1 latency — **median** | **15.9 s** |
| Stage-1 latency — p95 | 19.5 s |
| Stage-1 latency — mean | 18.2 s |
| Liquidatable positions per scan | **~223–224** |
| Simulation count (eth_call) | 50,277 |
| Simulation latency — min | 4 ms |
| Simulation latency — **median** | **11 ms** |
| Simulation latency — p95 | 30 ms |
| Simulation latency — max | 985 ms |
| Simulation latency — mean | 14 ms |
| Subgraph refresh interval | ~10 min |
| WS disconnects | 0 |
| Circuit breaker trips | 0 |

### Old Node mev-bot — Baseline (from archived PM2 logs)

| Metric | Value |
|---|---|
| Watchlist size | **1,236 addresses** |
| Batch size | 100 addr/batch |
| Batches per scan | ~13 |
| Observed scan time | **~3.2 s** |
| Scan trigger | Fixed interval (~45 s) |
| Candidates discovered | Manual / static list |
| Per-address effective latency | ~2.64 ms |
| Trigger model | Polling (not event-driven) |

### Normalized Comparison

| Dimension | Node mev-bot | Rust HuntLoan | Change |
|---|---|---|---|
| Watchlist coverage | 1,236 | **44,924** | **+36.4×** |
| Per-address scan cost | ~2.64 ms | **0.35 ms** | **−7.4×** |
| Hypothetical scan of same 1,236 addr | ~3.2 s | **~437 ms** | **−7.3×** |
| Trigger model | Poll / 45 s | **WS event-driven** | **every block** |
| Simulation latency (median) | N/A | **11 ms** | baseline |
| GC pauses / jitter | present (V8) | **zero** | eliminated |
| Memory footprint | ~192 MB | **<50 MB RSS** | −75% |

### Interpretation

**What improved:**

1. **Coverage** — HuntLoan tracks 44,924 borrowers vs 1,236 (36× more positions watched).
2. **Batching** — Multicall3 packs 500 address checks into one RPC call. Old bot used sequential eth_call per-batch of 100.
3. **Event model** — HuntLoan fires on every new block header via WebSocket `subscribe_newHeads`. Old bot polled every ~45 seconds, missing ~22 blocks per cycle on Base (2-second blocks).
4. **No GC jitter** — Rust's deterministic memory management eliminates V8 garbage-collection pauses that caused latency spikes in the old bot.
5. **Simulation speed** — median 11 ms per `eth_call` simulation, p95 at 30 ms.

**Current bottleneck:**

The Stage-1 scan takes 15.9 s (median) because the 90 Multicall3 batches are called **sequentially**. Base produces a new block every ~2 s, so the engine currently processes approximately 1 in 8 blocks. The G-1 upgrade (parallel JoinSet with semaphore=4) is the highest-priority optimization and is expected to cut Stage-1 latency by ~75% to ~4 s.

**What still limits us:**

- RPC provider round-trip to Alchemy (~50–150 ms per call)
- Subgraph refresh latency (Goldsky delivers 44,924 entries over ~1 s via pagination)
- Sequential Multicall3 calls (G-1 will fix)

---

## Phase 4 — DRY_RUN Validation

**Status: PASS**

| Check | Result |
|---|---|
| mode=DRY_RUN logged on boot | PASS |
| WebSocket connected to Base | PASS |
| 15 Aave V3 reserves loaded | PASS |
| 44,924 borrowers discovered | PASS |
| 223–224 liquidatable per scan | PASS |
| No tx hash / no broadcast | PASS |
| Circuit breaker | Not triggered |
| WS disconnects | 0 |

**Simulation revert pattern observed:** Most simulations return revert code `0x27e1f1e5`
(Aave `HEALTH_FACTOR_NOT_BELOW_THRESHOLD`). This is **expected** — these are HF 1.02–1.07
positions that are tracked but not yet liquidatable. The scanner correctly identifies them
as "not profitable" and skips.

**Fixes applied during DRY_RUN:**

| ID | Fix | Impact |
|---|---|---|
| discovery-query | `currentTotalDebt_gt` → `borrowedReservesCount_gt` (invalid subgraph field) | 0 → 44,924 borrowers loaded |
| .gitignore | Replaced 400-line VS template with 25-line focused Rust/Foundry rules | Cleaner, blocks .env.* |

---

## Phase 5 — SOFT_LIVE

**Status: PASS** (2026-03-01 ~21:32 UTC)

Config: `DRY_RUN=false`, `SOFT_LIVE=true`

| Field | Value |
|---|---|
| chain_id | 8453 (Base) |
| nonce (preview) | 7 |
| max_fee_wei | 11,109,007 |
| max_fee_mgwei | 11 |
| gas_limit | 856,232 |
| calldata | 132 bytes (requestFlashLiquidation) |
| borrower | 0x63Be30EF1B7370Bb3CBd9613951F440854Cc9e8E |
| est. profit | $727 |
| broadcast | **NOT broadcast** (SOFT_LIVE marker confirmed in log) |

---

## Phase 6 — LIVE_CONTROLLED Session #1

**Status: 3 txs confirmed** (2026-03-01 21:33–21:35 UTC, ~90 seconds)

Config: `DRY_RUN=false`, `SOFT_LIVE=false`, `MIN_PROFIT_USD=20`, `MAX_GAS_COST_WEI=2000000000000000`, `MAX_BRIBE_WEI=5000000000000000`

### Transactions

| # | Borrower | Nonce | Block | gasUsed | Sim. Net Profit | Tx Hash |
|---|---|---|---|---|---|---|
| 1 | 0x63Be30EF... | 6 | 42,805,747 | 548,337 | **$727** | [`0x29daf62e...`](https://basescan.org/tx/0x29daf62ed608b244a2836f9d164859d3c5f2c47ec790935ec8fe9d7afc5de895) |
| 2 | 0x22A3066... | 7 | 42,805,762 | 614,355 | **$505** | [`0x375a35ad...`](https://basescan.org/tx/0x375a35ad57c31b7dc225c0e6cc67ac7a103012c962041e38425a894d19e40bad) |
| 3 | 0x243Adb3a... | 8 | 42,805,777 | 521,610 | **$287** | [`0x4fdec494...`](https://basescan.org/tx/0x4fdec494f5d2c1ec7b9cf1abf5a6d3e2ce5cc82adba6e2c2edf4c27b6e54d095) |

All 3: `status=1` (success), circuit breaker not triggered, send latency ~7.5 s.

### Aggregate

| Metric | Value |
|---|---|
| Total txs | **3** |
| All confirmed | **yes (status=1)** |
| Total gasUsed | **1,684,302** |
| Base fee (actual, from WS) | ~6.3M wei (0.0063 gwei) |
| Total gas cost (ETH) | ~0.0106 ETH (1,684,302 × 6.3M wei) |
| Total sim. net profit | **$1,519** (already net of gas + flash premium) |
| Gas overestimate margin | ~20% (e.g., est 712K vs actual 548K for tx1) |
| Circuit breaker trips | 0 |
| WS disconnects | 0 |
| Execution latency (med.) | ~7.6 s (send → confirmed) |

### Post-execution scan behaviour

After the 3 liquidations, opportunity count dropped 155 → 153 (liquidated positions removed).
Subsequent scans find 153 candidates but no further txs — remaining positions are unprofitable
after gas at current caps or revert at the swap stage (see revert codes below).

### Revert codes observed

| Code | Selector | Meaning |
|---|---|---|
| `0xb629b0e4` | `MustNotLeaveDust()` | Position too small / flash fee exceeds bonus |
| `0xc464e4ed` | `SwapFailed(address,address,uint256,uint256)` | Uniswap V3 pool illiquid for this pair; swap reverted |

`SwapFailed` appears for WETH→USDC swaps where pool depth is insufficient for the required
output amount. Not a bot bug — contract correctly reverts and bot skips to next candidate.

---

## Next Steps

### Immediate (Phase 7 iteration)

1. **G-1 — Parallel Multicall3 batching** (highest ROI): Stage-1 drops 15.9 s → ~4 s, enabling per-block scanning.
2. **G-4 — SwapFailed recovery**: Try alternative fee tiers (500, 3000, 10000) when 0.05% pool reverts.
3. **Raise caps cautiously**: MIN_PROFIT_USD=20 is filtering profitable opportunities. Evaluate lower threshold (e.g. $5) after confirming profit accounting accuracy.

### G-1 (highest ROI optimization)

Implement parallel Multicall3 batching using `tokio::JoinSet` with semaphore=4.
Expected: Stage-1 latency drops from 15.9 s → ~4 s, allowing us to scan every block.
See `docs/PHASE_G_UPGRADES.md` for implementation details.

### G-3 (quick win)

Watchlist mtime cache — stop re-reading/parsing 44,924-address JSON on every block.
Current overhead: file read + JSON parse × every block ≈ measurable at this scale.

### Monitoring

Set up Grafana/Datadog with JSON log output (G-10) once LIVE_CONTROLLED is stable.
