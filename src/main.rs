mod alerts;
mod config;
mod constants;
mod executor;
mod gas;
mod math;
mod scanner;

use config::Config;
use eyre::Result;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let cfg = Config::from_env()?;

    info!("Huntloan MEV bot — chain {} | dry_run={}", cfg.chain_id, cfg.dry_run);
    info!("Contract:   {}", cfg.huntloan_addr);
    info!("Executor:   {}", cfg.executor_addr);
    info!("Min profit: ${}", cfg.min_profit_usd);

    if cfg.dry_run {
        warn!("DRY_RUN=true — transactions will be simulated but NOT broadcast");
    }

    // Send startup notice to Telegram
    let boot_msg = format!(
        "<b>[ HUNTLOAN BOT ONLINE ]</b>\n{}\n\
         {}  Base (chain 8453)\n\
         {}  {}\n\
         {}  ${}\n\
         {}  {}",
        "─────────────────────────────────",
        format!("{:<8}", "Chain"),
        format!("{:<8}", "Contract"), cfg.huntloan_addr,
        format!("{:<8}", "Min $"), cfg.min_profit_usd,
        format!("{:<8}", "Mode"), if cfg.dry_run { "DRY RUN" } else { "LIVE" },
    );
    alerts::send_telegram(boot_msg, Some("boot"), 600, true).await?;

    // ── Main event loop ──────────────────────────────────────────────────────
    // TODO: subscribe to new blocks via WS for zero-latency CRITICAL tier.
    // Current: HTTP poll every 400ms — matches CONFIG.POLL_INTERVAL in monitor_base.js.
    let mut cycle: u64 = 0;
    loop {
        cycle += 1;

        let candidates  = load_candidates(&cfg);
        let eth_price   = fetch_eth_price_usd().await;
        let gas_gwei    = 5_000_000u128; // 0.005 gwei — Base L2 typical

        match scanner::find_opportunities(&cfg, &candidates, eth_price, gas_gwei).await {
            Err(e) => {
                error!("Scanner error (cycle {cycle}): {e}");
            }
            Ok(opps) if opps.is_empty() => {
                if cycle % 25 == 0 {
                    info!("Cycle {cycle}: no profitable opportunities");
                }
            }
            Ok(opps) => {
                let best = &opps[0];
                info!(
                    cycle    = cycle,
                    borrower = %best.borrower,
                    hf       = best.health_factor,
                    profit   = best.estimated_profit_usd,
                    "Opportunity — executing"
                );

                // Telegram alert
                let msg = alerts::fmt_critical(
                    &best.borrower.to_string(),
                    best.health_factor,
                    best.debt_usd as f64,
                    best.estimated_profit_usd as f64,
                    None,
                    "CRITICAL",
                );
                alerts::send_telegram(msg, Some(&best.borrower.to_string()), 600, false).await.ok();

                if !cfg.dry_run {
                    if let Err(e) = executor::execute(&cfg, best).await {
                        error!("Executor error: {e}");
                    }
                } else {
                    info!("DRY_RUN: would execute liquidation for {}", best.borrower);
                }
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(400)).await;
    }
}

/// Load borrower candidates from watchlist.json (mirroring WATCHLIST_PATH in monitor_base.js).
/// Falls back to empty if file is missing.
fn load_candidates(_cfg: &Config) -> Vec<alloy::primitives::Address> {
    let path = std::env::var("WATCHLIST_PATH")
        .unwrap_or_else(|_| "../watchlist.json".into());

    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let entries: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap_or_default();
    entries
        .into_iter()
        .filter_map(|e| {
            let addr_str = if e.is_string() {
                e.as_str().unwrap_or("").to_string()
            } else {
                e.get("address")?.as_str()?.to_string()
            };
            addr_str.parse().ok()
        })
        .collect()
}

/// Fetch current ETH price in USD (stub — replace with Chainlink or Binance REST).
async fn fetch_eth_price_usd() -> u128 {
    // TODO: call https://api.binance.com/api/v3/ticker/price?symbol=ETHUSDT
    2_000
}
