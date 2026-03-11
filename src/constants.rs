//! Shared constants — chain config, gas caps, RBF escalation.

// ── Chain ──────────────────────────────────────────────────────────────────
#[allow(dead_code)]
pub const CHAIN_ID: u64 = 8453;

// ── Gas caps (in wei) ──────────────────────────────────────────────────────
pub const GAS_LIMIT: u64 = 800_000;
pub const GAS_HEADROOM_NUM: u128 = 120;
pub const GAS_HEADROOM_DEN: u128 = 100;

/// 2 ETH -- absolute safety ceiling on sequencer bribe.
pub const MAX_BRIBE_WEI: u128 = 2_000_000_000_000_000_000;
/// Maximum fraction of gross profit payable as priority fee bribe.
pub const DEFAULT_MAX_BRIBE_FRACTION: f64 = 0.90;

// ── RBF escalation ──────────────────────────────────────────────────────
/// Bribe-fraction steps for Replace-By-Fee escalation loop.
pub const RBF_BRIBE_STEPS: &[f64] = &[0.40, 0.60, 0.80];
/// Wait between RBF attempts before escalating (ms).
pub const RBF_WAIT_MS: u64 = 200;
