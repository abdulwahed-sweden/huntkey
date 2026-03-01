/// HuntLoan scanner — identifies liquidatable positions on Aave V3 Base.
///
/// Pipeline position: [scanner] → simulator → executor
///
/// Uses Multicall3 to batch getUserAccountData calls: 500 addresses per RPC
/// round-trip instead of one call per address. For 98K candidates this reduces
/// RPC calls from 98,000 → ~196 per block scan.
///
/// Per candidate:
///   1. Batch via Multicall3 aggregate3()
///   2. Check health factor < 1.0 (18-dec fixed-point)
///   3. Goldilocks filter: $5K–$500K debt range
///   4. Pre-screen profitability via math::simulate
///   5. Return sorted opportunities (highest profit first)
use alloy::{
    primitives::{Address, Bytes},
    providers::Provider,
    sol,
    sol_types::SolCall,
};
use eyre::Result;
use tracing::{info, warn};

use crate::{config::Config, constants, math};

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

/// Number of getUserAccountData calls batched per Multicall3 round-trip.
const MULTICALL_CHUNK: usize = 500;

// ── Opportunity ───────────────────────────────────────────────────────────────

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

// ── Public API ────────────────────────────────────────────────────────────────

/// Scan a list of candidate borrowers and return profitable opportunities.
///
/// Batches all getUserAccountData calls via Multicall3 in chunks of 500.
/// This function is called once per block by the HuntLoanEngine.
pub async fn scan<P: Provider>(
    provider: &P,
    cfg: &Config,
    candidates: &[Address],
    eth_price_usd: u128,
    base_fee_wei: u128,
) -> Result<Vec<Opportunity>> {
    let mut opportunities = Vec::new();
    let mut rpc_calls = 0_usize;

    for chunk in candidates.chunks(MULTICALL_CHUNK) {
        rpc_calls += 1;
        match scan_chunk(provider, cfg, chunk, eth_price_usd, base_fee_wei).await {
            Ok(mut opps) => opportunities.append(&mut opps),
            Err(e) => warn!("Multicall3 chunk failed (chunk {}): {}", rpc_calls, e),
        }
    }

    if rpc_calls > 0 {
        info!(
            candidates = candidates.len(),
            rpc_calls  = rpc_calls,
            found      = opportunities.len(),
            "Scan complete"
        );
    }

    // Sort by estimated profit descending
    opportunities.sort_by(|a, b| b.estimated_profit_usd.cmp(&a.estimated_profit_usd));
    Ok(opportunities)
}

// ── Internal ──────────────────────────────────────────────────────────────────

/// Execute one Multicall3 batch and decode the results.
async fn scan_chunk<P: Provider>(
    provider: &P,
    cfg: &Config,
    chunk: &[Address],
    eth_price_usd: u128,
    base_fee_wei: u128,
) -> Result<Vec<Opportunity>> {
    // Encode one getUserAccountData call per address
    let calls: Vec<IMulticall3::Call3> = chunk
        .iter()
        .map(|&addr| IMulticall3::Call3 {
            target:       cfg.aave_pool,
            allowFailure: true, // continue even if individual calls revert
            callData:     Bytes::from(
                IAavePool::getUserAccountDataCall { user: addr }.abi_encode(),
            ),
        })
        .collect();

    // Single RPC round-trip for the whole chunk.
    // alloy v1: call() on a single-return-value function unwraps it directly,
    // so aggregate3 (returns Result[] memory) comes back as Vec<IMulticall3::Result>.
    let mc = IMulticall3::new(constants::MULTICALL3, provider);
    let results = mc.aggregate3(calls).call().await?;

    let mut opps = Vec::new();

    for (&borrower, result) in chunk.iter().zip(results.iter()) {
        if !result.success || result.returnData.is_empty() {
            continue;
        }

        // Decode ABI-encoded return from getUserAccountData
        // abi_decode_returns takes only the raw bytes in alloy 1.x
        let data =
            match IAavePool::getUserAccountDataCall::abi_decode_returns(&result.returnData) {
                Ok(d) => d,
                Err(_) => continue,
            };

        // Health factor is 18-decimal fixed-point; < 1e18 = liquidatable
        let hf_raw: u128 = match data.healthFactor.try_into() {
            Ok(v) => v,
            Err(_) => continue,
        };
        if hf_raw == 0 || hf_raw >= 1_000_000_000_000_000_000_u128 {
            continue;
        }

        let hf: f64 = hf_raw as f64 / 1e18;

        // Aave base units are 8-decimal; convert to 6-decimal USD (÷100)
        let debt_usd: u128 = data.totalDebtBase.try_into().unwrap_or(0) / 100;
        let coll_usd: u128 = data.totalCollateralBase.try_into().unwrap_or(0) / 100;

        if debt_usd == 0 || coll_usd == 0 {
            continue;
        }

        // Goldilocks filter: $5K – $500K debt range
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

        if !sim.profitable || sim.net_profit_usd < cfg.min_profit_usd as i128 {
            continue;
        }

        opps.push(Opportunity {
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

    Ok(opps)
}
