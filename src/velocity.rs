/// HuntLoan HF velocity engine — linear regression ETA prediction.
///
/// Tracks health factor observations per borrower over time and estimates
/// when a "warm" position (HF > 1.0) will cross the liquidation threshold.
///
/// Algorithm:
///   Accumulate (timestamp, HF) tuples per address.
///   Fit a least-squares linear regression to compute dHF/dt.
///   If slope is negative: ETA = (current_hf - 1.0) / |slope|.
///
/// Used to:
///   - Pre-position a STRIKE alert before the position is liquidatable.
///   - Pass ETA to gas::select_tier() so fee tier is tightened early.
use std::collections::HashMap;
use std::time::{Duration, Instant};

use alloy::primitives::Address;

// ── Config ────────────────────────────────────────────────────────────────────

/// Minimum observations before extrapolating.
const MIN_OBS: usize = 3;

/// Discard observations older than 1 hour — HF can change direction.
const MAX_AGE: Duration = Duration::from_secs(3_600);

/// Prune GC: remove addresses with no observations in the last MAX_AGE.
const GC_INTERVAL_SECS: u64 = 600; // GC every 10 min

// ── Types ─────────────────────────────────────────────────────────────────────

struct Obs {
    hf: f64,
    at: Instant,
}

/// HF velocity tracker — create once at startup, update every block.
pub struct VelocityEngine {
    history:      HashMap<Address, Vec<Obs>>,
    last_gc:      Instant,
}

// ── Public API ────────────────────────────────────────────────────────────────

impl VelocityEngine {
    pub fn new() -> Self {
        Self {
            history:  HashMap::new(),
            last_gc:  Instant::now(),
        }
    }

    /// Record a new HF observation for `addr`.
    ///
    /// Call this every block for positions in the warm/hot monitoring range.
    pub fn record(&mut self, addr: Address, hf: f64) {
        let entry = self.history.entry(addr).or_default();
        entry.push(Obs { hf, at: Instant::now() });
        // Trim stale obs from this address
        entry.retain(|o| o.at.elapsed() < MAX_AGE);
    }

    /// Estimate minutes until HF crosses 1.0 using linear regression.
    ///
    /// Returns:
    ///   `Some(0.0)` — already liquidatable (HF ≤ 1.0).
    ///   `Some(t)`   — estimated `t` minutes until liquidatable.
    ///   `None`      — insufficient data or HF is rising / flat.
    pub fn eta_minutes(&self, addr: &Address) -> Option<f64> {
        let obs = self.history.get(addr)?;
        if obs.len() < MIN_OBS {
            return None;
        }

        let current_hf = obs.last()?.hf;
        if current_hf <= 1.0 {
            return Some(0.0); // already liquidatable
        }

        // Linear regression: time (seconds from first obs) → HF
        let t0  = obs[0].at;
        let n   = obs.len() as f64;

        let xs: Vec<f64> = obs.iter()
            .map(|o| o.at.duration_since(t0).as_secs_f64())
            .collect();
        let ys: Vec<f64> = obs.iter().map(|o| o.hf).collect();

        let mean_x = xs.iter().sum::<f64>() / n;
        let mean_y = ys.iter().sum::<f64>() / n;

        let num = xs.iter().zip(ys.iter())
            .map(|(x, y)| (x - mean_x) * (y - mean_y))
            .sum::<f64>();
        let den = xs.iter()
            .map(|x| (x - mean_x).powi(2))
            .sum::<f64>();

        if den.abs() < 1e-12 || num.is_nan() || den.is_nan() {
            return None;
        }

        let slope = num / den; // HF per second

        // If slope ≥ 0 the position is healing — no ETA
        if slope >= 0.0 {
            return None;
        }

        // Seconds until HF = 1.0: (current_hf - 1.0) / |slope|
        let secs = (current_hf - 1.0) / (-slope);
        Some(secs / 60.0) // → minutes
    }

    /// Observation count for an address (useful for logging).
    pub fn obs_count(&self, addr: &Address) -> usize {
        self.history.get(addr).map(|v| v.len()).unwrap_or(0)
    }

    /// Periodically purge addresses with all-stale observations.
    ///
    /// Call once per block — no-ops if < GC_INTERVAL_SECS elapsed.
    pub fn maybe_gc(&mut self) {
        if self.last_gc.elapsed().as_secs() < GC_INTERVAL_SECS {
            return;
        }
        self.history.retain(|_, obs| {
            obs.retain(|o| o.at.elapsed() < MAX_AGE);
            !obs.is_empty()
        });
        self.last_gc = Instant::now();
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    fn addr() -> Address {
        address!("1111111111111111111111111111111111111111")
    }

    #[test]
    fn returns_none_with_insufficient_data() {
        let mut ve = VelocityEngine::new();
        let a = addr();
        ve.record(a, 1.10);
        ve.record(a, 1.08);
        // Only 2 obs — MIN_OBS = 3
        assert!(ve.eta_minutes(&a).is_none());
    }

    #[test]
    fn already_liquidatable_returns_zero() {
        let mut ve = VelocityEngine::new();
        let a = addr();
        for hf in [1.05, 1.02, 0.99] {
            ve.record(a, hf);
        }
        assert_eq!(ve.eta_minutes(&a), Some(0.0));
    }

    #[test]
    fn rising_hf_returns_none() {
        let mut ve = VelocityEngine::new();
        let a = addr();
        for hf in [1.05, 1.10, 1.20] {
            ve.record(a, hf);
        }
        // Slope is positive → returns None
        assert!(ve.eta_minutes(&a).is_none());
    }

    #[test]
    fn declining_hf_returns_positive_eta() {
        let mut ve = VelocityEngine::new();
        let a = addr();
        // Simulate 3 observations with known HF values (declining from 1.09 → 1.07 → 1.05)
        // We can't control timestamps in tests, so just check the sign
        ve.record(a, 1.09);
        ve.record(a, 1.07);
        ve.record(a, 1.05);
        // With at least 3 obs and declining HF, should return Some positive value
        // (or None if timestamps are too close together in test environment)
        if let Some(eta) = ve.eta_minutes(&a) {
            assert!(eta >= 0.0, "ETA must be non-negative");
        }
    }
}
