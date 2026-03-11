//! Engine — block subscription loop + pipeline coordinator.
//!
//! Subscribes to new block headers via WebSocket and runs your pipeline
//! on each block. Circuit breaker stops the engine after too many errors.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use alloy::providers::{Provider, ProviderBuilder, WsConnect};
use eyre::{bail, Result, WrapErr};
use futures_util::StreamExt;
use tracing::{error, info, warn};

use crate::{alerts, config::Config, executor::Executor};

const CIRCUIT_BREAKER: &str = "CIRCUIT_BREAKER";

pub struct Engine {
    config:   Arc<Config>,
    executor: Executor,

    /// Counts consecutive execution reverts. Reset to 0 on any success.
    consecutive_reverts: AtomicU64,
    /// Counts consecutive RPC-level errors. Reset on any clean block.
    rpc_error_streak:    AtomicU64,
}

impl Engine {
    pub fn new(config: Config) -> Result<Self> {
        let config = Arc::new(config);
        let executor = Executor::new(config.clone())?;
        Ok(Self {
            config,
            executor,
            consecutive_reverts: AtomicU64::new(0),
            rpc_error_streak:    AtomicU64::new(0),
        })
    }

    pub async fn run(&self) -> Result<()> {
        let ws_url = self.config.rpc_ws.as_deref()
            .ok_or_else(|| eyre::eyre!(
                "WS_RPC_URL is required for event-driven mode. Set it in .env."
            ))?;

        info!("[Engine] Connecting WebSocket -> {}", ws_url);
        let ws = WsConnect::new(ws_url);
        let ws_provider = ProviderBuilder::new()
            .connect_ws(ws)
            .await
            .wrap_err("WebSocket connection failed")?;
        let ws_provider = Arc::new(ws_provider);

        info!("[Engine] Subscribing to new block headers...");
        let sub = ws_provider
            .subscribe_blocks()
            .await
            .wrap_err("subscribe_blocks failed")?;
        let mut block_stream = sub.into_stream();

        info!(
            "[Engine] Pipeline active -- DRY_RUN={}",
            self.config.dry_run,
        );

        // Summary timer
        let mut last_summary = Instant::now();

        let mut shutdown = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to register SIGTERM handler");

        loop {
            let block = tokio::select! {
                b = block_stream.next() => match b {
                    Some(b) => b,
                    None => break,
                },
                _ = tokio::signal::ctrl_c() => {
                    warn!("[Engine] SIGINT received -- graceful shutdown");
                    let msg = alerts::fmt_circuit_breaker("SIGINT", "Operator-initiated graceful shutdown");
                    let _ = alerts::send_telegram(msg, None, 0).await;
                    break;
                }
                _ = shutdown.recv() => {
                    warn!("[Engine] SIGTERM received -- graceful shutdown");
                    let msg = alerts::fmt_circuit_breaker("SIGTERM", "Process manager initiated shutdown");
                    let _ = alerts::send_telegram(msg, None, 0).await;
                    break;
                }
            };

            let block_num = block.inner.number;
            let base_fee: u128 = block.inner.base_fee_per_gas
                .unwrap_or(5_000_000_u64)
                .into();
            tracing::debug!(block = block_num, base_fee_wei = base_fee, "New block");

            alerts::get_stats().blocks_processed.fetch_add(1, Ordering::Relaxed);

            // Periodic summary alert
            if last_summary.elapsed() >= Duration::from_secs(self.config.summary_interval_secs) {
                self.fire_summary_alert().await;
                last_summary = Instant::now();
            }

            // Daily heartbeat
            tokio::spawn(async move { alerts::send_heartbeat(0).await; });

            match self.process_block(block_num, base_fee).await {
                Ok(()) => {}
                Err(e) => {
                    if e.to_string().contains(CIRCUIT_BREAKER) {
                        error!("[CIRCUIT BREAKER] Engine stopping: {}", e);
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

        warn!("[Engine] Block stream ended -- shutting down");
        Ok(())
    }

    async fn fire_summary_alert(&self) {
        let s = alerts::get_stats();
        let uptime  = s.session_start.elapsed().as_secs();
        let blocks  = s.blocks_processed.load(Ordering::Relaxed);
        let tried   = s.execs_attempted.load(Ordering::Relaxed);
        let ok      = s.execs_succeeded.load(Ordering::Relaxed);

        let msg = alerts::fmt_summary(uptime, blocks, tried, ok);
        let interval = self.config.summary_interval_secs;
        let _ = alerts::send_telegram(msg, Some("cat_SUMMARY"), interval).await;
    }

    async fn process_block(
        &self,
        block_num: u64,
        _base_fee_wei: u128,
    ) -> Result<()> {
        // TODO: Implement your contract interaction here
        //
        // This is where you would:
        // 1. Read on-chain state relevant to your contract
        // 2. Decide whether to act
        // 3. Call self.executor.execute(...) to send a transaction
        //
        // Example skeleton:
        //   let data = read_contract_state(&provider, block_num).await?;
        //   if should_act(&data) {
        //       self.executor.send_tx(calldata, base_fee_wei).await?;
        //   }

        tracing::trace!(block = block_num, "Block processed (no-op skeleton)");
        self.rpc_error_streak.store(0, Ordering::Relaxed);
        Ok(())
    }
}
