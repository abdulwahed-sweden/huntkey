/// HuntLoanEngine — full pipeline coordinator.
///
/// Pipeline: block event → scanner → simulator → executor → contract
///
/// Architecture:
///   - WebSocket provider subscribes to new block headers (event-driven, <50ms target)
///   - HTTP provider (wallet-backed) used for simulation and execution
///   - Each block triggers: scan → simulate → execute best opportunity
///   - Timing metrics logged for every stage
///
/// Replaces the 400ms polling loop from the legacy Node.js bot.
use std::sync::Arc;
use std::time::Instant;

use alloy::providers::{Provider, ProviderBuilder, WsConnect};
use eyre::{Result, WrapErr};
use futures_util::StreamExt;
use tracing::{error, info, warn};

use crate::{
    config::Config,
    executor::HuntLoanExecutor,
    scanner,
    simulator,
};

/// Top-level engine struct.
pub struct HuntLoanEngine {
    config:   Arc<Config>,
    executor: HuntLoanExecutor,
}

impl HuntLoanEngine {
    /// Build the engine from config.
    /// Returns Err if PRIVATE_KEY is invalid.
    pub fn new(config: Config) -> Result<Self> {
        let config = Arc::new(config);
        let executor = HuntLoanExecutor::new(config.clone())?;
        Ok(Self { config, executor })
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

        info!("[HuntLoanEngine] Subscribing to new block headers...");
        let sub = ws_provider
            .subscribe_blocks()
            .await
            .wrap_err("subscribe_blocks failed — provider may not support pubsub")?;
        let mut block_stream = sub.into_stream();

        info!("[HuntLoanEngine] Pipeline active — DRY_RUN={}", self.config.dry_run);

        while let Some(block) = block_stream.next().await {
            let block_start = Instant::now();
            // alloy v1: subscribe_blocks() returns Header { hash, inner, .. }
            // inner is alloy_consensus::Header which holds number + base_fee_per_gas
            let block_num   = block.inner.number;
            let base_fee: u128 = block.inner.base_fee_per_gas
                .unwrap_or(1_000_000_u64)
                .into();

            if let Err(e) = self
                .process_block(&ws_provider, block_num, base_fee, block_start)
                .await
            {
                error!(block = block_num, error = %e, "Block processing error");
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
    ) -> Result<()> {
        let eth_price = fetch_eth_price_usd();

        // ── Stage 1: SCAN ────────────────────────────────────────────────────
        let scan_t = Instant::now();
        let candidates = crate::load_candidates(&self.config.watchlist_path)?;
        let opportunities = scanner::scan(
            provider.as_ref(),
            &self.config,
            &candidates,
            eth_price,
            base_fee_wei,
        )
        .await?;
        let scan_ms = scan_t.elapsed().as_millis();

        if opportunities.is_empty() {
            return Ok(());
        }

        info!(
            block = block_num,
            scan_ms = scan_ms,
            candidates = candidates.len(),
            opportunities = opportunities.len(),
            "Scan complete"
        );

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
                    borrower = %opp.borrower,
                    reason = ?sim.revert_reason,
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
            block = block_num,
            borrower = %best_opp.borrower,
            hf = best_opp.health_factor,
            debt_usd = best_opp.debt_usd,
            net_profit_usd = best_sim.net_profit_usd,
            est_gas = best_sim.estimated_gas,
            sim_ms = sim_ms,
            "Best opportunity selected"
        );

        // ── Stage 3: EXECUTE ─────────────────────────────────────────────────
        let exec_t = Instant::now();
        match self.executor.execute(&best_opp, &best_sim, base_fee_wei).await {
            Ok(result) => {
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
            }
            Err(e) => {
                error!(
                    borrower = %best_opp.borrower,
                    error    = %e,
                    "Execution failed"
                );
            }
        }

        Ok(())
    }
}

/// ETH price oracle stub.
/// TODO: replace with Chainlink on-chain read or Binance REST.
pub fn fetch_eth_price_usd() -> u128 {
    2_000
}
