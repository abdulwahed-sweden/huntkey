/// HuntLoan scanner — identifies liquidatable positions on Aave V3 Base.
///
/// Pipeline: [scanner] → simulator → executor
///
/// Stage 1 — Multicall3 batch (500 addresses / RPC call):
///   getUserAccountData for every candidate → filter HF < 1.0 + Goldilocks range.
///
/// Stage 2 — Reserve resolution (1 Multicall3 per surviving candidate):
///   getUserReserveData for every Aave reserve → identify actual collateral/debt assets.
///   Applies delta-neutral filter: skip positions where collateral and debt are
///   in the same price family (e.g. wstETH collateral + WETH debt).
///
/// Stage 3 — Profit pre-screen:
///   math::simulate with actual liquidation bonus from on-chain reserve config.
use alloy::{
    primitives::{Address, Bytes},
    providers::Provider,
    sol,
    sol_types::SolCall,
};
use eyre::Result;
use tracing::{info, warn};

use crate::{config::Config, constants, math, reserves::ReserveCache};

// ── Aave V3 Pool ─────────────────────────────────────────────────────────────

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

// ── Multicall3 ───────────────────────────────────────────────────────────────

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    interface IMulticall3 {
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

/// Addresses batched per Multicall3 aggregate3() call.
const MULTICALL_CHUNK: usize = 500;

// ── Warm candidate ────────────────────────────────────────────────────────────

/// A warm-zone position: HF above 1.0 but trending toward liquidation.
/// Not executed — only fed to VelocityEngine for ETA tracking.
#[derive(Debug, Clone)]
pub struct WarmCandidate {
    pub borrower:      Address,
    pub health_factor: f64,
}

// ── Opportunity ───────────────────────────────────────────────────────────────

/// A fully resolved liquidation opportunity ready for simulation + execution.
#[derive(Debug, Clone)]
pub struct Opportunity {
    pub borrower:             Address,
    pub health_factor:        f64,
    pub debt_usd:             u128,
    #[allow(dead_code)] // available for future alert enrichment
    pub collateral_usd:       u128,
    /// Resolved on-chain collateral asset address.
    pub collateral_asset:     Address,
    /// Resolved on-chain debt asset address.
    pub debt_asset:           Address,
    /// 50% of total debt in whole USD dollars — used only for profit math.
    pub debt_to_repay:        u128,
    /// 50% of total debt in raw token atoms — passed to requestFlashLiquidation.
    /// Sourced from getUserReserveData.currentVariableDebt (correct units).
    pub debt_to_repay_raw:    u128,
    /// Actual liquidation bonus from Aave reserve config (bps, e.g. 500 = 5%).
    pub liquidation_bonus_bps: u128,
    #[allow(dead_code)] // available for future logging/alert enrichment
    pub estimated_profit_usd: i128,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Scan candidates and return resolved, profitable, non-delta-neutral opportunities.
///
/// The `reserve_cache` must be pre-loaded at engine startup (ReserveCache::load).
pub async fn scan<P: Provider + Clone + Send + Sync + 'static>(
    provider: &P,
    cfg: &Config,
    candidates: &[Address],
    eth_price_usd: u128,
    base_fee_wei: u128,
    reserve_cache: &ReserveCache,
) -> Result<Vec<Opportunity>> {
    // ── Stage 1: batch HF check ───────────────────────────────────────────────
    let mut liquidatable: Vec<(Address, f64, u128, u128)> = Vec::new(); // (addr, hf, debt, coll) USD 6-dec
    let mut rpc_calls = 0_usize;

    for chunk in candidates.chunks(MULTICALL_CHUNK) {
        rpc_calls += 1;
        match hf_chunk_full(provider, cfg, chunk).await {
            Ok(mut v) => liquidatable.append(&mut v),
            Err(e)    => warn!("HF chunk {} failed: {}", rpc_calls, e),
        }
    }

    if liquidatable.is_empty() {
        return Ok(vec![]);
    }

    info!(
        candidates  = candidates.len(),
        rpc_batches = rpc_calls,
        liquidatable = liquidatable.len(),
        "Stage 1 complete"
    );

    // ── Stage 2: resolve actual assets + delta-neutral filter ────────────────
    let borrowers: Vec<Address> = liquidatable.iter().map(|(a, ..)| *a).collect();
    let positions: std::collections::HashMap<Address, reserves::ResolvedPosition> =
        reserves::resolve_positions(provider, reserve_cache, &borrowers).await;

    let mut opportunities = Vec::new();

    for (borrower, hf, debt_usd, coll_usd) in &liquidatable {
        let pos = match positions.get(borrower) {
            Some(p) => p,
            None    => {
                warn!(borrower = %borrower, "Reserve resolution failed — skipping");
                continue;
            }
        };

        // Skip delta-neutral positions (e.g. wstETH vs WETH — no directional profit)
        if pos.is_delta_neutral {
            continue;
        }

        // USD approximation (whole dollars) — used only for profit pre-screen math.
        let debt_to_repay = debt_usd / 2;
        // Raw token atoms from getUserReserveData — used for the actual flash loan call.
        let debt_to_repay_raw = pos.debt_amount_raw / 2;

        let sim = math::simulate(
            debt_to_repay,
            pos.bonus_bps,
            base_fee_wei,
            eth_price_usd,
        );

        if !sim.profitable || sim.net_profit_usd < cfg.min_profit_usd as i128 {
            continue;
        }

        info!(
            borrower          = %borrower,
            hf                = hf,
            debt_usd          = debt_usd,
            debt_to_repay_raw = debt_to_repay_raw,
            coll              = %pos.collateral_asset,
            debt              = %pos.debt_asset,
            bonus_bps         = pos.bonus_bps,
            profit            = sim.net_profit_usd,
            "Opportunity found"
        );

        opportunities.push(Opportunity {
            borrower:              *borrower,
            health_factor:         *hf,
            debt_usd:              *debt_usd,
            collateral_usd:        *coll_usd,
            collateral_asset:      pos.collateral_asset,
            debt_asset:            pos.debt_asset,
            debt_to_repay,
            debt_to_repay_raw,
            liquidation_bonus_bps: pos.bonus_bps,
            estimated_profit_usd:  sim.net_profit_usd,
        });
    }

    Ok(opportunities)
}

// ── Public API — warm-zone scan ───────────────────────────────────────────────

/// Scan candidates for warm-zone positions: HF_HOT < HF < HF_WARM (1.07–1.15).
///
/// These positions are not yet liquidatable but are trending downward.
/// Returns (address, hf) pairs for feeding into the VelocityEngine so ETA
/// predictions are ready before the position crosses HF = 1.0.
///
/// No reserve resolution or profit math — intentionally cheap (Multicall3 only).
pub async fn scan_warm<P: Provider>(
    provider: &P,
    cfg: &Config,
    candidates: &[Address],
) -> Vec<WarmCandidate> {
    let mut warm = Vec::new();
    for chunk in candidates.chunks(MULTICALL_CHUNK) {
        match hf_batch_raw(provider, cfg, chunk).await {
            Ok(v) => {
                for (addr, hf, debt_usd) in v {
                    // Warm zone: above liquidation but approaching
                    if hf > 1.0
                        && hf < constants::HF_WARM
                        && debt_usd >= constants::GOLDILOCKS_MIN_DEBT_USD as u128
                    {
                        warm.push(WarmCandidate { borrower: addr, health_factor: hf });
                    }
                }
            }
            Err(e) => warn!("Warm HF chunk failed: {}", e),
        }
    }
    warm
}

// ── Internal — Stage 1 batch ──────────────────────────────────────────────────

/// Raw Multicall3 batch — returns (addr, hf, debt_usd) for every non-zero-debt
/// address. No HF filtering applied; callers apply their own range logic.
async fn hf_batch_raw<P: Provider>(
    provider: &P,
    cfg: &Config,
    chunk: &[Address],
) -> Result<Vec<(Address, f64, u128)>> {
    let calls: Vec<IMulticall3::Call3> = chunk
        .iter()
        .map(|&addr| IMulticall3::Call3 {
            target:       cfg.aave_pool,
            allowFailure: true,
            callData:     Bytes::from(
                IAavePool::getUserAccountDataCall { user: addr }.abi_encode(),
            ),
        })
        .collect();

    let mc      = IMulticall3::new(constants::MULTICALL3, provider);
    let results = mc.aggregate3(calls).call().await?;

    let mut out = Vec::new();

    for (&borrower, result) in chunk.iter().zip(results.iter()) {
        if !result.success || result.returnData.is_empty() {
            continue;
        }
        let data =
            match IAavePool::getUserAccountDataCall::abi_decode_returns(&result.returnData) {
                Ok(d) => d,
                Err(_) => continue,
            };

        // HF 18-dec fixed-point
        let hf_raw: u128 = match data.healthFactor.try_into() {
            Ok(v) => v,
            Err(_) => continue,
        };
        if hf_raw == 0 { continue; }

        let hf: f64  = hf_raw as f64 / 1e18;
        // Aave V3 BASE_CURRENCY_UNIT = 10^8 (confirmed on-chain via oracle.BASE_CURRENCY_UNIT()).
        // totalDebtBase is denominated in 10^-8 USD, so dividing by 10^8 gives whole USD dollars.
        let debt_usd = data.totalDebtBase.try_into().unwrap_or(0_u128) / 100_000_000; // 8-dec USD → whole USD $

        if debt_usd == 0 { continue; }

        out.push((borrower, hf, debt_usd));
    }

    Ok(out)
}

/// Full HF batch including collateral USD — used by the liquidatable pipeline.
async fn hf_chunk_full<P: Provider>(
    provider: &P,
    cfg: &Config,
    chunk: &[Address],
) -> Result<Vec<(Address, f64, u128, u128)>> {
    let calls: Vec<IMulticall3::Call3> = chunk
        .iter()
        .map(|&addr| IMulticall3::Call3 {
            target:       cfg.aave_pool,
            allowFailure: true,
            callData:     Bytes::from(
                IAavePool::getUserAccountDataCall { user: addr }.abi_encode(),
            ),
        })
        .collect();

    let mc      = IMulticall3::new(constants::MULTICALL3, provider);
    let results = mc.aggregate3(calls).call().await?;

    let mut out = Vec::new();

    for (&borrower, result) in chunk.iter().zip(results.iter()) {
        if !result.success || result.returnData.is_empty() {
            continue;
        }
        let data =
            match IAavePool::getUserAccountDataCall::abi_decode_returns(&result.returnData) {
                Ok(d) => d,
                Err(_) => continue,
            };

        let hf_raw: u128 = match data.healthFactor.try_into() {
            Ok(v) => v,
            Err(_) => continue,
        };
        if hf_raw == 0 || hf_raw >= 1_000_000_000_000_000_000_u128 {
            continue;
        }

        let hf: f64   = hf_raw as f64 / 1e18;
        // Aave V3 BASE_CURRENCY_UNIT = 10^8: divide by 10^8 to get whole USD dollars.
        let debt_usd  = data.totalDebtBase.try_into().unwrap_or(0_u128) / 100_000_000;
        let coll_usd  = data.totalCollateralBase.try_into().unwrap_or(0_u128) / 100_000_000;

        if debt_usd == 0 || coll_usd == 0 { continue; }

        // Goldilocks filter: $5K – $500K
        if debt_usd < constants::GOLDILOCKS_MIN_DEBT_USD as u128
            || debt_usd > constants::GOLDILOCKS_MAX_DEBT_USD as u128
        {
            continue;
        }

        out.push((borrower, hf, debt_usd, coll_usd));
    }

    Ok(out)
}

// ── Bring reserves into scope ─────────────────────────────────────────────────
use crate::reserves;
