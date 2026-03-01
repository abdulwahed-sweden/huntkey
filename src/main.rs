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

    info!(
        rpc      = %config.rpc_http,
        ws       = ?config.rpc_ws.as_deref(),
        contract = %config.huntloan_addr,
        dry_run  = config.dry_run,
        "Config loaded"
    );

    if config.huntloan_addr == Address::ZERO {
        tracing::warn!(
            "HUNTLOAN_CONTRACT is not set — simulation will revert. \
             Deploy HuntLoanFlashReceiver.sol and set HUNTLOAN_CONTRACT in .env"
        );
    }

    // ── Boot alert ───────────────────────────────────────────────────────────
    if let (Some(token), Some(chat_id)) = (
        config.telegram_token.as_deref(),
        config.telegram_chat_id.as_deref(),
    ) {
        let msg = format!(
            "<b>[ HUNTLOAN BOT — ONLINE ]</b>\n\
             ─────────────────────────────────\n\
             Mode        {}\n\
             Contract    {}\n\
             Chain       Base (8453)",
            if config.dry_run { "DRY_RUN" } else { "LIVE" },
            config.huntloan_addr,
        );
        alerts::send_telegram_raw(token, chat_id, &msg).await;
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
