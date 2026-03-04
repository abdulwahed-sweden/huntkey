/// HuntLoan runtime configuration — loaded once at startup from .env
///
/// Variable naming convention (normalized from legacy Bitcoin-Sentinel names):
///   RPC_URL           (was: BASE_RPC_URL)
///   WS_RPC_URL        (was: BASE_WS_URL)
///   HUNTLOAN_CONTRACT (unchanged)
///   TELEGRAM_BOT_TOKEN / TELEGRAM_CHAT_ID (unchanged)
use alloy::primitives::{address, Address};
use alloy::signers::local::PrivateKeySigner;
use eyre::{Result, WrapErr};

use crate::constants;

#[derive(Debug, Clone)]
pub struct Config {
    // [NETWORK]
    pub rpc_http:         String,          // RPC_URL
    pub rpc_ws:           Option<String>,  // WS_RPC_URL — required for WS scanner
    /// Private RPC for tx submission — when set, used instead of rpc_http for
    /// broadcast calls. Provides Base MEV protection (no public mempool exposure).
    pub private_rpc_http: Option<String>,  // PRIVATE_RPC_URL

    // [WALLET]
    pub operator_key:  String,        // PRIVATE_KEY
    pub operator_addr: Address,       // derived from PRIVATE_KEY at startup

    // [CONTRACTS]
    pub huntloan_addr: Address,       // HUNTLOAN_CONTRACT
    pub aave_pool:     Address,       // AAVE_POOL (defaults to Base mainnet)

    // [TELEGRAM]
    #[allow(dead_code)]
    pub telegram_token:   Option<String>, // TELEGRAM_BOT_TOKEN
    #[allow(dead_code)]
    pub telegram_chat_id: Option<String>, // TELEGRAM_CHAT_ID

    // [BOT SETTINGS]
    pub watchlist_path:   String,     // WATCHLIST_PATH
    pub dry_run:          bool,       // DRY_RUN — default TRUE (fail-safe)
    /// Sign + print full tx preview, do NOT broadcast. Requires DRY_RUN=false.
    pub soft_live:        bool,       // SOFT_LIVE — preview mode
    pub min_profit_usd:   f64,        // MIN_PROFIT_USD
    #[allow(dead_code)] // wired into executor gas-cost cap in a later commit
    pub max_gas_cost_wei: u128,       // MAX_GAS_COST_WEI
    pub max_bribe_wei:    u128,       // MAX_BRIBE_WEI
    /// Maximum fraction of gross profit payable as priority fee bribe.
    /// Default: 0.90 (90%). Set MAX_BRIBE_FRACTION=0.75 to be more conservative.
    pub max_bribe_fraction: f64,      // MAX_BRIBE_FRACTION

    // [CIRCUIT BREAKER]
    /// Engine stops after this many consecutive execution reverts. Default: 3.
    pub max_consecutive_reverts: u32, // MAX_CONSECUTIVE_REVERTS
    /// Engine stops after this many consecutive RPC-level errors (scan/sim). Default: 10.
    pub max_rpc_errors: u32,          // MAX_RPC_ERRORS

    // [ALERTS]
    /// Minimum seconds between same-category Telegram alerts. Default: 60.
    pub alert_rate_limit_secs: u64,   // ALERT_RATE_LIMIT_SECONDS
    /// Seconds between hourly/daily summary alerts. Default: 3600.
    pub summary_interval_secs: u64,   // SUMMARY_INTERVAL_SECONDS

    // [EXECUTION FILTERS]
    /// Only execute if HF <= this value (1.0 = execute all, 0.85 = strict). Default: 1.0.
    pub strong_hf_threshold: f64,     // STRONG_HF_THRESHOLD
    /// Minimum profit margin in basis points (profit/debt × 10000). Default: 50.
    pub min_margin_bps: u64,          // MIN_MARGIN_BPS
    /// Hard daily gas spend cap in wei (u128::MAX = disabled). Default: disabled.
    pub max_daily_gas_wei: u128,      // MAX_DAILY_GAS_WEI
    /// Hard daily bribe spend cap in wei (u128::MAX = disabled). Default: disabled.
    pub max_daily_bribe_wei: u128,    // MAX_DAILY_BRIBE_WEI

    // [SPEED]
    /// Maximum concurrent on-chain simulation calls (JoinSet concurrency). Default: 4.
    pub max_parallel_sims: usize,     // MAX_PARALLEL_SIMS
    /// Seconds a failed target stays blacklisted before retry. Default: 300.
    pub target_cooldown_secs: u64,    // TARGET_COOLDOWN_SECONDS

    // [MICRO-BANKROLL SURVIVAL]
    /// Reduce expected gross profit by this many bps before the post-sim profitability check.
    /// Accounts for price drift between eth_call simulation and actual broadcast (~200-400ms).
    /// 0 = disabled (default). 2500 = 25% haircut (recommended for small bankroll).
    pub slippage_buffer_bps: u64,     // SLIPPAGE_BUFFER_BPS
    /// Refuse broadcast if wallet ETH balance (wei) is below this floor.
    /// Default: 5_000_000_000_000_000 (0.005 ETH). Engine logs warning and skips execution.
    pub min_wallet_eth_wei: u128,     // MIN_WALLET_ETH_WEI

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

        let operator_addr = operator_key
            .parse::<PrivateKeySigner>()
            .wrap_err("PRIVATE_KEY is not a valid hex private key")?
            .address();

        let huntloan_addr = parse_addr_env("HUNTLOAN_CONTRACT")
            .or_else(|_| parse_addr_env("EXECUTOR_ADDRESS")) // legacy fallback
            .unwrap_or(Address::ZERO);

        let aave_pool = parse_addr_env("AAVE_POOL")
            .unwrap_or(address!("A238Dd80C259a72e81d7e4664a9801593F98d1c5"));

        let telegram_token = optional_env("TELEGRAM_BOT_TOKEN");
        let telegram_chat_id = optional_env("TELEGRAM_CHAT_ID");

        let watchlist_path = std::env::var("WATCHLIST_PATH")
            .unwrap_or_else(|_| "watchlist.json".to_string());

        // Safety default: true. Must explicitly set DRY_RUN=false to go live.
        let dry_run = std::env::var("DRY_RUN")
            .map(|v| !(v.eq_ignore_ascii_case("false") || v == "0"))
            .unwrap_or(true);

        // SOFT_LIVE: sign tx and print full preview, but do NOT broadcast.
        // Only active when DRY_RUN=false. Use SOFT_LIVE=true as an intermediate step.
        let soft_live = std::env::var("SOFT_LIVE")
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
            .unwrap_or(500_000_000_000_000_000_u128); // 0.5 ETH operational default

        let max_bribe_fraction = std::env::var("MAX_BRIBE_FRACTION")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(constants::DEFAULT_MAX_BRIBE_FRACTION); // 0.90

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

        let strong_hf_threshold = std::env::var("STRONG_HF_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0_f64); // 1.0 = disabled (execute all < 1.0)

        let min_margin_bps = std::env::var("MIN_MARGIN_BPS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50_u64); // 50 bps = 0.5% minimum margin

        let max_daily_gas_wei = std::env::var("MAX_DAILY_GAS_WEI")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(u128::MAX); // disabled by default

        let max_daily_bribe_wei = std::env::var("MAX_DAILY_BRIBE_WEI")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(u128::MAX); // disabled by default

        let max_parallel_sims = std::env::var("MAX_PARALLEL_SIMS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4_usize);

        let target_cooldown_secs = std::env::var("TARGET_COOLDOWN_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300_u64);

        let slippage_buffer_bps = std::env::var("SLIPPAGE_BUFFER_BPS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0_u64); // 0 = disabled; 2500 = 25% haircut recommended

        let min_wallet_eth_wei = std::env::var("MIN_WALLET_ETH_WEI")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5_000_000_000_000_000_u128); // 0.005 ETH floor

        Ok(Self {
            rpc_http,
            rpc_ws,
            private_rpc_http,
            operator_key,
            operator_addr,
            huntloan_addr,
            aave_pool,
            telegram_token,
            telegram_chat_id,
            watchlist_path,
            dry_run,
            soft_live,
            min_profit_usd,
            max_gas_cost_wei,
            max_bribe_wei,
            max_bribe_fraction,
            max_consecutive_reverts,
            max_rpc_errors,
            alert_rate_limit_secs,
            summary_interval_secs,
            strong_hf_threshold,
            min_margin_bps,
            max_daily_gas_wei,
            max_daily_bribe_wei,
            max_parallel_sims,
            target_cooldown_secs,
            slippage_buffer_bps,
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
