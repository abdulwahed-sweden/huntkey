use alloy::{
    primitives::Address,
    providers::ProviderBuilder,
    sol,
};
use eyre::Result;
use tracing::{info, warn};

use crate::{config::Config, math};

/// Aave V3 Pool interface — only the calls we need
sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    interface IAavePool {
        function getUserAccountData(address user)
            external view returns (
                uint256 totalCollateralBase,
                uint256 totalDebtBase,
                uint256 availableBorrowsBase,
                uint256 currentLiquidationThreshold,
                uint256 ltv,
                uint256 healthFactor
            );
    }
}

/// A liquidation opportunity ready to be executed
#[derive(Debug, Clone)]
pub struct Opportunity {
    pub borrower:              Address,
    pub health_factor:         f64,
    pub debt_usd:              u128,
    pub collateral_usd:        u128,
    pub collateral_asset:      Address,
    pub debt_asset:            Address,
    pub debt_to_repay:         u128,
    pub estimated_profit_usd:  i128,
}

/// Scan a list of borrower addresses, return those that are liquidatable
/// and pass the profitability check.
pub async fn find_opportunities(
    cfg: &Config,
    candidates: &[Address],
    eth_price_usd: u128,
    gas_gwei: u128,
) -> Result<Vec<Opportunity>> {
    let provider = ProviderBuilder::new()
        .connect_http(cfg.rpc_http.parse()?);

    let pool = IAavePool::new(crate::constants::AAVE_POOL, provider.clone());

    let mut opportunities = Vec::new();

    for &borrower in candidates {
        let data = match pool.getUserAccountData(borrower).call().await {
            Ok(d) => d,
            Err(e) => {
                warn!("getUserAccountData failed for {borrower}: {e}");
                continue;
            }
        };

        // Health factor is in 18-decimal fixed point; < 1e18 = liquidatable
        let hf_raw: u128 = data.healthFactor.try_into().unwrap_or(u128::MAX);
        if hf_raw == 0 || hf_raw >= 1_000_000_000_000_000_000u128 {
            continue; // not liquidatable
        }

        let hf: f64 = hf_raw as f64 / 1e18;
        let debt_usd: u128 = data.totalDebtBase.try_into().unwrap_or(0) / 100; // 8-dec → 6-dec
        let coll_usd: u128 = data.totalCollateralBase.try_into().unwrap_or(0) / 100;

        if debt_usd == 0 || coll_usd == 0 {
            continue;
        }

        // Aave allows liquidating up to 50% of debt in one call
        let debt_to_repay = debt_usd / 2;

        let sim = math::simulate(
            debt_to_repay,
            coll_usd,
            500, // assume 5% bonus — replace with on-chain reserve config
            gas_gwei,
            eth_price_usd,
        );

        if !sim.profitable {
            continue;
        }

        if sim.net_profit_usd < cfg.min_profit_usd as i128 {
            continue;
        }

        info!(
            borrower = %borrower,
            hf = hf,
            debt_usd = debt_usd,
            net_profit = sim.net_profit_usd,
            "Opportunity found"
        );

        opportunities.push(Opportunity {
            borrower,
            health_factor: hf,
            debt_usd,
            collateral_usd: coll_usd,
            collateral_asset: Address::ZERO, // populated in full implementation
            debt_asset:       Address::ZERO,
            debt_to_repay,
            estimated_profit_usd: sim.net_profit_usd,
        });
    }

    // Sort by profit descending
    opportunities.sort_by(|a, b| b.estimated_profit_usd.cmp(&a.estimated_profit_usd));
    Ok(opportunities)
}
