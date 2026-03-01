# Phase G — Unleashed Predator Upgrades

Prioritized backlog of performance, MEV, and resilience improvements.
**Prerequisite:** 24 hours of stable `live-controlled` with 0 circuit-breaker trips.

Items are ordered: implement in sequence. Each is a self-contained PR-sized task.

---

## Priority 1 — Latency & Throughput

### G-1: Parallel simulation across all opportunities (est. −60% sim latency)

**Problem:** Opportunities are simulated sequentially. 5 opportunities × 90ms = 450ms wasted.

**Fix:** Spawn bounded `tokio::JoinSet` over opportunities, limit to 4 concurrent `eth_call`s.

```rust
// engine.rs — replace sequential for loop
let mut join_set = tokio::task::JoinSet::new();
let sem = Arc::new(tokio::sync::Semaphore::new(4)); // max 4 concurrent eth_calls

for opp in &opportunities {
    let opp = opp.clone();
    let provider = provider.clone();
    let config   = self.config.clone();
    let permit   = sem.clone().acquire_owned().await.unwrap();
    join_set.spawn(async move {
        let _permit = permit;
        simulator::simulate_on_chain(&provider, &config, &opp, eth_price, base_fee_wei).await
            .map(|sim| (opp, sim))
    });
}
// collect results...
```

**Test:** Add unit test asserting all opportunities pass through.
**Doc update:** ARCHITECTURE.md — Stage 2 now parallel.

---

### G-2: Precompute calldata per opportunity (est. −5ms per tx)

**Problem:** `abi_encode()` is called once on simulation and again on execution. Minor but consistent.

**Fix:** Add `encoded_calldata: Bytes` to `SimOutput`. Encode once in simulator, reuse in executor.

**Test:** Assert executor calldata matches freshly encoded version.

---

### G-3: Cache watchlist in memory, reload only on file mtime change

**Problem:** `load_candidates()` reads and JSON-parses `watchlist.json` on **every block** (~2s). With 10K+ addresses this adds measurable overhead.

**Fix:** In engine, store `(Vec<Address>, SystemTime)`. Compare file mtime before re-reading.

```rust
struct WatchlistCache {
    addresses: Vec<Address>,
    mtime: SystemTime,
}
```

**Test:** Assert cache is only invalidated when file changes.

---

## Priority 2 — MEV Defense/Offense

### G-4: Private tx submission via Alchemy sendPrivateTransaction

**Problem:** Txs broadcast to public mempool are visible to front-runners. Confirmed losses from sandwich attacks possible on Base.

**Fix:** When `PRIVATE_RPC_URL` is set and Base supports it, route txs through private submission.

Alchemy Base private tx endpoint (if supported):
```
POST https://base-mainnet.g.alchemy.com/v2/YOUR_KEY
{"method": "eth_sendRawTransaction", "params": [...]}
```

Or use Flashbots Protect (if available on Base):
```
PRIVATE_RPC_URL=https://protect.flashbots.net
```

**Config:** `PRIVATE_RPC_URL` is already wired in `config.rs` and `executor.rs`.
**Task:** Verify endpoint works on Base, add integration test (send a 0-value tx, confirm in private).

---

### G-5: Dual-broker submission (public + private simultaneously)

**Problem:** Private RPC may be slower than public mempool for some block builders.

**Fix:** For KILL-tier opportunities, send to both public and private RPC simultaneously using `tokio::join!`. First receipt wins; second reverts harmlessly.

**Risk:** Doubles nonce consumption rate. Add nonce guard.

---

## Priority 3 — Risk Controls

### G-6: Per-asset caps (already partially in constants.rs)

**Problem:** Some assets (e.g. LBTC, ezETH) have thinner markets and higher slippage.

**Fix:** Add `ASSET_BLACKLIST` in constants or env var. Skip any opportunity where `collateral_asset` or `debt_asset` is in the blacklist.

```bash
# .env — comma-separated addresses
ASSET_BLACKLIST=0x...,0x...
```

**Test:** Confirm blacklisted assets are skipped even when HF < 1.0.

---

### G-7: Adaptive slippage threshold per asset family

**Problem:** Swap success rate is not tracked. We don't know which asset pairs consistently fail.

**Fix:** Add `SwapSuccessTracker` in engine: `HashMap<(Address, Address), (success_count, fail_count)>`. If fail_rate > 80% over 10 attempts → add to soft-blacklist for 30 min.

**Test:** Assert pairs with > 80% failure are soft-blacklisted.

---

### G-8: Gas spike circuit breaker (base fee monitoring)

**Problem:** Base L2 can spike during L1 congestion. Current circuit breaker only triggers on execution reverts, not on gas price anomalies.

**Fix:** In `process_block`, check `base_fee_wei > config.max_gas_cost_wei / gas_limit`. If so, skip execution and warn. If this happens 5 consecutive blocks, alert via Telegram.

```rust
let implied_gas_cap = self.config.max_gas_cost_wei / 800_000;
if base_fee_wei > implied_gas_cap * 2 {
    warn!(base_fee = base_fee_wei, cap = implied_gas_cap, "Gas spike — skipping execution");
    return Ok(());
}
```

---

### G-9: WS reconnect with exponential backoff

**Problem:** The engine exits when the WebSocket stream ends. The supervisor (systemd) restarts it, but there is a gap.

**Fix:** Wrap the block stream in a retry loop with exponential backoff (1s, 2s, 4s, 8s, max 30s).

```rust
let mut backoff_s = 1_u64;
loop {
    match run_ws_loop(...).await {
        Ok(()) => break, // clean shutdown
        Err(e) if e.to_string().contains(CIRCUIT_BREAKER) => return Err(e), // stop
        Err(e) => {
            warn!(error = %e, "WS disconnected — retrying in {}s", backoff_s);
            tokio::time::sleep(Duration::from_secs(backoff_s)).await;
            backoff_s = (backoff_s * 2).min(30);
        }
    }
}
```

---

## Priority 4 — Operational

### G-10: Structured JSON logging for external monitoring

**Problem:** Current logging is human-readable text. Difficult to pipe into Grafana/Datadog/Elastic.

**Fix:** Add `tracing-json` formatter, toggle with `RUST_LOG_FORMAT=json`.

```toml
# Cargo.toml
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
```

```rust
// main.rs — add JSON layer if env var set
if std::env::var("RUST_LOG_FORMAT").as_deref() == Ok("json") {
    registry.with(tracing_subscriber::fmt::layer().json()).init();
} else {
    registry.with(tracing_subscriber::fmt::layer()).init();
}
```

---

## Upgrade Completion Tracker

| ID | Title | Status | Priority |
|---|---|---|---|
| G-1 | Parallel simulation | Pending | P1 |
| G-2 | Precompute calldata | Pending | P1 |
| G-3 | Watchlist mtime cache | Pending | P1 |
| G-4 | Private tx (Alchemy) | Pending | P2 |
| G-5 | Dual-broker submission | Pending | P2 |
| G-6 | Per-asset blacklist | Pending | P3 |
| G-7 | Adaptive slippage tracker | Pending | P3 |
| G-8 | Gas spike circuit breaker | Pending | P3 |
| G-9 | WS reconnect + backoff | Pending | P3 |
| G-10 | JSON structured logging | Pending | P4 |
