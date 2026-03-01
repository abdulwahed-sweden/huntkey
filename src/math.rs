/// Profitability math for flash loan liquidations.
///
/// Aave V3 flash loan fee: 0.05% (5 bps) of the borrowed amount.
/// Gas cost on Base L2: estimated at 800K gas × current base fee.
/// Net profit = liquidation bonus - flash fee - gas cost.

/// Aave V3 flash loan fee numerator (0.05% = 5 / 10_000)
const FLASH_FEE_BPS: u128 = 5;
const BPS_DENOM: u128 = 10_000;

/// Liquidation bonus on Base Aave V3 (typically 5% for WETH, 10% for volatile)
/// Expressed in basis points. Retrieved on-chain; 500 = 5%.
pub struct SimResult {
    pub gross_usd: u128,       // collateral seized value in USD (6-dec, USDC-normalised)
    pub flash_fee_usd: u128,   // 0.05% of borrowed amount
    pub gas_cost_usd: u128,    // estimated gas in USD
    pub net_profit_usd: i128,  // gross - flash_fee - gas (can be negative)
    pub profitable: bool,
}

/// Estimate net profit from a flash-loan liquidation.
///
/// # Arguments
/// * `debt_to_repay_usd`   — USD value of debt being repaid (6-dec)
/// * `collateral_usd`      — USD value of collateral to be seized (6-dec)
/// * `liquidation_bonus`   — bonus in BPS (e.g. 500 = 5%)
/// * `gas_price_gwei`      — current base fee in gwei
/// * `eth_price_usd`       — current ETH price in USD (no decimals)
pub fn simulate(
    debt_to_repay_usd: u128,
    collateral_usd: u128,
    liquidation_bonus_bps: u128,
    gas_price_gwei: u128,
    eth_price_usd: u128,
) -> SimResult {
    // Gross: collateral seized at bonus above repaid debt
    let bonus_usd = debt_to_repay_usd * liquidation_bonus_bps / BPS_DENOM;
    let gross_usd = bonus_usd;

    // Flash loan fee (0.05% of amount borrowed)
    let flash_fee_usd = debt_to_repay_usd * FLASH_FEE_BPS / BPS_DENOM;

    // Gas cost: ~800K gas on Base L2
    // gas_price_gwei is passed in wei (e.g. 5_000_000 = 0.005 gwei = 5M wei)
    let gas_units: u128 = 800_000;
    let gas_eth_wei: u128 = gas_units * gas_price_gwei; // already in wei
    let gas_cost_usd: u128 = gas_eth_wei * eth_price_usd / 1_000_000_000_000_000_000u128;

    let net_profit_usd = gross_usd as i128 - flash_fee_usd as i128 - gas_cost_usd as i128;

    SimResult {
        gross_usd,
        flash_fee_usd,
        gas_cost_usd,
        net_profit_usd,
        profitable: net_profit_usd > 0,
    }
}

/// Distribute net profits per the investment agreement:
///   - Saeed (financier): 60% of net profit + full capital recovery
///   - Omar  (operator):  40% of net profit (zero if net profit is negative)
///
/// Returns (saeed_share, omar_share) in the same unit as inputs.
pub fn distribute(capital: u128, final_balance: u128) -> (u128, u128) {
    if final_balance <= capital {
        // Loss — financier takes whatever is left, operator gets nothing
        return (final_balance, 0);
    }
    let net_profit = final_balance - capital;
    let saeed = capital + net_profit * 60 / 100;
    let omar  = net_profit * 40 / 100;
    (saeed, omar)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_distribute_profit() {
        // 10K capital, 15K final → profit 5K → saeed 13K, omar 2K
        let (s, o) = distribute(10_000, 15_000);
        assert_eq!(s, 13_000);
        assert_eq!(o, 2_000);
    }

    #[test]
    fn test_distribute_loss() {
        // 10K capital, 8K final → loss → saeed 8K, omar 0
        let (s, o) = distribute(10_000, 8_000);
        assert_eq!(s, 8_000);
        assert_eq!(o, 0);
    }

    #[test]
    fn test_simulate_profitable() {
        // Repay 10K USDC, 5% bonus, gas 0.005 gwei, ETH $2000
        let r = simulate(10_000, 10_500, 500, 5_000_000, 2_000);
        assert!(r.profitable);
    }
}
