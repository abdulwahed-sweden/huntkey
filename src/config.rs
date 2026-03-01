/// config.rs — Runtime configuration from .env
///
/// Mirrors environment variables used in Bitcoin-Sentinel/eth_forensics/.env

use alloy::primitives::Address;
use eyre::{eyre, Result};
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct Config {
    // RPC
    pub rpc_http:    String,
    pub rpc_ws:      Option<String>,
    pub rpc_public:  String,

    // Identity
    pub operator_key:     String,
    pub huntloan_addr:    Address,
    pub executor_addr:    Address, // legacy flash liquidator (fallback)

    // Telegram
    pub telegram_token:   Option<String>,
    pub telegram_chat_id: Option<String>,

    // Bot behaviour
    pub dry_run:          bool,
    pub min_profit_usd:   f64,
    pub chain_id:         u64,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        let rpc_http   = var("BASE_RPC_URL").unwrap_or_else(|_| "https://mainnet.base.org".into());
        let rpc_ws     = std::env::var("BASE_WS_URL").ok().filter(|s| !s.is_empty());
        let rpc_public = "https://mainnet.base.org".into();

        let operator_key = var("PRIVATE_KEY")?;

        // HUNTLOAN_CONTRACT is the new Huntloan.sol — falls back to legacy EXECUTOR_ADDRESS
        let huntloan_addr = addr("HUNTLOAN_CONTRACT")
            .or_else(|_| addr("EXECUTOR_ADDRESS"))
            .unwrap_or(Address::ZERO);

        let executor_addr = addr("EXECUTOR_ADDRESS").unwrap_or(Address::ZERO);

        let telegram_token   = std::env::var("TELEGRAM_BOT_TOKEN").ok().filter(|s| !s.is_empty());
        let telegram_chat_id = std::env::var("TELEGRAM_CHAT_ID").ok().filter(|s| !s.is_empty());

        let dry_run = std::env::var("DRY_RUN").map(|v| v == "true").unwrap_or(false);

        let min_profit_usd: f64 = std::env::var("MIN_PROFIT_USD")
            .unwrap_or_else(|_| "10".into())
            .parse()
            .unwrap_or(10.0);

        Ok(Self {
            rpc_http,
            rpc_ws,
            rpc_public,
            operator_key,
            huntloan_addr,
            executor_addr,
            telegram_token,
            telegram_chat_id,
            dry_run,
            min_profit_usd,
            chain_id: 8453,
        })
    }
}

fn var(key: &str) -> Result<String> {
    std::env::var(key).map_err(|_| eyre!("Missing env var: {key}"))
}

fn addr(key: &str) -> Result<Address> {
    let raw = var(key)?;
    Address::from_str(&raw).map_err(|e| eyre!("Invalid address for {key}: {e}"))
}
