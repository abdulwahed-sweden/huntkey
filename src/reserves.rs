/// HuntLoan reserve resolution — finds the best (collateral, debt) asset pair
/// for a liquidatable Aave V3 borrower.
///
/// Problem: getUserAccountData returns aggregate USD values but not which
/// specific assets are being used as collateral or borrowed. This module
/// resolves those assets so the executor can call requestFlashLiquidation
/// with correct parameters.
///
/// Algorithm per borrower:
///   1. Batch getUserReserveData for every Aave V3 reserve via Multicall3
///      (one RPC call regardless of number of reserves)
///   2. Collect candidates: collateral reserves (aTokenBalance > 0 && enabled)
///                          and debt reserves (variableDebt > 0)
///   3. Select highest-value collateral + highest-value debt
///   4. Return (collateral_addr, debt_addr, liquidation_bonus_bps)
///
/// The reserve list is fetched once at engine startup and cached.
use std::collections::HashMap;

use alloy::{
    primitives::{address, Address, Bytes},
    providers::Provider,
    sol,
    sol_types::SolCall,
};
use eyre::Result;
use tracing::{info, warn};

use crate::constants::{self, MULTICALL3};

// ── Aave interfaces ──────────────────────────────────────────────────────────

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    interface IAavePoolReserves {
        function getReservesList() external view returns (address[] memory);
    }

    #[allow(missing_docs)]
    #[sol(rpc)]
    interface IAaveDataProvider {
        function getUserReserveData(address asset, address user) external view returns (
            uint256 currentATokenBalance,
            uint256 currentStableDebt,
            uint256 currentVariableDebt,
            uint256 principalStableDebt,
            uint256 scaledVariableDebt,
            uint256 stableBorrowRate,
            uint256 liquidityRate,
            uint40  stableRateLastUpdated,
            bool    usedAsCollateralEnabled
        );
        function getReserveConfigurationData(address asset) external view returns (
            uint256 decimals,
            uint256 ltv,
            uint256 liquidationThreshold,
            uint256 liquidationBonus,
            uint256 reserveFactor,
            bool    usageAsCollateralEnabled,
            bool    borrowingEnabled,
            bool    stableBorrowRateEnabled,
            bool    isActive,
            bool    isFrozen
        );
    }
}

// ── Multicall3 ───────────────────────────────────────────────────────────────

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    interface IMulticall3Reserves {
        struct Call3 {
            address target;
            bool    allowFailure;
            bytes   callData;
        }
        struct Result {
            bool  success;
            bytes returnData;
        }
        function aggregate3(Call3[] calldata calls)
            external payable returns (Result[] memory returnData);
    }
}

// ── Reserve cache ─────────────────────────────────────────────────────────────

/// Static data about one Aave V3 reserve.
#[derive(Debug, Clone)]
pub struct ReserveInfo {
    pub address:               Address,
    /// Liquidation bonus in bps, e.g. 500 = 5%. Derived from Aave's
    /// liquidationBonus field: bonus_bps = liquidationBonus - 10_000.
    pub liquidation_bonus_bps: u128,
    /// Asset family for delta-neutral detection (from address lookup).
    pub family:                constants::AssetFamily,
}

/// Reserve list fetched once at startup and reused for every block.
#[derive(Debug, Clone)]
pub struct ReserveCache {
    pub reserves:         Vec<ReserveInfo>,
    pub data_provider:    Address,
}

impl ReserveCache {
    /// Fetch the active reserve list from Aave V3 Pool and query each
    /// reserve's configuration. One Multicall3 batch per data type.
    pub async fn load<P: Provider>(
        provider: &P,
        pool_addr: Address,
        data_provider_addr: Address,
    ) -> Result<Self> {
        // 1. Get all reserve addresses from the Pool
        let pool = IAavePoolReserves::new(pool_addr, provider);
        let reserve_addrs = pool.getReservesList().call().await?;

        info!(count = reserve_addrs.len(), "[reserves] Active Aave V3 reserves on Base");

        // 2. Batch getReserveConfigurationData for all reserves
        let mc = IMulticall3Reserves::new(MULTICALL3, provider);
        let dp_addr = data_provider_addr;

        let config_calls: Vec<IMulticall3Reserves::Call3> = reserve_addrs
            .iter()
            .map(|&addr| IMulticall3Reserves::Call3 {
                target:       dp_addr,
                allowFailure: true,
                callData:     Bytes::from(
                    IAaveDataProvider::getReserveConfigurationDataCall { asset: addr }
                        .abi_encode(),
                ),
            })
            .collect();

        let config_results = mc.aggregate3(config_calls).call().await?;

        // 3. Build reserve list
        let mut reserves = Vec::new();
        for (&addr, result) in reserve_addrs.iter().zip(config_results.iter()) {
            if !result.success || result.returnData.is_empty() {
                warn!(reserve = %addr, "[reserves] getReserveConfigurationData failed");
                continue;
            }
            let cfg = match IAaveDataProvider::getReserveConfigurationDataCall::
                abi_decode_returns(&result.returnData)
            {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Skip inactive or frozen reserves
            if !cfg.isActive || cfg.isFrozen {
                continue;
            }

            // Aave stores bonus as 10_000 + bonus_bps (e.g. 10500 = 5%)
            let aave_bonus: u128 = cfg.liquidationBonus.try_into().unwrap_or(10_000);
            let liquidation_bonus_bps = aave_bonus.saturating_sub(10_000);

            reserves.push(ReserveInfo {
                address: addr,
                liquidation_bonus_bps,
                family: constants::asset_family_by_addr(addr),
            });
        }

        info!(loaded = reserves.len(), "[reserves] Reserve cache ready");

        Ok(Self {
            reserves,
            data_provider: data_provider_addr,
        })
    }
}

// ── Position resolution ───────────────────────────────────────────────────────

/// Resolved (collateral, debt) pair for a specific borrower.
#[derive(Debug, Clone)]
pub struct ResolvedPosition {
    pub collateral_asset:     Address,
    pub collateral_bonus_bps: u128,
    pub debt_asset:           Address,
    /// Best estimate of liquidation bonus in bps (from collateral reserve config).
    pub bonus_bps:            u128,
    /// True if collateral and debt are in the same asset family (e.g. WETH/weETH).
    pub is_delta_neutral:     bool,
}

/// Batch-resolve the best (collateral, debt) pair for a set of borrowers.
///
/// For each borrower: batches getUserReserveData for ALL reserves in one
/// Multicall3 call. Picks highest aToken balance as collateral, highest
/// variable debt as debt asset.
///
/// Returns a map: borrower → ResolvedPosition.
/// Missing entries = position could not be resolved (skip execution).
pub async fn resolve_positions<P: Provider>(
    provider: &P,
    cache: &ReserveCache,
    borrowers: &[Address],
) -> HashMap<Address, ResolvedPosition> {
    let mut out = HashMap::new();

    for &borrower in borrowers {
        match resolve_single(provider, cache, borrower).await {
            Some(pos) => {
                out.insert(borrower, pos);
            }
            None => {
                warn!(borrower = %borrower, "[reserves] Could not resolve position");
            }
        }
    }

    out
}

async fn resolve_single<P: Provider>(
    provider: &P,
    cache: &ReserveCache,
    borrower: Address,
) -> Option<ResolvedPosition> {
    let mc = IMulticall3Reserves::new(MULTICALL3, provider);

    // Batch getUserReserveData for every reserve in one call
    let calls: Vec<IMulticall3Reserves::Call3> = cache
        .reserves
        .iter()
        .map(|r| IMulticall3Reserves::Call3 {
            target:       cache.data_provider,
            allowFailure: true,
            callData:     Bytes::from(
                IAaveDataProvider::getUserReserveDataCall {
                    asset: r.address,
                    user:  borrower,
                }
                .abi_encode(),
            ),
        })
        .collect();

    let results = mc.aggregate3(calls).call().await.ok()?;

    // Parse results: find best collateral and debt
    let mut best_collateral: Option<(Address, u128, u128)> = None; // (addr, atoken_balance, bonus_bps)
    let mut best_debt:       Option<(Address, u128)>       = None; // (addr, total_debt)

    for (reserve, result) in cache.reserves.iter().zip(results.iter()) {
        if !result.success || result.returnData.is_empty() {
            continue;
        }
        let data = IAaveDataProvider::getUserReserveDataCall::
            abi_decode_returns(&result.returnData).ok()?;

        let atoken_bal: u128 = data.currentATokenBalance.try_into().unwrap_or(0);
        let var_debt:   u128 = data.currentVariableDebt.try_into().unwrap_or(0);
        let stb_debt:   u128 = data.currentStableDebt.try_into().unwrap_or(0);
        let total_debt  = var_debt + stb_debt;

        // Collateral: positive aToken balance AND enabled as collateral
        if atoken_bal > 0 && data.usedAsCollateralEnabled {
            if best_collateral.map_or(true, |(_, bal, _)| atoken_bal > bal) {
                best_collateral = Some((reserve.address, atoken_bal, reserve.liquidation_bonus_bps));
            }
        }

        // Debt: positive variable or stable debt
        if total_debt > 0 {
            if best_debt.map_or(true, |(_, d)| total_debt > d) {
                best_debt = Some((reserve.address, total_debt));
            }
        }
    }

    let (coll_addr, _, bonus_bps) = best_collateral?;
    let (debt_addr, _)            = best_debt?;

    let coll_family = constants::asset_family_by_addr(coll_addr);
    let debt_family = constants::asset_family_by_addr(debt_addr);
    let is_delta_neutral = coll_family != constants::AssetFamily::Other
        && debt_family != constants::AssetFamily::Other
        && coll_family == debt_family;

    Some(ResolvedPosition {
        collateral_asset:     coll_addr,
        collateral_bonus_bps: bonus_bps,
        debt_asset:           debt_addr,
        bonus_bps,
        is_delta_neutral,
    })
}
