//! gas.rs — EIP-1559 fee + bribe computation.
//!
//! Ported from: Bitcoin-Sentinel/eth_forensics/simulation/scripts/gas_strategy.js
//! Logic: three tiers (PROBE / STRIKE / KILL) × three regimes (STABLE / VOLATILE / CRASH).
//! All caps enforced at compute time.

use crate::constants::{
    MAX_BRIBE_WEI, MAX_GAS_COST_WEI, MIN_NET_PROFIT_USD, MIN_WALLET_ETH,
    BRIBE_CRASH, BRIBE_STABLE, BRIBE_ULTRA, BRIBE_VOLATILE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Probe,
    Strike,
    Kill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Regime {
    Stable,
    Volatile,
    Crash,
}

/// Computed gas parameters for one tier × regime combination.
#[derive(Debug, Clone)]
pub struct GasTier {
    #[allow(dead_code)]
    pub label:                  &'static str,
    pub max_fee_per_gas:        u128, // wei
    pub max_priority_fee:       u128, // wei
    pub bribe_fraction:         f64,  // fraction of gross profit to pay as sequencer bribe
}

/// Regime multipliers — mirrors REGIME_MULT in gas_strategy.js
struct RegimeMult {
    fee: f64,
    priority: f64,
    bribe: f64,
}

fn regime_mult(r: Regime) -> RegimeMult {
    match r {
        Regime::Stable   => RegimeMult { fee: 1.10, priority: 1.20, bribe: 1.00 },
        Regime::Volatile => RegimeMult { fee: 1.30, priority: 1.50, bribe: 1.20 },
        Regime::Crash    => RegimeMult { fee: 1.60, priority: 2.00, bribe: 1.40 },
    }
}

/// Tier config — mirrors TIER_CONFIG in gas_strategy.js
struct TierCfg {
    fee_mul:        f64, // multiply base fee
    pri_mul:        f64, // multiply base priority
    bribe_fraction: f64,
}

fn tier_cfg(t: Tier) -> TierCfg {
    match t {
        Tier::Probe  => TierCfg { fee_mul: 1.10, pri_mul: 1.00, bribe_fraction: 0.25 },
        Tier::Strike => TierCfg { fee_mul: 1.30, pri_mul: 1.50, bribe_fraction: 0.45 },
        Tier::Kill   => TierCfg { fee_mul: 1.60, pri_mul: 2.00, bribe_fraction: 0.65 },
    }
}

/// Compute gas tier parameters from live fee data.
///
/// `base_fee_wei` and `priority_fee_wei` come from `eth_feeHistory` or `eth_maxPriorityFeePerGas`.
pub fn compute_gas_tier(
    base_fee_wei:     u128,
    priority_fee_wei: u128,
    tier:             Tier,
    regime:           Regime,
) -> GasTier {
    let rm  = regime_mult(regime);
    let tc  = tier_cfg(tier);

    let max_fee = (base_fee_wei as f64 * tc.fee_mul * rm.fee) as u128;
    let raw_pri = (priority_fee_wei as f64 * tc.pri_mul * rm.priority) as u128;
    let max_pri = raw_pri.min(max_fee); // EIP-1559: priority ≤ maxFee

    let bribe_fraction = (tc.bribe_fraction * rm.bribe).min(0.95);

    GasTier {
        label: match tier {
            Tier::Probe  => "PROBE",
            Tier::Strike => "STRIKE",
            Tier::Kill   => "KILL",
        },
        max_fee_per_gas:  max_fee,
        max_priority_fee: max_pri,
        bribe_fraction,
    }
}

/// Select tier from HF + ETA.
/// Mirrors selectTier() in gas_strategy.js.
pub fn select_tier(hf: f64, eta_min: f64) -> Tier {
    if hf < 1.002 || eta_min < 5.0  { return Tier::Kill; }
    if hf < 1.010 || eta_min < 30.0 { return Tier::Strike; }
    Tier::Probe
}

/// Detect market regime from ETH price change over 5-min window.
/// Mirrors regime detection in monitor_base.js.
pub fn detect_regime(pct_change_5m: f64) -> Regime {
    if pct_change_5m <= -0.030 { Regime::Crash }
    else if pct_change_5m <= -0.015 { Regime::Volatile }
    else { Regime::Stable }
}

/// Bribe fraction based on HF urgency (used when gas tier bribe is overridden).
#[allow(dead_code)]
pub fn bribe_by_hf(hf: f64) -> f64 {
    if hf <= 1.005 { BRIBE_ULTRA }
    else if hf <= 1.010 { BRIBE_CRASH }
    else if hf <= 1.040 { BRIBE_VOLATILE }
    else { BRIBE_STABLE }
}

/// Compute bribe in wei as a fraction of gross profit, hard-capped.
pub fn compute_bribe_wei(gross_profit_wei: u128, bribe_fraction: f64) -> u128 {
    if gross_profit_wei == 0 { return 0; }
    let raw = (gross_profit_wei as f64 * bribe_fraction) as u128;
    raw.min(MAX_BRIBE_WEI)
}

/// Validate all safety caps before broadcast.
/// Returns Ok(()) or Err with the blocking reason.
#[allow(dead_code)]
pub fn validate_caps(
    gas_cost_wei:   u128,
    bribe_wei:      u128,
    net_profit_usd: f64,
    wallet_eth:     f64,
) -> Result<(), String> {
    if gas_cost_wei > MAX_GAS_COST_WEI {
        return Err(format!(
            "gas {:.6} ETH > cap {:.6} ETH",
            gas_cost_wei as f64 / 1e18,
            MAX_GAS_COST_WEI as f64 / 1e18
        ));
    }
    if bribe_wei > MAX_BRIBE_WEI {
        return Err(format!(
            "bribe {:.6} ETH > cap {:.6} ETH",
            bribe_wei as f64 / 1e18,
            MAX_BRIBE_WEI as f64 / 1e18
        ));
    }
    if net_profit_usd < MIN_NET_PROFIT_USD {
        return Err(format!(
            "net ${:.2} < floor ${:.2}",
            net_profit_usd, MIN_NET_PROFIT_USD
        ));
    }
    if wallet_eth < MIN_WALLET_ETH {
        return Err(format!(
            "wallet {:.4} ETH < safety {:.4} ETH",
            wallet_eth, MIN_WALLET_ETH
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_ordering_kill_pays_more() {
        let base = 1_000_000_000u128; // 1 gwei
        let pri  = 100_000_000u128;   // 0.1 gwei
        let probe  = compute_gas_tier(base, pri, Tier::Probe,  Regime::Stable);
        let strike = compute_gas_tier(base, pri, Tier::Strike, Regime::Stable);
        let kill   = compute_gas_tier(base, pri, Tier::Kill,   Regime::Stable);
        assert!(probe.max_fee_per_gas < strike.max_fee_per_gas);
        assert!(strike.max_fee_per_gas < kill.max_fee_per_gas);
        assert!(probe.bribe_fraction < strike.bribe_fraction);
        assert!(strike.bribe_fraction < kill.bribe_fraction);
    }

    #[test]
    fn crash_pays_more_than_stable() {
        let base = 1_000_000_000u128;
        let pri  = 100_000_000u128;
        let stable = compute_gas_tier(base, pri, Tier::Kill, Regime::Stable);
        let crash  = compute_gas_tier(base, pri, Tier::Kill, Regime::Crash);
        assert!(crash.max_fee_per_gas > stable.max_fee_per_gas);
    }

    #[test]
    fn select_tier_logic() {
        assert_eq!(select_tier(1.001, 100.0), Tier::Kill);
        assert_eq!(select_tier(1.005, 20.0),  Tier::Strike);
        assert_eq!(select_tier(1.050, 100.0), Tier::Probe);
    }

    #[test]
    fn bribe_cap_enforced() {
        // Very large profit should still be capped
        let bribe = compute_bribe_wei(10_000_000_000_000_000_000, 0.90);
        assert!(bribe <= MAX_BRIBE_WEI);
    }
}
