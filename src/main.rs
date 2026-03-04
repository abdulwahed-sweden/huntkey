/// HuntLoan — Aave V3 flash loan liquidation bot for Base mainnet.
///
/// Entry point: loads config, sends boot alert, then hands off to
/// HuntLoanEngine which subscribes to new blocks and runs the full pipeline.
///
/// Pipeline: block event → scanner → simulator → executor → contract
mod alerts;
mod config;
mod constants;
mod discovery;
mod engine;
mod executor;
mod gas;
mod math;
mod oracle;
mod reserves;
mod scanner;
mod simulator;
mod trades;
mod velocity;

use alloy::primitives::Address;
use color_eyre::install as install_panic;
use eyre::{Result, WrapErr};
use serde::Deserialize;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::{config::Config, engine::HuntLoanEngine};

#[tokio::main]
async fn main() -> Result<()> {
    // ── Panic handler + logging ──────────────────────────────────────────────
    // Load .env BEFORE the subscriber so RUST_LOG from .env takes effect.
    // Config::from_env() also calls dotenv() — the second call is a no-op.
    let _ = dotenvy::dotenv();
    install_panic()?;
    tracing_subscriber::registry()
        .with(EnvFilter::from_env("RUST_LOG"))
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("╔══════════════════════════════════════════╗");
    info!("║  HuntLoan — Flash Loan Liquidation Bot   ║");
    info!("║  Base Mainnet — Aave V3                  ║");
    info!("╚══════════════════════════════════════════╝");

    // ── Config ───────────────────────────────────────────────────────────────
    let config = Config::from_env().wrap_err("Failed to load config from .env")?;

    let mode = if config.dry_run { "DRY_RUN" }
               else if config.soft_live { "SOFT_LIVE" }
               else { "LIVE" };
    info!(
        rpc      = %config.rpc_http,
        ws       = ?config.rpc_ws.as_deref(),
        contract = %config.huntloan_addr,
        operator = %config.operator_addr,
        mode     = mode,
        "Config loaded"
    );

    // ── Address validation ────────────────────────────────────────────────
    if config.huntloan_addr == Address::ZERO {
        if !config.dry_run {
            return Err(eyre::eyre!(
                "HUNTLOAN_CONTRACT is Address::ZERO — refusing to start in {} mode. \
                 Deploy HuntLoanFlashReceiver.sol and set HUNTLOAN_CONTRACT in .env",
                if config.soft_live { "SOFT_LIVE" } else { "LIVE" }
            ));
        }
        tracing::warn!(
            "HUNTLOAN_CONTRACT is not set — simulations will revert. \
             Deploy HuntLoanFlashReceiver.sol and set HUNTLOAN_CONTRACT in .env"
        );
    }

    if config.huntloan_addr != Address::ZERO
        && config.huntloan_addr != constants::HUNTLOAN_FLASH_RECEIVER
    {
        tracing::warn!(
            config   = %config.huntloan_addr,
            expected = %constants::HUNTLOAN_FLASH_RECEIVER,
            "HUNTLOAN_CONTRACT differs from constants::HUNTLOAN_FLASH_RECEIVER — \
             verify this is intentional (new deployment?)"
        );
    }

    info!(
        huntloan_contract  = %config.huntloan_addr,
        aave_pool          = %config.aave_pool,
        operator           = %config.operator_addr,
        chain_id           = config.chain_id,
        max_bribe_wei      = config.max_bribe_wei,
        max_bribe_fraction = config.max_bribe_fraction,
        min_profit_usd     = config.min_profit_usd,
        min_wallet_eth_wei = config.min_wallet_eth_wei,
        "Active config"
    );

    // ── Boot alert ───────────────────────────────────────────────────────────
    {
        let msg = alerts::fmt_boot(
            mode,
            &format!("{}", config.huntloan_addr),
            &format!("{}", config.operator_addr),
        );
        let _ = alerts::send_telegram(msg, Some("boot"), 0).await;
    }

    // ── Engine ───────────────────────────────────────────────────────────────
    let engine = HuntLoanEngine::new(config)?;
    engine.run().await
}

/// Load candidate borrower addresses from the watchlist JSON.
///
/// Supports two formats:
///   - `["0xabc...", "0xdef...", ...]`               — plain address array
///   - `[{"address": "0xabc..."}, ...]`              — object array with .address
pub fn load_candidates(path: &str) -> Result<Vec<Address>> {
    let raw = std::fs::read_to_string(path)
        .wrap_err_with(|| format!("Cannot read watchlist: {path}"))?;

    // Try plain string array first
    if let Ok(addrs) = serde_json::from_str::<Vec<String>>(&raw) {
        return addrs
            .iter()
            .filter_map(|s| s.parse::<Address>().ok())
            .collect::<Vec<_>>()
            .pipe(Ok);
    }

    // Fall back to object array with optional fields
    #[derive(Deserialize)]
    struct Entry {
        address: String,
    }
    let entries: Vec<Entry> = serde_json::from_str(&raw)
        .wrap_err("Watchlist JSON format not recognised")?;
    let addrs = entries
        .iter()
        .filter_map(|e| e.address.parse::<Address>().ok())
        .collect();
    Ok(addrs)
}

// ── Helper trait for pipe() ──────────────────────────────────────────────────
trait Pipe: Sized {
    fn pipe<T, F: FnOnce(Self) -> T>(self, f: F) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}
