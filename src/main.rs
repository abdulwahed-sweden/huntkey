mod config;
mod executor;
mod math;
mod scanner;

use config::Config;
use eyre::Result;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // Structured logging — set RUST_LOG=huntloan=info in .env
    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let cfg = Config::from_env()?;
    info!("Huntloan bot starting — chain {}", cfg.chain_id);
    info!("Contract: {}", cfg.huntloan_addr);
    info!("Min profit: {} USD", cfg.min_profit_usd);

    // ── Main loop ──────────────────────────────────────────────────────────────
    // In production this subscribes to new blocks via WebSocket and feeds
    // updated candidate lists from an on-chain event index or subgraph.
    // For the boilerplate we use a static list to validate the full pipeline.
    loop {
        let candidates = load_candidates();
        let eth_price  = fetch_eth_price_usd().await;
        let gas_gwei   = 5_000_000u128; // 0.005 gwei — Base L2 typical

        match scanner::find_opportunities(&cfg, &candidates, eth_price, gas_gwei).await {
            Err(e) => error!("Scanner error: {e}"),
            Ok(opps) if opps.is_empty() => {
                info!("No profitable opportunities this cycle");
            }
            Ok(opps) => {
                let best = &opps[0];
                info!(
                    borrower = %best.borrower,
                    hf       = best.health_factor,
                    profit   = best.estimated_profit_usd,
                    "Best opportunity — executing"
                );
                if let Err(e) = executor::execute(&cfg, best).await {
                    error!("Executor error: {e}");
                }
            }
        }

        // Poll every 2 seconds — replace with block subscription for zero-latency
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }
}

/// Load borrower candidates — plug in subgraph / on-chain event feed here.
fn load_candidates() -> Vec<alloy::primitives::Address> {
    // TODO: load from watchlist.json or subscribe to Borrow events
    vec![]
}

/// Fetch current ETH price in USD.
/// Stub — replace with Chainlink on-chain oracle or Binance REST call.
async fn fetch_eth_price_usd() -> u128 {
    2_000 // placeholder — $2000
}
