/// HuntLoan price oracle — ETH/USD with on-chain primary and REST fallback.
///
/// Priority:
///   1. Chainlink AggregatorV3 on Base (0x71041ddd...) — live, no rate limit
///   2. Binance REST ticker — fallback if chain read fails
///   3. Hardcoded $2000 — last resort, logs a warning
///
/// Returns price as a whole-dollar u128 (e.g. 3_250 = $3,250).
/// Chainlink ETH/USD feed reports 8-decimal answers; we truncate to integer.
use alloy::{
    primitives::{address, Address, I256},
    providers::Provider,
    sol,
};
use eyre::Result;
use tracing::warn;

// ── Chainlink AggregatorV3 interface ────────────────────────────────────────

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    interface IAggregatorV3 {
        function latestRoundData() external view returns (
            uint80  roundId,
            int256  answer,
            uint256 startedAt,
            uint256 updatedAt,
            uint80  answeredInRound
        );
        function decimals() external view returns (uint8);
    }
}

/// Chainlink ETH/USD price feed on Base mainnet.
const CHAINLINK_ETH_USD: Address = address!("71041dddad3595F9CEd3DcCFBe3D1F4b0a16Bb70");

/// Maximum acceptable staleness for Chainlink data (1 hour).
const MAX_STALENESS_SECS: u64 = 3_600;

/// Fetch current ETH price in USD using Chainlink on-chain, with Binance fallback.
///
/// Never returns 0. Falls back gracefully through the priority chain.
pub async fn fetch_eth_price_usd<P: Provider>(provider: &P) -> u128 {
    match chainlink_price(provider).await {
        Ok(p) if p > 0 => return p,
        Ok(_) => warn!("[oracle] Chainlink returned 0 — falling back to Binance"),
        Err(e) => warn!("[oracle] Chainlink read failed: {} — falling back to Binance", e),
    }

    match binance_price().await {
        Ok(p) if p > 0 => return p,
        Ok(_) => warn!("[oracle] Binance returned 0 — using hardcoded fallback"),
        Err(e) => warn!("[oracle] Binance REST failed: {} — using hardcoded fallback", e),
    }

    warn!("[oracle] All price sources failed — using $2,000 hardcoded fallback");
    2_000
}

// ── Chainlink ────────────────────────────────────────────────────────────────

async fn chainlink_price<P: Provider>(provider: &P) -> Result<u128> {
    let feed = IAggregatorV3::new(CHAINLINK_ETH_USD, provider);

    let data = feed.latestRoundData().call().await?;

    // Sanity: answer must be positive and not stale
    if data.answer.is_negative() || data.answer == I256::ZERO {
        return Err(eyre::eyre!("Chainlink answer is non-positive: {}", data.answer));
    }

    // Check staleness
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let updated_at: u64 = data.updatedAt.try_into().unwrap_or(0);
    if now.saturating_sub(updated_at) > MAX_STALENESS_SECS {
        return Err(eyre::eyre!(
            "Chainlink data stale by {}s (max {}s)",
            now.saturating_sub(updated_at),
            MAX_STALENESS_SECS
        ));
    }

    // Chainlink ETH/USD answer has 8 decimals → integer dollars
    // I256 → U256 (safe: we verified positive above) → u128 (price fits easily)
    let price_8dec: u128 = data.answer.unsigned_abs().to::<u128>();
    Ok(price_8dec / 100_000_000)
}

// ── Binance REST ─────────────────────────────────────────────────────────────

async fn binance_price() -> Result<u128> {
    let url = "https://api.binance.com/api/v3/ticker/price?symbol=ETHUSDT";

    let resp = reqwest::get(url).await?.json::<serde_json::Value>().await?;

    let price_str = resp["price"]
        .as_str()
        .ok_or_else(|| eyre::eyre!("Binance response missing 'price' field"))?;

    let price: f64 = price_str.parse()?;

    if price <= 0.0 {
        return Err(eyre::eyre!("Binance returned non-positive price: {}", price));
    }

    Ok(price as u128)
}

#[cfg(test)]
mod tests {
    #[test]
    fn binance_parse_sanity() {
        // Verify the JSON parsing logic with a mock response
        let mock_json = serde_json::json!({"symbol": "ETHUSDT", "price": "3250.55"});
        let price_str = mock_json["price"].as_str().unwrap();
        let price: f64 = price_str.parse().unwrap();
        assert_eq!(price as u128, 3250);
    }
}
