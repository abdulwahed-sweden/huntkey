/// HuntLoan scanner — identifies liquidatable positions on Aave V3 Base.
///
/// Pipeline position: [scanner] → simulator → executor
///
/// For each candidate address:
///   1. Call IAavePool.getUserAccountData() via HTTP RPC
///   2. Check health factor < 1.0 (fixed-point 18-dec)
///   3. Filter by Goldilocks debt range ($5K–$500K)
///   4. Pre-screen profitability via math::simulate
///   5. Return sorted opportunities (highest profit first)
///
/// Full multicall3 batching and per-reserve delta-neutral check are TODO.
use alloy::{
    primitives::Address,
    providers::Provider,
    sol,
};
use eyre::Result;
use tracing::{info, warn};

use crate::{config::Config, constants, math};

/// Aave V3 Pool — getUserAccountData interface.
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

/// A liquidation opportunity ready for simulation + execution.
#[derive(Debug, Clone)]
pub struct Opportunity {
    pub borrower:             Address,
    pub health_factor:        f64,
    pub debt_usd:             u128,
    pub collateral_usd:       u128,
    pub collateral_asset:     Address, // Address::ZERO until per-reserve scan
    pub debt_asset:           Address, // Address::ZERO until per-reserve scan
    pub debt_to_repay:        u128,    // 50% of total debt (Aave V3 cap)
    pub estimated_profit_usd: i128,
}

/// Scan a list of candidate borrowers and return profitable opportunities.
///
/// This function is called once per block by the HuntLoanEngine.
/// All HTTP calls are made against the standard (non-wallet) provider.
pub async fn scan<P: Provider>(
    provider: &P,
    cfg: &Config,
    candidates: &[Address],
    eth_price_usd: u128,
    base_fee_wei: u128,
) -> Result<Vec<Opportunity>> {
    let pool = IAavePool::new(cfg.aave_pool, provider);

    let mut opportunities = Vec::new();

    for &borrower in candidates {
        let data = match pool.getUserAccountData(borrower).call().await {
            Ok(d) => d,
            Err(e) => {
                warn!(borrower = %borrower, "getUserAccountData failed: {e}");
                continue;
            }
        };

        // Health factor is 18-decimal fixed-point; < 1e18 means liquidatable
        let hf_raw: u128 = match data.healthFactor.try_into() {
            Ok(v) => v,
            Err(_) => continue,
        };
        if hf_raw == 0 || hf_raw >= 1_000_000_000_000_000_000_u128 {
            continue;
        }

        let hf: f64 = hf_raw as f64 / 1e18;
        // Aave base units are 8-decimal; convert to 6-decimal USD
        let debt_usd: u128 = data.totalDebtBase.try_into().unwrap_or(0);
        let coll_usd: u128 = data.totalCollateralBase.try_into().unwrap_or(0);
        let debt_usd = debt_usd / 100;
        let coll_usd = coll_usd / 100;

        if debt_usd == 0 || coll_usd == 0 {
            continue;
        }

        // Goldilocks filter: $5K–$500K debt range
        if debt_usd < constants::GOLDILOCKS_MIN_DEBT_USD as u128
            || debt_usd > constants::GOLDILOCKS_MAX_DEBT_USD as u128
        {
            continue;
        }

        // Aave V3 allows liquidating up to 50% of debt per call
        let debt_to_repay = debt_usd / 2;

        let sim = math::simulate(
            debt_to_repay,
            coll_usd,
            500, // 5% liquidation bonus — TODO: per-reserve on-chain lookup
            base_fee_wei,
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
            hf       = hf,
            debt_usd = debt_usd,
            profit   = sim.net_profit_usd,
            "Opportunity identified"
        );

        opportunities.push(Opportunity {
            borrower,
            health_factor:        hf,
            debt_usd,
            collateral_usd:       coll_usd,
            collateral_asset:     Address::ZERO, // TODO: per-reserve breakdown
            debt_asset:           Address::ZERO,
            debt_to_repay,
            estimated_profit_usd: sim.net_profit_usd,
        });
    }

    // Sort by profit descending — execute highest-value opportunity first
    opportunities.sort_by(|a, b| b.estimated_profit_usd.cmp(&a.estimated_profit_usd));
    Ok(opportunities)
}
