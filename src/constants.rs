//! HuntLoan constants — all Base mainnet addresses and protocol thresholds.
//!
//! Ported from: Bitcoin-Sentinel/eth_forensics/simulation/scripts/monitor_base.js (ADDRS block)
//! and deployment_flash.json (contract addresses).
#![allow(dead_code)]

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
/// HuntLoanFlashReceiver V2 — deployed 2026-03-04 on Base mainnet (ReentrancyGuard + sweepToUsdc)
pub const HUNTLOAN_FLASH_RECEIVER: Address =
    address!("60d0C491dF2d35E4C95D98dF37897f908b04b46f");
/// Legacy AbdulwahidFlashLiquidator V2 (fallback) — deployed 2026-02-28
pub const LEGACY_FLASH_LIQUIDATOR: Address =
    address!("E5c3e80C243A6E21883E787013254BeAC829AD1E");
/// Legacy capital-funded contract (fallback)
pub const LEGACY_BASE_ALPHA: Address = address!("F8B715bC559032316B56cE41E7fcF7F008a5E093");
/// Operator wallet
pub const OPERATOR: Address = address!("3011BfD673a9D09f9761203A7fFCca757Af22587");

// ── HF tier thresholds ─────────────────────────────────────────────────────
/// Ported from CONFIG in monitor_base.js
pub const HF_COLD:     f64 = 1.50;
pub const HF_WARM:     f64 = 1.15;
pub const HF_HOT:      f64 = 1.07;
pub const HF_CRITICAL: f64 = 1.04;

// ── Goldilocks debt range (USD, 6-dec USDC units) ─────────────────────────
pub const GOLDILOCKS_MIN_DEBT_USD: u64 = 5_000;
pub const GOLDILOCKS_MAX_DEBT_USD: u64 = 500_000;

// ── Gas caps (in wei) ──────────────────────────────────────────────────────
/// 0.008 ETH — hard ceiling on per-tx gas cost
pub const MAX_GAS_COST_WEI: u128 = 8_000_000_000_000_000; // 0.008 ETH
/// 2 ETH -- absolute safety ceiling on sequencer bribe (configurable via MAX_BRIBE_WEI env).
/// This is a catastrophe guard, NOT the economic limit. The profit-fraction cap
/// (DEFAULT_MAX_BRIBE_FRACTION) is the real limiter in normal operation.
pub const MAX_BRIBE_WEI: u128 = 2_000_000_000_000_000_000; // 2 ETH
/// Maximum fraction of gross profit payable as priority fee bribe.
/// Default 0.90 = willing to pay up to 90% of gross profit for inclusion.
pub const DEFAULT_MAX_BRIBE_FRACTION: f64 = 0.90;
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

// ── RBF escalation ──────────────────────────────────────────────────────
/// Bribe-fraction steps for Replace-By-Fee escalation loop.
/// Final attempt uses `config.max_bribe_fraction` (0.90), giving 4 total: 40% → 60% → 80% → 90%.
pub const RBF_BRIBE_STEPS: &[f64] = &[0.40, 0.60, 0.80];
/// Wait between RBF attempts before escalating (ms).
pub const RBF_WAIT_MS: u64 = 200;

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

// ── Address-based family lookup (for on-chain reserve resolution) ────────────
// Maps known Base Aave V3 token addresses → AssetFamily without a symbol lookup.

/// Resolve asset family directly from contract address.
/// Used by the reserves module which works with addresses, not symbols.
pub fn asset_family_by_addr(addr: Address) -> AssetFamily {
    // ETH family — Base mainnet
    const ETH_ADDRS: &[Address] = &[
        address!("4200000000000000000000000000000000000006"), // WETH
        address!("2Ae3F1Ec7F1F5012CFEab0185bfc7aa3cf0DEc22"), // cbETH
        address!("c1CBa3fCea344f92D9239c08C0568f6F2F0ee452"), // wstETH
        address!("04C0599Ae5A44757c0af6F9eC3b93da8976c150A"), // weETH
        address!("B6fe221Fe9EeF5aBa221c348bA20A1Bf5e73624c"), // rETH
        address!("9Bcef72be871e61ED4fBbc7630889beE758eb81D"), // rETH (alt)
        address!("7FcD174E80f567B3CE2f6C75B27b4b06A6A7e24B"), // ezETH
        address!("1FE5da4fad2E30a0aB0C3b22F04C5ab1Ab4f29ba"), // pxETH
    ];
    // Stable family — Base mainnet
    const STABLE_ADDRS: &[Address] = &[
        address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"), // USDC
        address!("d9aAEc86B65D86f6A7B5B1b0c42FFA531710b6CA"), // USDbC
        address!("50c5725949A6F0c72E6C4a641F24049A917DB0Cb"), // DAI
        address!("4A3A6Dd60A34bB2Aba60D73B4C88315E9CeB6A3D"), // USDT
        address!("cD68DFf4415358c35a28f96Fd5bF7083B37B45A4"), // LUSD
        address!("6Bb7a212910682DCFdbd5BCBb3e28FB4E8da10Ee"), // GHO
        address!("60a3E35Cc302bFA44Cb288Bc5a4F316Fdb1adb42"), // EURC
    ];
    // BTC family — Base mainnet
    const BTC_ADDRS: &[Address] = &[
        address!("cbB7C0000aB88B473b1f5aFd9ef808440eed33Bf"), // cbBTC
        address!("2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"), // WBTC (bridged)
    ];

    if ETH_ADDRS.contains(&addr)    { return AssetFamily::Eth; }
    if STABLE_ADDRS.contains(&addr) { return AssetFamily::Stable; }
    if BTC_ADDRS.contains(&addr)    { return AssetFamily::Btc; }
    AssetFamily::Other
}
