//! gas.rs — EIP-1559 fee + bribe computation.
//!
//! Three tiers (PROBE / STRIKE / KILL) x three regimes (STABLE / VOLATILE / CRASH).
//! All caps enforced at compute time.

use crate::constants::{MAX_BRIBE_WEI, GAS_LIMIT};

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

#[derive(Debug, Clone)]
pub struct GasTier {
    #[allow(dead_code)]
    pub label:                  &'static str,
    pub max_fee_per_gas:        u128,
    pub max_priority_fee:       u128,
    pub bribe_fraction:         f64,
}

struct RegimeMult { fee: f64, priority: f64, bribe: f64 }

fn regime_mult(r: Regime) -> RegimeMult {
    match r {
        Regime::Stable   => RegimeMult { fee: 1.10, priority: 1.20, bribe: 1.00 },
        Regime::Volatile => RegimeMult { fee: 1.30, priority: 1.50, bribe: 1.20 },
        Regime::Crash    => RegimeMult { fee: 1.60, priority: 2.00, bribe: 1.40 },
    }
}

struct TierCfg { fee_mul: f64, pri_mul: f64, bribe_fraction: f64 }

fn tier_cfg(t: Tier) -> TierCfg {
    match t {
        Tier::Probe  => TierCfg { fee_mul: 1.10, pri_mul: 1.00, bribe_fraction: 0.25 },
        Tier::Strike => TierCfg { fee_mul: 1.30, pri_mul: 1.50, bribe_fraction: 0.45 },
        Tier::Kill   => TierCfg { fee_mul: 1.60, pri_mul: 2.00, bribe_fraction: 0.65 },
    }
}

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
    let max_pri = raw_pri.min(max_fee);

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

pub fn select_tier(urgency: f64, eta_min: f64) -> Tier {
    if urgency < 1.002 || eta_min < 5.0  { return Tier::Kill; }
    if urgency < 1.010 || eta_min < 30.0 { return Tier::Strike; }
    Tier::Probe
}

pub fn detect_regime(pct_change_5m: f64) -> Regime {
    if pct_change_5m <= -0.030 { Regime::Crash }
    else if pct_change_5m <= -0.015 { Regime::Volatile }
    else { Regime::Stable }
}

pub fn compute_bribe_wei(gross_profit_wei: u128, bribe_fraction: f64) -> u128 {
    if gross_profit_wei == 0 { return 0; }
    let raw = (gross_profit_wei as f64 * bribe_fraction) as u128;
    raw.min(MAX_BRIBE_WEI)
}

pub fn compute_profit_aware_fees(
    base_fee_wei:       u128,
    gross_profit_wei:   u128,
    urgency:            f64,
    regime:             Regime,
    est_gas:            u64,
    max_bribe_wei:      u128,
    max_bribe_fraction: f64,
) -> GasTier {
    let tier = select_tier(urgency, 30.0);
    let gas_limit = if est_gas > 0 {
        ((est_gas as u128 * 120 / 100).max(GAS_LIMIT as u128)) as u64
    } else {
        GAS_LIMIT
    };

    if gross_profit_wei == 0 {
        return compute_gas_tier(base_fee_wei, 1_000_000_000, tier, regime);
    }

    let rm = regime_mult(regime);
    let tc = tier_cfg(tier);

    let bribe_fraction = (tc.bribe_fraction * rm.bribe).min(max_bribe_fraction);
    let bribe_total = ((gross_profit_wei as f64 * bribe_fraction) as u128).min(max_bribe_wei);
    let priority_fee_per_gas = bribe_total / gas_limit as u128;

    let multiplied_fee = (base_fee_wei as f64 * tc.fee_mul * rm.fee) as u128;
    let max_fee_per_gas = multiplied_fee.max(base_fee_wei + priority_fee_per_gas);

    GasTier {
        label: match tier {
            Tier::Probe  => "PROBE",
            Tier::Strike => "STRIKE",
            Tier::Kill   => "KILL",
        },
        max_fee_per_gas,
        max_priority_fee: priority_fee_per_gas.min(max_fee_per_gas),
        bribe_fraction,
    }
}

pub fn compute_profit_aware_fees_with_bribe(
    base_fee_wei:            u128,
    gross_profit_wei:        u128,
    urgency:                 f64,
    regime:                  Regime,
    est_gas:                 u64,
    max_bribe_wei:           u128,
    bribe_fraction_override: f64,
) -> GasTier {
    let tier = select_tier(urgency, 30.0);
    let gas_limit = if est_gas > 0 {
        ((est_gas as u128 * 120 / 100).max(GAS_LIMIT as u128)) as u64
    } else {
        GAS_LIMIT
    };

    if gross_profit_wei == 0 {
        return compute_gas_tier(base_fee_wei, 1_000_000_000, tier, regime);
    }

    let rm = regime_mult(regime);
    let tc = tier_cfg(tier);

    let bribe_fraction = bribe_fraction_override.min(0.95);
    let bribe_total = ((gross_profit_wei as f64 * bribe_fraction) as u128).min(max_bribe_wei);
    let priority_fee_per_gas = bribe_total / gas_limit as u128;

    let multiplied_fee = (base_fee_wei as f64 * tc.fee_mul * rm.fee) as u128;
    let max_fee_per_gas = multiplied_fee.max(base_fee_wei + priority_fee_per_gas);

    GasTier {
        label: match tier {
            Tier::Probe  => "PROBE",
            Tier::Strike => "STRIKE",
            Tier::Kill   => "KILL",
        },
        max_fee_per_gas,
        max_priority_fee: priority_fee_per_gas.min(max_fee_per_gas),
        bribe_fraction,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_ordering_kill_pays_more() {
        let base = 1_000_000_000u128;
        let pri  = 100_000_000u128;
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
        let bribe = compute_bribe_wei(10_000_000_000_000_000_000, 0.90);
        assert!(bribe <= MAX_BRIBE_WEI);
    }

    #[test]
    fn profit_aware_scales_priority_fee() {
        let gross_profit_wei = 1_000_000_000_000_000_000_u128;
        let base_fee = 1_000_000_000_u128;

        let fees = compute_profit_aware_fees(
            base_fee, gross_profit_wei, 1.001, Regime::Crash,
            800_000, MAX_BRIBE_WEI, 0.90,
        );

        assert!(fees.max_priority_fee > 900_000_000_000,
            "Expected large priority fee, got {} wei/gas", fees.max_priority_fee);
        assert!(fees.max_fee_per_gas >= fees.max_priority_fee);
    }

    #[test]
    fn profit_aware_override_escalates_correctly() {
        let gross = 1_000_000_000_000_000_000_u128;
        let base  = 1_000_000_000_u128;

        let steps = [0.40, 0.60, 0.90];
        let fees: Vec<GasTier> = steps.iter().map(|&frac| {
            compute_profit_aware_fees_with_bribe(
                base, gross, 1.001, Regime::Crash, 800_000, MAX_BRIBE_WEI, frac,
            )
        }).collect();

        assert!(fees[0].max_priority_fee < fees[1].max_priority_fee);
        assert!(fees[1].max_priority_fee < fees[2].max_priority_fee);
        assert!((fees[0].bribe_fraction - 0.40).abs() < 1e-9);
        assert!((fees[1].bribe_fraction - 0.60).abs() < 1e-9);
        assert!((fees[2].bribe_fraction - 0.90).abs() < 1e-9);
    }

    #[test]
    fn profit_aware_respects_absolute_cap() {
        let gross_profit_wei = 100_000_000_000_000_000_000_u128;
        let max_bribe = 50_000_000_000_000_000_u128;
        let base_fee = 1_000_000_000_u128;

        let fees = compute_profit_aware_fees(
            base_fee, gross_profit_wei, 1.001, Regime::Crash,
            800_000, max_bribe, 0.90,
        );

        let max_priority = max_bribe / 800_000;
        assert!(fees.max_priority_fee <= max_priority + 1,
            "Priority fee {} should be capped by max_bribe_wei -> {}",
            fees.max_priority_fee, max_priority);
    }
}
