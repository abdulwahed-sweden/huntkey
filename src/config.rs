/// Runtime configuration — loaded once at startup from .env
use alloy::primitives::Address;
use alloy::signers::local::PrivateKeySigner;
use eyre::{Result, WrapErr};

#[derive(Debug, Clone)]
pub struct Config {
    // [NETWORK]
    pub rpc_http:         String,          // RPC_URL
    pub rpc_ws:           Option<String>,  // WS_RPC_URL
    pub private_rpc_http: Option<String>,  // PRIVATE_RPC_URL

    // [WALLET]
    pub operator_key:  String,        // PRIVATE_KEY
    pub operator_addr: Address,       // derived from PRIVATE_KEY at startup

    // [CONTRACT]
    pub contract_addr: Address,       // CONTRACT_ADDRESS

    // [TELEGRAM]
    #[allow(dead_code)]
    pub telegram_token:   Option<String>,
    #[allow(dead_code)]
    pub telegram_chat_id: Option<String>,

    // [BOT SETTINGS]
    pub dry_run:          bool,       // DRY_RUN — default TRUE (fail-safe)
    pub max_gas_cost_wei: u128,       // MAX_GAS_COST_WEI
    pub max_bribe_wei:    u128,       // MAX_BRIBE_WEI
    pub max_bribe_fraction: f64,      // MAX_BRIBE_FRACTION

    // [CIRCUIT BREAKER]
    pub max_consecutive_reverts: u32, // MAX_CONSECUTIVE_REVERTS
    pub max_rpc_errors: u32,         // MAX_RPC_ERRORS

    // [ALERTS]
    pub alert_rate_limit_secs: u64,  // ALERT_RATE_LIMIT_SECONDS
    pub summary_interval_secs: u64,  // SUMMARY_INTERVAL_SECONDS

    // [SAFETY]
    pub min_wallet_eth_wei: u128,    // MIN_WALLET_ETH_WEI

    // [CHAIN]
    pub chain_id: u64,               // 8453 (Base mainnet)
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let _ = dotenvy::dotenv();

        let rpc_http = std::env::var("RPC_URL")
            .wrap_err("RPC_URL is required")?;

        let rpc_ws = match std::env::var("WS_RPC_URL") {
            Ok(v) if !v.is_empty() => Some(v),
            _ => None,
        };

        let private_rpc_http = optional_env("PRIVATE_RPC_URL");

        let operator_key = std::env::var("PRIVATE_KEY")
            .wrap_err("PRIVATE_KEY is required")?;

        let operator_addr = operator_key
            .parse::<PrivateKeySigner>()
            .wrap_err("PRIVATE_KEY is not a valid hex private key")?
            .address();

        let contract_addr = parse_addr_env("CONTRACT_ADDRESS")
            .unwrap_or(Address::ZERO);

        let telegram_token = optional_env("TELEGRAM_BOT_TOKEN");
        let telegram_chat_id = optional_env("TELEGRAM_CHAT_ID");

        let dry_run = std::env::var("DRY_RUN")
            .map(|v| !(v.eq_ignore_ascii_case("false") || v == "0"))
            .unwrap_or(true);

        let max_gas_cost_wei = std::env::var("MAX_GAS_COST_WEI")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8_000_000_000_000_000_u128); // 0.008 ETH

        let max_bribe_wei = std::env::var("MAX_BRIBE_WEI")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(500_000_000_000_000_000_u128); // 0.5 ETH

        let max_bribe_fraction = std::env::var("MAX_BRIBE_FRACTION")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(crate::constants::DEFAULT_MAX_BRIBE_FRACTION);

        let max_consecutive_reverts = std::env::var("MAX_CONSECUTIVE_REVERTS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3_u32);

        let max_rpc_errors = std::env::var("MAX_RPC_ERRORS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10_u32);

        let alert_rate_limit_secs = std::env::var("ALERT_RATE_LIMIT_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60_u64);

        let summary_interval_secs = std::env::var("SUMMARY_INTERVAL_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600_u64);

        let min_wallet_eth_wei = std::env::var("MIN_WALLET_ETH_WEI")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5_000_000_000_000_000_u128); // 0.005 ETH

        Ok(Self {
            rpc_http,
            rpc_ws,
            private_rpc_http,
            operator_key,
            operator_addr,
            contract_addr,
            telegram_token,
            telegram_chat_id,
            dry_run,
            max_gas_cost_wei,
            max_bribe_wei,
            max_bribe_fraction,
            max_consecutive_reverts,
            max_rpc_errors,
            alert_rate_limit_secs,
            summary_interval_secs,
            min_wallet_eth_wei,
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
