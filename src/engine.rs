/// HuntLoanEngine — full pipeline coordinator.
///
/// Pipeline: block event → scanner → simulator → executor → contract
///
/// Architecture:
///   - WebSocket provider subscribes to new block headers (event-driven, <50ms target)
///   - HTTP provider (wallet-backed) used for simulation and execution
///   - Each block triggers: scan → simulate → execute best opportunity
///   - Timing metrics logged for every stage
///   - Telegram alerts fired at key pipeline events (non-blocking)
///   - VelocityEngine records HF per borrower to compute ETA
///
/// Replaces the 400ms polling loop from the legacy Node.js bot.
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use alloy::providers::{Provider, ProviderBuilder, WsConnect};
use eyre::{bail, Result, WrapErr};
use futures_util::StreamExt;
use tokio::sync::Mutex;
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
    scanner,
    simulator,
    velocity::VelocityEngine,
};

/// Sentinel string embedded in circuit-breaker errors so run() can distinguish
/// them from transient per-block errors and propagate immediately.
const CIRCUIT_BREAKER: &str = "CIRCUIT_BREAKER";

/// Top-level engine struct.
pub struct HuntLoanEngine {
    config:   Arc<Config>,
    executor: HuntLoanExecutor,
    velocity: Mutex<VelocityEngine>,
    /// Counts consecutive execution reverts. Reset to 0 on any success.
    consecutive_reverts: AtomicU32,
    /// Counts consecutive RPC-level errors (scan or sim). Reset on any clean block.
    rpc_error_streak:    AtomicU32,
}

impl HuntLoanEngine {
    /// Build the engine from config.
    /// Returns Err if PRIVATE_KEY is invalid.
    pub fn new(config: Config) -> Result<Self> {
        let config = Arc::new(config);
        let executor = HuntLoanExecutor::new(config.clone())?;
        Ok(Self {
            config,
            executor,
            velocity: Mutex::new(VelocityEngine::new()),
            consecutive_reverts: AtomicU32::new(0),
            rpc_error_streak:    AtomicU32::new(0),
        })
    }

    /// Run the engine: connect via WebSocket, subscribe to blocks, process each.
    ///
    /// This function runs indefinitely. Errors from individual blocks are logged
    /// and do not stop the engine.
    pub async fn run(&self) -> Result<()> {
        let ws_url = self.config.rpc_ws.as_deref()
            .ok_or_else(|| eyre::eyre!(
                "WS_RPC_URL is required for event-driven mode. \
                 Set it in .env or fall back to --poll mode."
            ))?;

        info!("[HuntLoanEngine] Connecting WebSocket → {}", ws_url);
        let ws = WsConnect::new(ws_url);
        let ws_provider = ProviderBuilder::new()
            .connect_ws(ws)
            .await
            .wrap_err("WebSocket connection failed")?;
        let ws_provider = Arc::new(ws_provider);

        // ── Load reserve cache once at startup ───────────────────────────────
        info!("[HuntLoanEngine] Loading Aave V3 reserve cache...");
        let reserve_cache = ReserveCache::load(
            ws_provider.as_ref(),
            constants::AAVE_POOL,
            constants::AAVE_DATA,
        )
        .await
        .wrap_err("Failed to load reserve cache")?;

        info!("[HuntLoanEngine] Subscribing to new block headers...");
        let sub = ws_provider
            .subscribe_blocks()
            .await
            .wrap_err("subscribe_blocks failed — provider may not support pubsub")?;
        let mut block_stream = sub.into_stream();

        info!("[HuntLoanEngine] Pipeline active — DRY_RUN={}", self.config.dry_run);

        // Refresh watchlist on first boot so we don't rely solely on a stale file
        if let Err(e) = discovery::refresh_watchlist(&self.config.watchlist_path).await {
            warn!("[HuntLoanEngine] Initial watchlist refresh failed: {}", e);
        }

        let mut last_discovery_block: u64 = 0;
        const DISCOVERY_INTERVAL_BLOCKS: u64 = 300; // ~10 min on Base (2s blocks)

        while let Some(block) = block_stream.next().await {
            let block_start = Instant::now();
            // alloy v1: subscribe_blocks() returns Header { hash, inner, .. }
            // inner is alloy_consensus::Header which holds number + base_fee_per_gas
            let block_num  = block.inner.number;
            // Base L2 base fees are typically 1M–50M wei (0.001–0.05 gwei).
            // Fallback 5_000_000 (0.005 gwei) ensures max_fee always > real base fee
            // if the WS subscription omits base_fee_per_gas.
            let base_fee: u128 = block.inner.base_fee_per_gas
                .unwrap_or(5_000_000_u64)
                .into();
            tracing::debug!(block = block_num, base_fee_wei = base_fee, "New block");

            // Periodic subgraph discovery (non-blocking — runs in background)
            if block_num.saturating_sub(last_discovery_block) >= DISCOVERY_INTERVAL_BLOCKS {
                let path = self.config.watchlist_path.clone();
                tokio::spawn(async move {
                    if let Err(e) = discovery::refresh_watchlist(&path).await {
                        warn!("[discovery] Background refresh failed: {}", e);
                    }
                });
                last_discovery_block = block_num;
            }

            match self
                .process_block(&ws_provider, block_num, base_fee, block_start, &reserve_cache)
                .await
            {
                Ok(()) => {}
                Err(e) => {
                    // Circuit-breaker errors must propagate to stop the engine.
                    // All other per-block errors are logged and skipped.
                    if e.to_string().contains(CIRCUIT_BREAKER) {
                        error!("[CIRCUIT BREAKER] Engine stopping: {}", e);
                        return Err(e);
                    }
                    error!(block = block_num, error = %e, "Block processing error");
                }
            }
        }

        warn!("[HuntLoanEngine] Block stream ended — shutting down");
        Ok(())
    }

    // ── Per-block pipeline ───────────────────────────────────────────────────

    async fn process_block<P: Provider>(
        &self,
        provider: &Arc<P>,
        block_num: u64,
        base_fee_wei: u128,
        block_start: Instant,
        reserve_cache: &ReserveCache,
    ) -> Result<()> {
        let eth_price = oracle::fetch_eth_price_usd(provider.as_ref()).await;

        // ── Stage 1: SCAN ────────────────────────────────────────────────────
        let scan_t = Instant::now();
        let candidates = crate::load_candidates(&self.config.watchlist_path)?;
        let opportunities = match scanner::scan(
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
                // Successful scan clears the RPC error streak
                self.rpc_error_streak.store(0, Ordering::Relaxed);
                opps
            }
            Err(e) => {
                let streak = self.rpc_error_streak.fetch_add(1, Ordering::Relaxed) + 1;
                warn!(block = block_num, streak = streak, error = %e, "Scan RPC error");
                if streak >= self.config.max_rpc_errors {
                    bail!(
                        "{} — {} consecutive RPC errors (last: {})",
                        CIRCUIT_BREAKER, streak, e
                    );
                }
                return Ok(()); // non-fatal — skip this block
            }
        };
        let scan_ms = scan_t.elapsed().as_millis();

        if opportunities.is_empty() {
            return Ok(());
        }

        info!(
            block        = block_num,
            scan_ms      = scan_ms,
            candidates   = candidates.len(),
            opportunities = opportunities.len(),
            "Scan complete"
        );

        // ── Warm-zone scan → velocity engine ────────────────────────────────
        // Run alongside the liquidatable pipeline. Cheap: Multicall3 only, no
        // reserve resolution. Gives the velocity engine observations before a
        // position crosses HF = 1.0 so ETA is meaningful when it matters.
        let warm = scanner::scan_warm(
            provider.as_ref(),
            &self.config,
            &candidates,
        )
        .await;

        // Feed velocity engine from both warm candidates and liquidatable positions
        {
            let mut ve = self.velocity.lock().await;
            for wc in &warm {
                ve.record(wc.borrower, wc.health_factor);
            }
            for opp in &opportunities {
                ve.record(opp.borrower, opp.health_factor);
            }
            ve.maybe_gc();
        }

        // ── Stage 2: SIMULATE ────────────────────────────────────────────────
        let sim_t = Instant::now();
        let mut profitable = Vec::new();

        for opp in &opportunities {
            match simulator::simulate_on_chain(
                provider.as_ref(),
                &self.config,
                opp,
                eth_price,
                base_fee_wei,
            )
            .await
            {
                Ok(sim) if sim.passes => profitable.push((opp.clone(), sim)),
                Ok(sim) => warn!(
                    borrower   = %opp.borrower,
                    reason     = ?sim.revert_reason,
                    net_profit = sim.net_profit_usd,
                    "Simulation skip"
                ),
                Err(e) => warn!(borrower = %opp.borrower, "Simulate error: {}", e),
            }
        }
        let sim_ms = sim_t.elapsed().as_millis();

        if profitable.is_empty() {
            return Ok(());
        }

        // Best = highest net profit
        let (best_opp, best_sim) = profitable
            .into_iter()
            .max_by_key(|(_, s)| s.net_profit_usd)
            .unwrap();

        info!(
            block          = block_num,
            borrower       = %best_opp.borrower,
            hf             = best_opp.health_factor,
            debt_usd       = best_opp.debt_usd,
            net_profit_usd = best_sim.net_profit_usd,
            est_gas        = best_sim.estimated_gas,
            sim_ms         = sim_ms,
            "Best opportunity selected"
        );

        // ── Alert: opportunity locked ────────────────────────────────────────
        {
            let tier = if best_opp.health_factor < 1.002 { "STRIKE" }
                       else if best_opp.health_factor < 1.010 { "CRITICAL" }
                       else { "HOT" };

            let eta_str: Option<String> = {
                let ve = self.velocity.lock().await;
                ve.eta_minutes(&best_opp.borrower)
                    .map(|m| format!("{:.0} min", m.max(0.0)))
            };

            let msg = alerts::fmt_critical(
                &format!("{}", best_opp.borrower),
                best_opp.health_factor,
                best_opp.debt_usd as f64,
                best_sim.net_profit_usd as f64,
                eta_str.as_deref(),
                tier,
            );
            // Throttle to one alert per borrower per 5 min (dedupe_key = addr)
            let dedupe = format!("crit-{}", best_opp.borrower);
            tokio::spawn(async move {
                let _ = alerts::send_telegram(msg, Some(&dedupe), 300, false).await;
            });
        }

        // ── Stage 3: EXECUTE ─────────────────────────────────────────────────
        let exec_t = Instant::now();

        // Compute gas tier + bribe for alert and logging
        let gas_tier_sel = gas::select_tier(best_opp.health_factor, 30.0);
        let gas_tier     = gas::compute_gas_tier(base_fee_wei, 1_000_000_000, gas_tier_sel, gas::Regime::Stable);
        let max_fee_gwei = gas_tier.max_fee_per_gas as f64 / 1_000_000_000.0;
        let max_pri_gwei = gas_tier.max_priority_fee as f64 / 1_000_000_000.0;

        // Gross profit estimate in wei for bribe calculation
        let gross_profit_wei: u128 = if eth_price > 0 {
            (best_sim.net_profit_usd as f64 / eth_price as f64 * 1e18) as u128
        } else { 0 };
        let bribe_wei = gas::compute_bribe_wei(gross_profit_wei, gas_tier.bribe_fraction);
        let bribe_eth = bribe_wei as f64 / 1e18;
        let bribe_usd = bribe_eth * eth_price as f64;

        // Pre-execute strike alert (forced — fire for every execution attempt)
        {
            let msg = alerts::fmt_strike(
                &format!("{}", best_opp.borrower),
                best_opp.health_factor,
                "FLASH → UNISWAP V3",
                bribe_eth,
                bribe_usd,
                max_fee_gwei,
                max_pri_gwei,
            );
            tokio::spawn(async move {
                let _ = alerts::send_telegram(msg, None, 0, true).await;
            });
        }

        if best_sim.net_profit_usd >= PARALLEL_CONVICTION_USD as i128 {
            // High-conviction: dual-shot (STRIKE + KILL) fired in parallel
            let (r1, r2) = self.executor
                .execute_parallel(&best_opp, &best_sim, base_fee_wei)
                .await;

            let exec_ms  = exec_t.elapsed().as_millis();
            let total_ms = block_start.elapsed().as_millis();

            let mut any_confirmed = false;
            for (shot, result) in [("STRIKE", r1), ("KILL", r2)] {
                if let Some(r) = result {
                    any_confirmed = true;
                    info!(
                        shot     = shot,
                        tx_hash  = %r.tx_hash,
                        block    = r.block_number,
                        gas_used = r.gas_used,
                        scan_ms  = scan_ms,
                        sim_ms   = sim_ms,
                        exec_ms  = exec_ms,
                        total_ms = total_ms,
                        "Parallel shot confirmed"
                    );

                    // Profit alert
                    let gas_eth  = r.gas_used as f64 * base_fee_wei as f64 / 1e18;
                    let net_eth  = best_sim.net_profit_usd as f64 / eth_price as f64;
                    let borrower = format!("{}", best_opp.borrower);
                    let tx_hash  = format!("{}", r.tx_hash);
                    let block_no = r.block_number;
                    let net_usd  = best_sim.net_profit_usd as f64;
                    tokio::spawn(async move {
                        let msg = alerts::fmt_profit(net_eth, net_usd, &tx_hash, block_no, &borrower, gas_eth, bribe_eth);
                        let _ = alerts::send_telegram(msg, None, 0, true).await;
                    });
                }
            }
            // Circuit breaker: parallel shots both returned None → treat as revert
            if any_confirmed {
                self.consecutive_reverts.store(0, Ordering::Relaxed);
            } else {
                let n = self.consecutive_reverts.fetch_add(1, Ordering::Relaxed) + 1;
                warn!(count = n, "Parallel dual-shot: both shots returned no receipt");
                if n >= self.config.max_consecutive_reverts {
                    bail!("{} — {} consecutive execution failures", CIRCUIT_BREAKER, n);
                }
            }
        } else {
            // Standard single-shot execution
            match self.executor.execute(&best_opp, &best_sim, base_fee_wei).await {
                Ok(result) => {
                    // Successful confirm — reset revert counter
                    self.consecutive_reverts.store(0, Ordering::Relaxed);
                    let exec_ms  = exec_t.elapsed().as_millis();
                    let total_ms = block_start.elapsed().as_millis();
                    info!(
                        tx_hash      = %result.tx_hash,
                        block        = result.block_number,
                        gas_used     = result.gas_used,
                        scan_ms      = scan_ms,
                        sim_ms       = sim_ms,
                        exec_ms      = exec_ms,
                        total_ms     = total_ms,
                        "Liquidation complete"
                    );

                    // Profit alert
                    let gas_eth  = result.gas_used as f64 * base_fee_wei as f64 / 1e18;
                    let net_eth  = best_sim.net_profit_usd as f64 / eth_price as f64;
                    let borrower = format!("{}", best_opp.borrower);
                    let tx_hash  = format!("{}", result.tx_hash);
                    let block_no = result.block_number;
                    let net_usd  = best_sim.net_profit_usd as f64;
                    tokio::spawn(async move {
                        let msg = alerts::fmt_profit(net_eth, net_usd, &tx_hash, block_no, &borrower, gas_eth, bribe_eth);
                        let _ = alerts::send_telegram(msg, None, 0, true).await;
                    });
                }
                Err(e) => {
                    // Circuit breaker — increment revert counter
                    let n = self.consecutive_reverts.fetch_add(1, Ordering::Relaxed) + 1;
                    error!(
                        borrower = %best_opp.borrower,
                        revert_count = n,
                        max          = self.config.max_consecutive_reverts,
                        error    = %e,
                        "Execution failed"
                    );

                    // Failure alert
                    let reason   = e.to_string();
                    let borrower = format!("{}", best_opp.borrower);
                    tokio::spawn(async move {
                        let msg = alerts::fmt_failed(&reason, &borrower, 1, 3, "retry queued");
                        let _ = alerts::send_telegram(msg, None, 0, false).await;
                    });

                    // Trigger circuit breaker if threshold reached
                    if n >= self.config.max_consecutive_reverts {
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
