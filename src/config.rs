use alloy::primitives::Address;
use eyre::{eyre, Result};
use std::str::FromStr;

/// All runtime configuration, loaded once from .env at startup.
#[derive(Debug, Clone)]
pub struct Config {
    /// WebSocket RPC endpoint (Base mainnet)
    pub rpc_ws: String,
    /// HTTP RPC endpoint (fallback / simulation calls)
    pub rpc_http: String,
    /// Private key for the operator wallet (signs + sends txs, pays gas)
    pub operator_key: String,
    /// Deployed Huntloan contract address on Base
    pub huntloan_addr: Address,
    /// Aave V3 Pool address on Base
    pub aave_pool_addr: Address,
    /// Aave V3 PoolAddressesProvider on Base
    pub aave_provider_addr: Address,
    /// Minimum net profit in USD (18-dec) before firing a flash loan
    pub min_profit_usd: u64,
    /// Chain ID (Base = 8453)
    pub chain_id: u64,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        let rpc_ws = var("BASE_RPC_WS")?;
        let rpc_http = var("BASE_RPC_URL")?;
        let operator_key = var("OPERATOR_PRIVATE_KEY")?;
        let huntloan_addr = addr("HUNTLOAN_CONTRACT")?;

        // Aave V3 on Base — hardcoded as public constants, overridable via env
        let aave_pool_addr = addr("AAVE_POOL").unwrap_or_else(|_| {
            Address::from_str("0xA238Dd80C259a72e81d7e4664a9801593F98d1c5").unwrap()
        });
        let aave_provider_addr = addr("AAVE_PROVIDER").unwrap_or_else(|_| {
            Address::from_str("0xe20fCBdBfFC4Dd138cE8b2E6FBb6CB49777ad64D").unwrap()
        });

        let min_profit_usd: u64 = std::env::var("MIN_PROFIT_USD")
            .unwrap_or_else(|_| "50".into())
            .parse()
            .unwrap_or(50);

        Ok(Self {
            rpc_ws,
            rpc_http,
            operator_key,
            huntloan_addr,
            aave_pool_addr,
            aave_provider_addr,
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
