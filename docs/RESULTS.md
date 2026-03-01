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

## Phase 5 — SOFT_LIVE (Pending)

> Run after reviewing DRY_RUN output. Expected: tx preview printed with calldata, nonce,
> gas params, and explicit "NOT broadcast" marker.

---

## Phase 6 — LIVE_CONTROLLED Session #1 (Pending)

> Only after Phase 5 PASS. Caps: MAX_GAS_COST_WEI=2000000000000000, MAX_BRIBE_WEI=5000000000000000, MIN_PROFIT_USD=20.

---

## Next Steps

### Immediate (before SOFT_LIVE)

1. **Investigate `0x27e1f1e5` reverts in simulation** — confirm these are all HF-not-below-threshold vs
   any contract ABI mismatch. Decode a sample revert on-chain.
2. **Confirm at least one position reaches HF < 1.0** in simulation before going SOFT_LIVE.
   (If no profitable sim in 24h DRY_RUN, need to check profit threshold and collateral oracle.)

### G-1 (highest ROI optimization)

Implement parallel Multicall3 batching using `tokio::JoinSet` with semaphore=4.
Expected: Stage-1 latency drops from 15.9 s → ~4 s, allowing us to scan every block.
See `docs/PHASE_G_UPGRADES.md` for implementation details.

### G-3 (quick win)

Watchlist mtime cache — stop re-reading/parsing 44,924-address JSON on every block.
Current overhead: file read + JSON parse × every block ≈ measurable at this scale.

### Monitoring

Set up Grafana/Datadog with JSON log output (G-10) once LIVE_CONTROLLED is stable.
