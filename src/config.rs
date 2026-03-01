/// HuntLoan runtime configuration — loaded once at startup from .env
///
/// Variable naming convention (normalized from legacy Bitcoin-Sentinel names):
///   RPC_URL           (was: BASE_RPC_URL)
///   WS_RPC_URL        (was: BASE_WS_URL)
///   HUNTLOAN_CONTRACT (unchanged)
///   TELEGRAM_BOT_TOKEN / TELEGRAM_CHAT_ID (unchanged)
use alloy::primitives::{address, Address};
use eyre::{Result, WrapErr};

#[derive(Debug, Clone)]
pub struct Config {
    // [NETWORK]
    pub rpc_http:         String,          // RPC_URL
    pub rpc_ws:           Option<String>,  // WS_RPC_URL — required for WS scanner
    /// Private RPC for tx submission — when set, used instead of rpc_http for
    /// broadcast calls. Provides Base MEV protection (no public mempool exposure).
    pub private_rpc_http: Option<String>,  // PRIVATE_RPC_URL

    // [WALLET]
    pub operator_key: String,         // PRIVATE_KEY

    // [CONTRACTS]
    pub huntloan_addr: Address,       // HUNTLOAN_CONTRACT
    pub aave_pool:     Address,       // AAVE_POOL (defaults to Base mainnet)

    // [TELEGRAM]
    pub telegram_token:   Option<String>, // TELEGRAM_BOT_TOKEN
    pub telegram_chat_id: Option<String>, // TELEGRAM_CHAT_ID

    // [BOT SETTINGS]
    pub watchlist_path:   String,     // WATCHLIST_PATH
    pub dry_run:          bool,       // DRY_RUN
    pub min_profit_usd:   f64,        // MIN_PROFIT_USD
    pub max_gas_cost_wei: u128,       // MAX_GAS_COST_WEI
    pub max_bribe_wei:    u128,       // MAX_BRIBE_WEI

    // [CHAIN]
    pub chain_id: u64,                // 8453 (Base mainnet, hardcoded)
}

impl Config {
    /// Load configuration from environment variables.
    /// Calls `dotenvy::dotenv()` to load .env if present.
    pub fn from_env() -> Result<Self> {
        // Best-effort .env load — ignore error if file is absent
        let _ = dotenvy::dotenv();

        let rpc_http = std::env::var("RPC_URL")
            .or_else(|_| std::env::var("BASE_RPC_URL")) // legacy fallback
            .wrap_err("RPC_URL is required")?;

        let rpc_ws = match std::env::var("WS_RPC_URL")
            .or_else(|_| std::env::var("BASE_WS_URL"))
        {
            Ok(v) if !v.is_empty() => Some(v),
            _ => None,
        };

        let private_rpc_http = optional_env("PRIVATE_RPC_URL");

        let operator_key = std::env::var("PRIVATE_KEY")
            .wrap_err("PRIVATE_KEY is required")?;

        let huntloan_addr = parse_addr_env("HUNTLOAN_CONTRACT")
            .or_else(|_| parse_addr_env("EXECUTOR_ADDRESS")) // legacy fallback
            .unwrap_or(Address::ZERO);

        let aave_pool = parse_addr_env("AAVE_POOL")
            .unwrap_or(address!("A238Dd80C259a72e81d7e4664a9801593F98d1c5"));

        let telegram_token = optional_env("TELEGRAM_BOT_TOKEN");
        let telegram_chat_id = optional_env("TELEGRAM_CHAT_ID");

        let watchlist_path = std::env::var("WATCHLIST_PATH")
            .unwrap_or_else(|_| "watchlist.json".to_string());

        let dry_run = std::env::var("DRY_RUN")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

        let min_profit_usd = std::env::var("MIN_PROFIT_USD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10.0_f64);

        let max_gas_cost_wei = std::env::var("MAX_GAS_COST_WEI")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8_000_000_000_000_000_u128); // 0.008 ETH

        let max_bribe_wei = std::env::var("MAX_BRIBE_WEI")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50_000_000_000_000_000_u128); // 0.05 ETH

        Ok(Self {
            rpc_http,
            rpc_ws,
            private_rpc_http,
            operator_key,
            huntloan_addr,
            aave_pool,
            telegram_token,
            telegram_chat_id,
            watchlist_path,
            dry_run,
            min_profit_usd,
            max_gas_cost_wei,
            max_bribe_wei,
            chain_id: 8453,
        })
    }
}

fn parse_addr_env(key: &str) -> Result<Address> {
    let raw = std::env::var(key).wrap_err(format!("{key} not set"))?;
    raw.parse::<Address>().wrap_err(format!("{key} is not a valid address"))
}

fn optional_env(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}
