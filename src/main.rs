/// HuntLoan — Base Mainnet Bot Framework
///
/// Entry point: loads config, sends boot alert, then hands off to
/// the engine which subscribes to new blocks and runs the pipeline.
mod alerts;
mod config;
mod constants;
mod engine;
mod executor;
mod gas;

use color_eyre::install as install_panic;
use eyre::{Result, WrapErr};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::{config::Config, engine::Engine};

#[tokio::main]
async fn main() -> Result<()> {
    // ── Panic handler + logging ──────────────────────────────────────────────
    // Load .env BEFORE the subscriber so RUST_LOG from .env takes effect.
    let _ = dotenvy::dotenv();
    install_panic()?;
    tracing_subscriber::registry()
        .with(EnvFilter::from_env("RUST_LOG"))
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("========================================");
    info!("  HuntLoan — Base Mainnet Bot Framework ");
    info!("========================================");

    // ── Config ───────────────────────────────────────────────────────────────
    let config = Config::from_env().wrap_err("Failed to load config from .env")?;

    let mode = if config.dry_run { "DRY_RUN" } else { "LIVE" };
    info!(
        rpc      = %config.rpc_http,
        ws       = ?config.rpc_ws.as_deref(),
        contract = %config.contract_addr,
        operator = %config.operator_addr,
        mode     = mode,
        "Config loaded"
    );

    // ── Boot alert ───────────────────────────────────────────────────────────
    {
        let msg = alerts::fmt_boot(
            mode,
            &format!("{}", config.contract_addr),
            &format!("{}", config.operator_addr),
        );
        let _ = alerts::send_telegram(msg, Some("boot"), 0).await;
    }

    // ── Engine ───────────────────────────────────────────────────────────────
    let engine = Engine::new(config)?;
    engine.run().await
}
