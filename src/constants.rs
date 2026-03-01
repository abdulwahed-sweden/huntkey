/// constants.rs — All Base mainnet addresses and protocol thresholds.
///
/// Ported from: Bitcoin-Sentinel/eth_forensics/simulation/scripts/monitor_base.js (ADDRS block)
/// and deployment_flash.json (contract addresses).

use alloy::primitives::{address, Address};

// ── Chain ──────────────────────────────────────────────────────────────────
pub const CHAIN_ID: u64 = 8453;

// ── Aave V3 (Base) ─────────────────────────────────────────────────────────
pub const AAVE_POOL:     Address = address!("A238Dd80C259a72e81d7e4664a9801593F98d1c5");
pub const AAVE_DATA:     Address = address!("2d8A3C5677189723C4cB8873CfC9C8976FDF38Ac");
pub const AAVE_PROVIDER: Address = address!("e20fCBdBfFC4Dd138cE8b2E6FBb6CB49777ad64D");

// ── DEX Routers (Base) ─────────────────────────────────────────────────────
pub const UNISWAP_ROUTER:    Address = address!("2626664c2603336E57B271c5C0b26F421741e481");
pub const UNISWAP_QUOTER_V2: Address = address!("3d4e44Eb1374240CE5F1B871ab261CD16335B76a");
pub const AERODROME_ROUTER:  Address = address!("cF77a3Ba9A5CA399B7c97c74d54e5b1Beb874E43");
pub const AERODROME_FACTORY: Address = address!("420DD381b31aEf6683db6B902084cB0FFECe40Da");

// ── Tokens (Base) ─────────────────────────────────────────────────────────
pub const WETH: Address = address!("4200000000000000000000000000000000000006");
pub const USDC: Address = address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913");

// ── Infrastructure ─────────────────────────────────────────────────────────
pub const MULTICALL3: Address = address!("cA11bde05977b3631167028862bE2a173976CA11");

// ── Deployed contracts ─────────────────────────────────────────────────────
/// Active flash liquidator V2 (zero-capital) — deployed 2026-02-28
pub const FLASH_LIQUIDATOR_V2: Address =
    address!("E5c3e80C243A6E21883E787013254BeAC829AD1E");
/// Capital-funded fallback
pub const BASE_ALPHA: Address = address!("F8B715bC559032316B56cE41E7fcF7F008a5E093");
/// Owner / operator wallet
pub const OWNER: Address = address!("3011BfD673a9D09f9761203A7fFCca757Af22587");

// ── HF tier thresholds ─────────────────────────────────────────────────────
/// Ported from CONFIG in monitor_base.js
pub const HF_COLD:     f64 = 1.50;
pub const HF_WARM:     f64 = 1.15;
pub const HF_HOT:      f64 = 1.07;
pub const HF_CRITICAL: f64 = 1.04;

// ── Goldilocks debt range (USD, 6-dec USDC units) ─────────────────────────
pub const GOLDILOCKS_MIN_DEBT_USD: u64 = 5_000;
pub const GOLDILOCKS_MAX_DEBT_USD: u64 = 500_000;
pub const PARALLEL_CONVICTION_USD: u64 = 15_000;

// ── Gas caps (in wei) ──────────────────────────────────────────────────────
/// 0.008 ETH — hard ceiling on per-tx gas cost
pub const MAX_GAS_COST_WEI: u128 = 8_000_000_000_000_000; // 0.008 ETH
/// 0.05 ETH — hard ceiling on sequencer bribe
pub const MAX_BRIBE_WEI: u128 = 50_000_000_000_000_000;   // 0.05 ETH
/// Minimum net profit before firing
pub const MIN_NET_PROFIT_USD: f64 = 10.0;
/// Safety floor: skip execution if wallet balance is below this
pub const MIN_WALLET_ETH: f64 = 0.005;

// ── Bribe fractions by market regime ──────────────────────────────────────
/// Ported from BRIBE_* constants in monitor_base.js CONFIG
pub const BRIBE_STABLE:    f64 = 0.62;
pub const BRIBE_VOLATILE:  f64 = 0.78;
pub const BRIBE_CRASH:     f64 = 0.90;
pub const BRIBE_ULTRA:     f64 = 0.94;  // HF <= 1.005
pub const BRIBE_MAX:       f64 = 0.95;
pub const BRIBE_RETRY_INC: f64 = 0.05;

// ── Execution constants ────────────────────────────────────────────────────
pub const GAS_LIMIT:   u64 = 800_000;
pub const MAX_ATTEMPTS: u8 = 6;

// ── Goldilocks delta-neutral detection — symbol-based classification ───────
/// ETH-family assets: HF neutral to ETH price when paired with each other
pub const ETH_FAMILY: &[&str] = &["WETH", "weETH", "wstETH", "cbETH", "rETH", "ezETH", "pxETH"];
pub const STABLE_FAMILY: &[&str] = &["USDC", "USDT", "DAI", "USDbC", "cUSDbC", "LUSD", "GHO", "EURC", "USDS", "FRAX"];
pub const BTC_FAMILY: &[&str] = &["cbBTC", "WBTC", "wBTC", "tBTC", "LBTC"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetFamily {
    Eth,
    Stable,
    Btc,
    Other,
}

pub fn asset_family(sym: &str) -> AssetFamily {
    if ETH_FAMILY.contains(&sym)    { return AssetFamily::Eth; }
    if STABLE_FAMILY.contains(&sym) { return AssetFamily::Stable; }
    if BTC_FAMILY.contains(&sym)    { return AssetFamily::Btc; }
    AssetFamily::Other
}

/// Returns true when collateral and debt are in the same price family.
/// "Other" vs anything = not delta-neutral = include the position.
pub fn is_delta_neutral(coll_sym: &str, debt_sym: &str) -> bool {
    let cf = asset_family(coll_sym);
    let df = asset_family(debt_sym);
    cf != AssetFamily::Other && df != AssetFamily::Other && cf == df
}
