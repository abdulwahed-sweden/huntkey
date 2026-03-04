/// HuntLoan simulation layer — on-chain dry-run before any broadcast.
///
/// Pipeline position: scanner → [simulator] → executor
///
/// For each opportunity:
///   1. Encode `requestFlashLiquidation` calldata
///   2. Call via `eth_call` — catches reverts with zero gas cost
///   3. Estimate gas via `eth_estimateGas`
///   4. Cross-check net profit against `math::simulate`
///   5. Return `SimOutput` with pass/fail + timing
use std::time::Instant;

use alloy::{
    primitives::{Address, U256},
    providers::Provider,
    sol,
};
use eyre::Result;
use tracing::warn;

use crate::{config::Config, math, scanner::Opportunity};

// Solidity interface — must match HuntLoanFlashReceiver.sol
sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    interface IHuntLoanReceiver {
        function requestFlashLiquidation(
            address debtAsset,
            uint256 debtAmount,
            address collateralAsset,
            address borrower
        ) external;
    }
}

/// Result of the simulation layer for a single opportunity.
#[derive(Debug, Clone)]
pub struct SimOutput {
    /// Whether the on-chain simulation passed (no revert, profitable after gas).
    pub passes:           bool,
    /// Gas units estimated by eth_estimateGas (0 if simulation reverted).
    pub estimated_gas:    u64,
    /// Net profit in USD after gas and flash loan premium (may be negative).
    pub net_profit_usd:   i128,
    /// Revert reason string if simulation failed.
    pub revert_reason:    Option<String>,
    /// Wall-clock time for the eth_call simulation (ms).
    #[allow(dead_code)]
    pub sim_latency_ms:   u64,
}

/// Simulate a liquidation opportunity on-chain via eth_call.
///
/// Uses the HuntLoan contract address from config.
/// Returns Ok(SimOutput) always — revert is captured, not propagated.
pub async fn simulate_on_chain<P: Provider>(
    provider: &P,
    config: &Config,
    opp: &Opportunity,
    eth_price_usd: u128,
    base_fee_wei: u128,
) -> Result<SimOutput> {
    let t = Instant::now();

    let contract = IHuntLoanReceiver::new(config.huntloan_addr, provider);

    // ── 1. eth_call — check for revert ───────────────────────────────────────
    // Must call from the operator address so the contract's onlyOperator
    // modifier passes. Without .from(), eth_call uses address(0) → OnlyOperator().
    let call_result = contract
        .requestFlashLiquidation(
            opp.debt_asset,
            U256::from(opp.debt_to_repay_raw), // raw token atoms — NOT whole USD dollars
            opp.collateral_asset,
            opp.borrower,
        )
        .from(config.operator_addr)
        .call()
        .await;

    let sim_ms = t.elapsed().as_millis() as u64;

    if let Err(e) = call_result {
        let reason = e.to_string();
        warn!(
            borrower = %opp.borrower,
            reason = %reason,
            sim_ms = sim_ms,
            "eth_call simulation reverted"
        );
        return Ok(SimOutput {
            passes:         false,
            estimated_gas:  0,
            net_profit_usd: 0,
            revert_reason:  Some(reason),
            sim_latency_ms: sim_ms,
        });
    }

    // ── 2. Gas estimate ──────────────────────────────────────────────────────
    let gas_est = contract
        .requestFlashLiquidation(
            opp.debt_asset,
            U256::from(opp.debt_to_repay_raw), // raw token atoms — NOT whole USD dollars
            opp.collateral_asset,
            opp.borrower,
        )
        .from(config.operator_addr)
        .estimate_gas()
        .await
        .unwrap_or(crate::constants::GAS_LIMIT); // fallback to conservative estimate

    // ── 3. Profitability check with real gas estimate ────────────────────────
    // Use the actual on-chain liquidation bonus resolved by the scanner via
    // ReserveCache. Aave V3 Base bonuses range from 200bps (stablecoins)
    // to 1000bps (volatile assets); using the real value prevents both
    // false positives (overestimate) and false negatives (underestimate).
    let sim = math::simulate(
        opp.debt_to_repay,
        opp.liquidation_bonus_bps, // RISK-01 fixed: per-reserve bonus, not hardcoded 500
        base_fee_wei,
        eth_price_usd,
    );

    let gas_cost_adj = gas_est as u128 * base_fee_wei * eth_price_usd
        / 1_000_000_000_000_000_000u128;

    // Slippage buffer: reduce gross by SLIPPAGE_BUFFER_BPS to account for price
    // drift between eth_call time and actual broadcast (~200-400ms on Base).
    // 0 bps = no adjustment (default). 2500 = 25% haircut.
    let gross_adj = if config.slippage_buffer_bps > 0 {
        sim.gross_usd.saturating_mul(10_000 - config.slippage_buffer_bps as u128) / 10_000
    } else {
        sim.gross_usd
    };

    let net = gross_adj as i128
        - sim.flash_fee_usd as i128
        - gas_cost_adj as i128;

    let passes = net >= config.min_profit_usd as i128;

    Ok(SimOutput {
        passes,
        estimated_gas:  gas_est,
        net_profit_usd: net,
        revert_reason:  None,
        sim_latency_ms: sim_ms,
    })
}

/// Validate that contract address is set (not zero).
/// Called at startup to warn operator before the engine subscribes to blocks.
#[allow(dead_code)]
pub fn validate_contract_addr(addr: Address) -> bool {
    addr != Address::ZERO
}
