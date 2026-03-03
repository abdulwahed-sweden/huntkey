//! HuntLoanEngine — full pipeline coordinator.
//!
//! Pipeline: block event → scanner → simulator → executor → contract
//!
//! Architecture:
//!   - WebSocket provider subscribes to new block headers (event-driven, <50ms target)
//!   - HTTP provider (wallet-backed) used for simulation and execution
//!   - Each block triggers: scan → simulate (parallel JoinSet) → execute best opportunity
//!   - Timing metrics logged for every stage
//!   - Telegram alerts fired only on: EXECUTED, FAILED, CIRCUIT_BREAKER, SUMMARY
//!   - VelocityEngine records HF per borrower to compute ETA
//!
//! Phases implemented:
//!   PHASE 1 — High-signal Telegram alerts (4 classes, rate-limited)
//!   PHASE 2 — Execution filters (STRONG_HF, margin, blacklist, daily budget)
//!   PHASE 3 — Parallel JoinSet simulation + score-based candidate priority

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use alloy::primitives::Address;
use alloy::providers::{Provider, ProviderBuilder, WsConnect};
use eyre::{bail, Result, WrapErr};
use futures_util::StreamExt;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tracing::{error, info, warn};

use crate::{
    alerts,
    config::Config,
    constants::{self, PARALLEL_CONVICTION_USD},
    discovery,
    executor::HuntLoanExecutor,
    gas,
    oracle,
    reserves::ReserveCache,
    scanner::{self, Opportunity},
    simulator,
    trades,
    velocity::VelocityEngine,
};

/// Sentinel string embedded in circuit-breaker errors so run() can distinguish
/// them from transient per-block errors and propagate immediately.
const CIRCUIT_BREAKER: &str = "CIRCUIT_BREAKER";

// ── Target scoring ────────────────────────────────────────────────────────────

/// Score an opportunity for simulation priority.
/// Higher = simulate first.
///
/// Formula:
///   (1/HF) × (bonus_bps/100) × log2(debt_usd/1000 + 1)
///
/// Rationale:
///   - Low HF → more urgent, higher competition window closing
///   - High bonus → more gross profit per unit of debt
///   - Larger debt → more absolute profit (log-scaled to avoid $500K dominating)
fn score(opp: &Opportunity) -> f64 {
    let hf_factor     = if opp.health_factor > 0.0 { 1.0 / opp.health_factor } else { 100.0 };
    let bonus_factor  = opp.liquidation_bonus_bps as f64 / 100.0;
    let debt_factor   = ((opp.debt_usd as f64 / 1_000.0) + 1.0).log2();
    hf_factor * bonus_factor * debt_factor
}

// ── Daily budget tracker ──────────────────────────────────────────────────────

struct DailyBudget {
    gas_wei:   u128,
    bribe_wei: u128,
    /// Days since Common Era (from chrono::NaiveDate::num_days_from_ce).
    /// Reset triggers when current day differs from this value.
    reset_day: i32,
}

// ── Top-level engine struct ───────────────────────────────────────────────────

pub struct HuntLoanEngine {
    config:   Arc<Config>,
    executor: HuntLoanExecutor,
    velocity: Mutex<VelocityEngine>,

    /// Counts consecutive execution reverts. Reset to 0 on any success.
    consecutive_reverts: AtomicU64,
    /// Counts consecutive RPC-level errors (scan or sim). Reset on any clean block.
    rpc_error_streak:    AtomicU64,

    // [PHASE 2] Execution filters
    /// Target cooldown blacklist: address → time it was blacklisted.
    blacklist: Mutex<HashMap<Address, Instant>>,
    /// Daily running totals (midnight-reset via chrono).
    daily:     Mutex<DailyBudget>,
    /// ETH price snapshot for 5-min regime detection: (price_usd, snapshot_time).
    last_eth_price: Mutex<(u128, Instant)>,
    /// Watchlist candidates cached in memory — refreshed by background discovery task.
    candidates: Arc<Mutex<Vec<Address>>>,
}

impl HuntLoanEngine {
    /// Build the engine from config.
    pub fn new(config: Config) -> Result<Self> {
        let config = Arc::new(config);
        let executor = HuntLoanExecutor::new(config.clone())?;
        Ok(Self {
            config,
            executor,
            velocity: Mutex::new(VelocityEngine::new()),
            consecutive_reverts: AtomicU64::new(0),
            rpc_error_streak:    AtomicU64::new(0),
            blacklist:      Mutex::new(HashMap::new()),
            daily:          Mutex::new(DailyBudget { gas_wei: 0, bribe_wei: 0, reset_day: 0 }),
            last_eth_price: Mutex::new((0, Instant::now())),
            candidates:     Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Run indefinitely. WebSocket drives the event loop. Circuit-breaker errors propagate.
    pub async fn run(&self) -> Result<()> {
        let ws_url = self.config.rpc_ws.as_deref()
            .ok_or_else(|| eyre::eyre!(
                "WS_RPC_URL is required for event-driven mode. Set it in .env."
            ))?;

        info!("[HuntLoanEngine] Connecting WebSocket → {}", ws_url);
        let ws = WsConnect::new(ws_url);
        let ws_provider = ProviderBuilder::new()
            .connect_ws(ws)
            .await
            .wrap_err("WebSocket connection failed")?;
        let ws_provider = Arc::new(ws_provider);

        // ── Load reserve cache once at startup ───────────────────────────────
        info!("[HuntLoanEngine] Loading Aave V3 reserve cache…");
        let reserve_cache = ReserveCache::load(
            ws_provider.as_ref(),
            constants::AAVE_POOL,
            constants::AAVE_DATA,
        )
        .await
        .wrap_err("Failed to load reserve cache")?;

        info!("[HuntLoanEngine] Subscribing to new block headers…");
        let sub = ws_provider
            .subscribe_blocks()
            .await
            .wrap_err("subscribe_blocks failed")?;
        let mut block_stream = sub.into_stream();

        info!(
            "[HuntLoanEngine] Pipeline active — DRY_RUN={} SOFT_LIVE={} MAX_PARALLEL_SIMS={}",
            self.config.dry_run, self.config.soft_live, self.config.max_parallel_sims
        );

        // Initial watchlist refresh + candidate cache load
        if let Err(e) = discovery::refresh_watchlist(&self.config.watchlist_path).await {
            warn!("[HuntLoanEngine] Initial watchlist refresh failed: {}", e);
        }
        match crate::load_candidates(&self.config.watchlist_path) {
            Ok(v) => {
                info!("[HuntLoanEngine] Loaded {} initial candidates", v.len());
                *self.candidates.lock().await = v;
            }
            Err(e) => warn!("[HuntLoanEngine] Initial candidate load failed: {}", e),
        }

        let mut last_discovery_block: u64 = 0;
        const DISCOVERY_INTERVAL_BLOCKS: u64 = 300; // ~10 min on Base

        // [PHASE 1] Summary timer
        let mut last_summary = Instant::now();

        while let Some(block) = block_stream.next().await {
            let block_start = Instant::now();
            let block_num   = block.inner.number;
            let base_fee: u128 = block.inner.base_fee_per_gas
                .unwrap_or(5_000_000_u64)
                .into();
            tracing::debug!(block = block_num, base_fee_wei = base_fee, "New block");

            // Record block in session stats
            alerts::get_stats().blocks_processed.fetch_add(1, Ordering::Relaxed);

            // Periodic subgraph discovery (background) — refreshes file and updates candidate cache
            if block_num.saturating_sub(last_discovery_block) >= DISCOVERY_INTERVAL_BLOCKS {
                let path           = self.config.watchlist_path.clone();
                let candidates_arc = self.candidates.clone();
                tokio::spawn(async move {
                    if let Err(e) = discovery::refresh_watchlist(&path).await {
                        warn!("[discovery] Background refresh failed: {}", e);
                    } else {
                        match crate::load_candidates(&path) {
                            Ok(v) => {
                                let n = v.len();
                                *candidates_arc.lock().await = v;
                                info!("[discovery] Candidate cache updated: {} addresses", n);
                            }
                            Err(e) => warn!("[discovery] Failed to reload candidates: {}", e),
                        }
                    }
                });
                last_discovery_block = block_num;
            }

            // [PHASE 1] Hourly summary alert
            if last_summary.elapsed() >= Duration::from_secs(self.config.summary_interval_secs) {
                self.fire_summary_alert().await;
                last_summary = Instant::now();
            }

            match self
                .process_block(&ws_provider, block_num, base_fee, block_start, &reserve_cache)
                .await
            {
                Ok(()) => {}
                Err(e) => {
                    if e.to_string().contains(CIRCUIT_BREAKER) {
                        error!("[CIRCUIT BREAKER] Engine stopping: {}", e);
                        // [PHASE 1] Circuit-breaker alert
                        let msg = alerts::fmt_circuit_breaker(
                            "consecutive failures",
                            &e.to_string(),
                        );
                        let _ = alerts::send_telegram(msg, None, 0).await;
                        return Err(e);
                    }
                    error!(block = block_num, error = %e, "Block processing error");
                }
            }
        }

        warn!("[HuntLoanEngine] Block stream ended — shutting down");
        Ok(())
    }

    // ── Summary alert ─────────────────────────────────────────────────────────

    async fn fire_summary_alert(&self) {
        let s = alerts::get_stats();
        let uptime  = s.session_start.elapsed().as_secs();
        let blocks  = s.blocks_processed.load(Ordering::Relaxed);
        let opps    = s.opps_detected.load(Ordering::Relaxed);
        let sims_ok = s.sims_passed.load(Ordering::Relaxed);
        let tried   = s.execs_attempted.load(Ordering::Relaxed);
        let ok      = s.execs_succeeded.load(Ordering::Relaxed);
        let gas_usd = s.gas_cost_gwei.load(Ordering::Relaxed) as f64 / 1e9; // gwei → ETH, approx
        let pnl_usd = s.net_profit_cents.load(Ordering::Relaxed) as f64 / 100.0;
        let top_rev = s.top_reverts(3);

        let msg      = alerts::fmt_summary(uptime, blocks, opps, sims_ok, tried, ok, gas_usd, pnl_usd, &top_rev);
        let interval = self.config.summary_interval_secs;
        let _ = alerts::send_telegram(msg, Some("cat_SUMMARY"), interval).await;
    }

    // ── Per-block pipeline ────────────────────────────────────────────────────

    async fn process_block<P: Provider + Clone + Send + Sync + 'static>(
        &self,
        provider: &Arc<P>,
        block_num: u64,
        base_fee_wei: u128,
        block_start: Instant,
        reserve_cache: &ReserveCache,
    ) -> Result<()> {
        let eth_price = oracle::fetch_eth_price_usd(provider.as_ref()).await;

        // ── Regime detection (5-min ETH price window) ─────────────────────────
        let regime = {
            let mut price_guard = self.last_eth_price.lock().await;
            let (snap_price, snap_time) = *price_guard;
            let r = if snap_price > 0 && eth_price > 0 {
                let pct = (eth_price as f64 - snap_price as f64) / snap_price as f64;
                gas::detect_regime(pct)
            } else {
                gas::Regime::Stable
            };
            // Refresh snapshot every ~5 minutes (skip on oracle failure)
            if eth_price > 0 && (snap_price == 0 || snap_time.elapsed() >= Duration::from_secs(300)) {
                *price_guard = (eth_price, Instant::now());
            }
            r
        };

        // ── Stage 1: SCAN ─────────────────────────────────────────────────────
        let scan_t   = Instant::now();
        let candidates = {
            let guard = self.candidates.lock().await;
            guard.clone()
        };
        if candidates.is_empty() {
            return Ok(());
        }
        let mut opportunities = match scanner::scan(
            provider.as_ref(),
            &self.config,
            &candidates,
            eth_price,
            base_fee_wei,
            reserve_cache,
        )
        .await
        {
            Ok(opps) => {
                self.rpc_error_streak.store(0, Ordering::Relaxed);
                opps
            }
            Err(e) => {
                let streak = self.rpc_error_streak.fetch_add(1, Ordering::Relaxed) + 1;
                warn!(block = block_num, streak = streak, error = %e, "Scan RPC error");
                if streak >= self.config.max_rpc_errors as u64 {
                    bail!(
                        "{} — {} consecutive RPC errors (last: {})",
                        CIRCUIT_BREAKER, streak, e
                    );
                }
                return Ok(());
            }
        };
        let scan_us = scan_t.elapsed().as_micros();

        if opportunities.is_empty() {
            return Ok(());
        }

        // Record stats
        alerts::get_stats().opps_detected.fetch_add(opportunities.len() as u64, Ordering::Relaxed);

        info!(
            block         = block_num,
            scan_us       = scan_us,
            candidates    = candidates.len(),
            opportunities = opportunities.len(),
            "Scan complete"
        );

        // ── Warm-zone scan → velocity engine ─────────────────────────────────
        let warm = scanner::scan_warm(provider.as_ref(), &self.config, &candidates).await;
        {
            let mut ve = self.velocity.lock().await;
            for wc in &warm   { ve.record(wc.borrower, wc.health_factor); }
            for opp in &opportunities { ve.record(opp.borrower, opp.health_factor); }
            ve.maybe_gc();
        }

        // ── [PHASE 1] Approaching alerts (ETA < 10 min) ──────────────────────
        let approaching: Vec<(Address, f64, f64)> = {
            let ve = self.velocity.lock().await;
            warm.iter().filter_map(|wc| {
                ve.eta_minutes(&wc.borrower)
                    .filter(|&eta| eta > 0.0 && eta < 10.0)
                    .map(|eta| (wc.borrower, wc.health_factor, eta))
            }).collect()
        };
        for (addr, hf, eta) in approaching {
            let addr_s = addr.to_string();
            tokio::spawn(async move {
                alerts::send_approaching(&addr_s, hf, eta, 0).await;
            });
        }

        // ── [PHASE 2] Apply blacklist filter ─────────────────────────────────
        {
            let cooldown = Duration::from_secs(self.config.target_cooldown_secs);
            let bl = self.blacklist.lock().await;
            opportunities.retain(|opp| {
                bl.get(&opp.borrower)
                    .map(|t| t.elapsed() >= cooldown) // elapsed >= cooldown → un-blacklisted
                    .unwrap_or(true)                  // not in list → include
            });
        }

        // ── [PHASE 2] STRONG_HF filter ────────────────────────────────────────
        // Skip positions above the strong-HF threshold (barely underwater).
        // Default threshold = 1.0 (disabled). Set STRONG_HF_THRESHOLD=0.90 to restrict.
        let threshold = self.config.strong_hf_threshold;
        if threshold < 1.0 {
            let before = opportunities.len();
            opportunities.retain(|opp| opp.health_factor <= threshold);
            if opportunities.len() < before {
                info!(
                    filtered = before - opportunities.len(),
                    threshold = threshold,
                    "STRONG_HF filter applied"
                );
            }
        }

        if opportunities.is_empty() {
            return Ok(());
        }

        // ── [PHASE 3] Score + sort candidates ────────────────────────────────
        // Highest score → simulated first. Bounded by MAX_PARALLEL_SIMS concurrency.
        opportunities.sort_unstable_by(|a, b| {
            score(b).partial_cmp(&score(a)).unwrap_or(std::cmp::Ordering::Equal)
        });

        // ── Stage 2: SIMULATE (parallel JoinSet) ─────────────────────────────
        let sim_t  = Instant::now();
        let sem    = Arc::new(tokio::sync::Semaphore::new(self.config.max_parallel_sims));
        let mut join_set: JoinSet<(Opportunity, Result<simulator::SimOutput, eyre::Error>)> =
            JoinSet::new();

        for opp in opportunities {
            let sem      = sem.clone();
            let provider = provider.clone();
            let config   = self.config.clone();
            let ep       = eth_price;
            let bf       = base_fee_wei;

            join_set.spawn(async move {
                let _permit = sem.acquire_owned().await.expect("semaphore closed");
                let result  = simulator::simulate_on_chain(
                    provider.as_ref(), &config, &opp, ep, bf,
                ).await;
                (opp, result)
            });
        }

        let mut profitable: Vec<(Opportunity, simulator::SimOutput)> = Vec::new();

        while let Some(task_result) = join_set.join_next().await {
            match task_result {
                Ok((opp, Ok(sim))) if sim.passes => {
                    alerts::get_stats().sims_passed.fetch_add(1, Ordering::Relaxed);
                    profitable.push((opp, sim));
                }
                Ok((opp, Ok(sim))) => {
                    alerts::get_stats().record_revert(
                        sim.revert_reason.as_deref().unwrap_or("sim_fail_no_reason")
                    );
                    warn!(
                        borrower   = %opp.borrower,
                        reason     = ?sim.revert_reason,
                        net_profit = sim.net_profit_usd,
                        "Simulation skip"
                    );
                }
                Ok((opp, Err(e))) => {
                    alerts::get_stats().record_revert("rpc_error");
                    warn!(borrower = %opp.borrower, "Simulate error: {}", e);
                }
                Err(e) => warn!("Sim task join error: {e}"),
            }
        }

        let sim_us = sim_t.elapsed().as_micros();

        if profitable.is_empty() {
            return Ok(());
        }

        // ── [PHASE 2] Margin filter ────────────────────────────────────────────
        // Require net profit margin ≥ MIN_MARGIN_BPS (profit/debt × 10000).
        let min_bps = self.config.min_margin_bps;
        if min_bps > 0 {
            profitable.retain(|(opp, sim)| {
                if opp.debt_usd == 0 { return true; }
                let margin_bps = (sim.net_profit_usd as f64 / opp.debt_usd as f64 * 10_000.0)
                    .max(0.0) as u64;
                margin_bps >= min_bps
            });
        }

        if profitable.is_empty() {
            return Ok(());
        }

        // Best = highest net profit (already well-scored, so take max by net profit)
        let (best_opp, best_sim) = profitable
            .into_iter()
            .max_by_key(|(_, s)| s.net_profit_usd)
            .unwrap();

        info!(
            block          = block_num,
            borrower       = %best_opp.borrower,
            hf             = best_opp.health_factor,
            score          = %format!("{:.2}", score(&best_opp)),
            debt_usd       = best_opp.debt_usd,
            net_profit_usd = best_sim.net_profit_usd,
            est_gas        = best_sim.estimated_gas,
            sim_us         = sim_us,
            "Best opportunity selected"
        );

        // [PHASE 1] OPPORTUNITY alert — fire-and-forget off hot path
        {
            let borrower_s   = best_opp.borrower.to_string();
            let collateral_s = best_opp.collateral_asset.to_string();
            let debt_asset_s = best_opp.debt_asset.to_string();
            let hf           = best_opp.health_factor;
            let debt_usd     = best_opp.debt_usd;
            let net_profit   = best_sim.net_profit_usd;
            let sc           = score(&best_opp);
            tokio::spawn(async move {
                alerts::send_opportunity(
                    &borrower_s, hf, debt_usd, &collateral_s, &debt_asset_s, net_profit, sc,
                ).await;
            });
        }

        // ── Stage 3: EXECUTE ──────────────────────────────────────────────────
        let exec_t = Instant::now();

        // Gas tier + bribe
        let gas_tier_sel = gas::select_tier(best_opp.health_factor, 30.0);
        let gas_tier     = gas::compute_gas_tier(base_fee_wei, 1_000_000_000, gas_tier_sel, regime);
        let gross_profit_wei: u128 = if eth_price > 0 {
            (best_sim.net_profit_usd as f64 / eth_price as f64 * 1e18) as u128
        } else { 0 };
        let bribe_wei = gas::compute_bribe_wei(gross_profit_wei, gas_tier.bribe_fraction);
        let _bribe_eth = bribe_wei as f64 / 1e18;

        // [PHASE 2] Daily budget check — compute outside lock, minimize critical section
        let today = current_utc_day();
        let est_gas_cost = (best_sim.estimated_gas as u128).saturating_mul(base_fee_wei);
        {
            let mut daily = self.daily.lock().await;

            // Midnight reset
            if daily.reset_day != today {
                daily.gas_wei   = 0;
                daily.bribe_wei = 0;
                daily.reset_day = today;
            }

            let gas_ok = self.config.max_daily_gas_wei == u128::MAX
                || daily.gas_wei.saturating_add(est_gas_cost) <= self.config.max_daily_gas_wei;
            let bribe_ok = self.config.max_daily_bribe_wei == u128::MAX
                || daily.bribe_wei.saturating_add(bribe_wei) <= self.config.max_daily_bribe_wei;

            if !gas_ok || !bribe_ok {
                info!("Daily budget cap — skipping execution");
                return Ok(());
            }

            // Pre-reserve
            daily.gas_wei   = daily.gas_wei.saturating_add(est_gas_cost);
            daily.bribe_wei = daily.bribe_wei.saturating_add(bribe_wei);
        }

        // Record execution attempt
        alerts::get_stats().execs_attempted.fetch_add(1, Ordering::Relaxed);

        if best_sim.net_profit_usd >= PARALLEL_CONVICTION_USD as i128 {
            // High-conviction: dual-shot (STRIKE + KILL) in parallel
            let (r1, r2) = self.executor
                .execute_parallel(&best_opp, &best_sim, base_fee_wei, regime)
                .await;

            let exec_us  = exec_t.elapsed().as_micros();
            let total_us = block_start.elapsed().as_micros();

            // Pre-format strings once for alert + trade log
            let borrower_s   = best_opp.borrower.to_string();
            let collateral_s = best_opp.collateral_asset.to_string();
            let debt_asset_s = best_opp.debt_asset.to_string();

            let mut any_confirmed = false;
            let mut alert_sent    = false;
            for (shot, result) in [("STRIKE", r1), ("KILL", r2)] {
                if let Some(r) = result {
                    any_confirmed = true;
                    let gas_cost_wei = r.gas_used as u128 * base_fee_wei;

                    // Update daily actuals
                    {
                        let mut daily = self.daily.lock().await;
                        daily.gas_wei = daily.gas_wei.saturating_add(gas_cost_wei);
                    }

                    // Update session stats
                    alerts::get_stats().execs_succeeded.fetch_add(1, Ordering::Relaxed);
                    let gas_gwei = (gas_cost_wei / 1_000_000_000) as u64;
                    alerts::get_stats().gas_cost_gwei.fetch_add(gas_gwei, Ordering::Relaxed);
                    let profit_cents = (best_sim.net_profit_usd.max(0) * 100) as u64;
                    alerts::get_stats().net_profit_cents.fetch_add(profit_cents, Ordering::Relaxed);

                    info!(
                        shot     = shot,
                        tx_hash  = %r.tx_hash,
                        block    = r.block_number,
                        gas_used = r.gas_used,
                        scan_us  = scan_us,
                        sim_us   = sim_us,
                        exec_us  = exec_us,
                        total_us = total_us,
                        "Parallel shot confirmed"
                    );

                    // [PHASE 1] EXECUTED alert — only fire once (first confirmed shot)
                    if !alert_sent {
                        alert_sent = true;
                        let tx_hash_s = r.tx_hash.to_string();
                        let alert_key = format!("exec-{}", borrower_s);
                        let msg = alerts::fmt_executed(
                            &borrower_s, best_opp.health_factor, best_opp.debt_usd,
                            &collateral_s, &debt_asset_s,
                            best_sim.net_profit_usd, r.gas_used, base_fee_wei,
                            eth_price, &tx_hash_s, r.block_number, 1,
                        );
                        tokio::spawn(async move {
                            let _ = alerts::send_telegram(msg, Some(&alert_key), 0).await;
                        });
                    }

                    // [PHASE 5] Trade log
                    let ts          = chrono_utc_now();
                    let tx_hash_log = r.tx_hash.to_string();
                    trades::append_trade(&trades::TradeRecord {
                        timestamp:          &ts,
                        tx_hash:            &tx_hash_log,
                        target:             &borrower_s,
                        debt_asset:         &debt_asset_s,
                        collateral_asset:   &collateral_s,
                        debt_usd:           best_opp.debt_usd,
                        sim_net_profit_usd: best_sim.net_profit_usd,
                        estimated_gas:      best_sim.estimated_gas,
                        gas_used:           r.gas_used,
                        base_fee_wei,
                        bribe_wei,
                        block_number:       r.block_number,
                        status:             1,
                        scan_us,
                        sim_us,
                        exec_us,
                    });
                }
            }

            if any_confirmed {
                self.consecutive_reverts.store(0, Ordering::Relaxed);
            } else {
                let n = self.consecutive_reverts.fetch_add(1, Ordering::Relaxed) + 1;
                warn!(count = n, "Parallel dual-shot: both shots returned no receipt");
                if n >= self.config.max_consecutive_reverts as u64 {
                    bail!("{} — {} consecutive execution failures", CIRCUIT_BREAKER, n);
                }
            }
        } else {
            // Standard single-shot
            match self.executor.execute(&best_opp, &best_sim, base_fee_wei, regime).await {
                Ok(result) => {
                    self.consecutive_reverts.store(0, Ordering::Relaxed);

                    let exec_us  = exec_t.elapsed().as_micros();
                    let total_us = block_start.elapsed().as_micros();
                    let gas_cost_wei = result.gas_used as u128 * base_fee_wei;

                    // Update daily actuals
                    {
                        let mut daily = self.daily.lock().await;
                        daily.gas_wei = daily.gas_wei.saturating_add(gas_cost_wei);
                    }

                    // Update session stats
                    alerts::get_stats().execs_succeeded.fetch_add(1, Ordering::Relaxed);
                    let gas_gwei = (gas_cost_wei / 1_000_000_000) as u64;
                    alerts::get_stats().gas_cost_gwei.fetch_add(gas_gwei, Ordering::Relaxed);
                    let profit_cents = (best_sim.net_profit_usd.max(0) * 100) as u64;
                    alerts::get_stats().net_profit_cents.fetch_add(profit_cents, Ordering::Relaxed);

                    // Pre-format strings once for alert + trade log
                    let borrower_s   = best_opp.borrower.to_string();
                    let collateral_s = best_opp.collateral_asset.to_string();
                    let debt_asset_s = best_opp.debt_asset.to_string();
                    let tx_hash_s    = result.tx_hash.to_string();

                    info!(
                        tx_hash      = %result.tx_hash,
                        block        = result.block_number,
                        gas_used     = result.gas_used,
                        scan_us      = scan_us,
                        sim_us       = sim_us,
                        exec_us      = exec_us,
                        total_us     = total_us,
                        "Liquidation complete"
                    );

                    // [PHASE 1] EXECUTED alert (confirmed, status=1)
                    {
                        let alert_key = format!("exec-{}", borrower_s);
                        let msg = alerts::fmt_executed(
                            &borrower_s, best_opp.health_factor, best_opp.debt_usd,
                            &collateral_s, &debt_asset_s,
                            best_sim.net_profit_usd, result.gas_used, base_fee_wei,
                            eth_price, &tx_hash_s, result.block_number, 1,
                        );
                        tokio::spawn(async move {
                            let _ = alerts::send_telegram(msg, Some(&alert_key), 0).await;
                        });
                    }

                    // [PHASE 5] Trade log
                    let ts = chrono_utc_now();
                    trades::append_trade(&trades::TradeRecord {
                        timestamp:          &ts,
                        tx_hash:            &tx_hash_s,
                        target:             &borrower_s,
                        debt_asset:         &debt_asset_s,
                        collateral_asset:   &collateral_s,
                        debt_usd:           best_opp.debt_usd,
                        sim_net_profit_usd: best_sim.net_profit_usd,
                        estimated_gas:      best_sim.estimated_gas,
                        gas_used:           result.gas_used,
                        base_fee_wei,
                        bribe_wei,
                        block_number:       result.block_number,
                        status:             1,
                        scan_us,
                        sim_us,
                        exec_us,
                    });
                }
                Err(e) => {
                    let n = self.consecutive_reverts.fetch_add(1, Ordering::Relaxed) + 1;
                    error!(
                        borrower     = %best_opp.borrower,
                        revert_count = n,
                        max          = self.config.max_consecutive_reverts,
                        error        = %e,
                        "Execution failed"
                    );

                    // [PHASE 2] Blacklist the target after failure
                    {
                        let mut bl = self.blacklist.lock().await;
                        bl.insert(best_opp.borrower, Instant::now());
                    }

                    // Release pre-reserved daily budget
                    {
                        let est_gas_cost = (best_sim.estimated_gas as u128).saturating_mul(base_fee_wei);
                        let mut daily = self.daily.lock().await;
                        daily.gas_wei   = daily.gas_wei.saturating_sub(est_gas_cost);
                        daily.bribe_wei = daily.bribe_wei.saturating_sub(bribe_wei);
                    }

                    // [PHASE 1] FAILED alert (rate-limited per category)
                    let reason   = e.to_string();
                    let borrower = format!("{}", best_opp.borrower);
                    let rl       = self.config.alert_rate_limit_secs;
                    tokio::spawn(async move {
                        let hint = if reason.contains("outcompeted") || reason.contains("nonce") {
                            "position likely taken — cooldown active"
                        } else if reason.contains("gas") {
                            "gas spike — check MAX_GAS_COST_WEI"
                        } else {
                            "check logs for revert detail"
                        };
                        let msg = alerts::fmt_failed_exec(&reason, &borrower, hint);
                        let key = format!("fail-{}", &borrower[..10.min(borrower.len())]);
                        let _ = alerts::send_telegram(msg, Some(&key), rl).await;
                    });

                    if n >= self.config.max_consecutive_reverts as u64 {
                        bail!(
                            "{} — {} consecutive execution reverts (last: {})",
                            CIRCUIT_BREAKER, n, e
                        );
                    }
                }
            }
        }

        Ok(())
    }
}

/// Returns current UTC time as RFC-3339 string (e.g. "2026-03-02T15:30:00Z").
fn chrono_utc_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Returns current UTC date as days since the Common Era.
/// Monotonically increasing — used only for == comparisons between calls.
fn current_utc_day() -> i32 {
    use chrono::Datelike;
    chrono::Utc::now().date_naive().num_days_from_ce()
}
