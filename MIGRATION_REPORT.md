# MIGRATION REPORT — huntloan

Migration from: `Bitcoin-Sentinel/eth_forensics/simulation/` (Node.js)
Migration to:   `huntloan/` (Rust + Solidity / Foundry)
Date: 2026-03-01

---

## 1. Reused Assets

| Old file | New location | How |
|---|---|---|
| `scripts/monitor_base.js` → `CONFIG` block | `src/config.rs` + `src/constants.rs` | All thresholds, addresses, env-var names ported verbatim |
| `scripts/monitor_base.js` → `ADDRS` block | `src/constants.rs` | All Base mainnet addresses copied exactly |
| `scripts/monitor_base.js` → `ETH_FAMILY_SYMS`, `STABLE_FAMILY_SYMS`, `BTC_FAMILY_SYMS`, `isDeltaNeutral()` | `src/constants.rs` → `asset_family()`, `is_delta_neutral()` | Direct port — same symbol sets, same logic |
| `scripts/gas_strategy.js` → `CAPS`, `REGIME_MULT`, `TIER_CONFIG`, `computeGasTiers()`, `selectTier()`, `computeBribeWei()`, `validateCaps()` | `src/gas.rs` | Full port — tiers, regime multipliers, bribe fractions, caps all preserved |
| `scripts/telegram.js` → all formatters | `src/alerts.rs` + `eth_forensics/simulation/scripts/telegram.js` | Rust version in `alerts.rs`; JS version already rebuilt with clean format (no emoji) |
| `scripts/monitor_base.js` → HF tier thresholds | `src/constants.rs` → `HF_COLD/WARM/HOT/CRITICAL` | Values: 1.50 / 1.15 / 1.07 / 1.04 |
| `scripts/monitor_base.js` → BRIBE_* constants | `src/constants.rs` → `BRIBE_STABLE/VOLATILE/CRASH/ULTRA` | Values: 0.62 / 0.78 / 0.90 / 0.94 |
| `scripts/monitor_base.js` → Goldilocks debt range | `src/constants.rs` → `GOLDILOCKS_MIN/MAX_DEBT_USD` | $5K–$500K (aggressive mode) |
| `deployment_flash.json` → contract address | `src/constants.rs` → `FLASH_LIQUIDATOR_V2` | `0xE5c3e80C243A6E21883E787013254BeAC829AD1E` |
| `artifacts/contracts/AbdulwahidFlashLiquidator.sol/AbdulwahidFlashLiquidator.json` | `src/abi/AbdulwahidFlashLiquidator.json` | Copied verbatim — used for alloy bindings |
| `scripts/monitor_base.js` → `POLL_INTERVAL: 400ms` | `src/main.rs` → `sleep(400ms)` | Preserved |
| `scripts/monitor_base.js` → watchlist loading logic | `src/main.rs` → `load_candidates()` | Same JSON schema support (`string` or `{ address, ... }`) |
| `scripts/monitor_base.js` → `PARALLEL_CONVICTION_USD: 15_000` | `src/constants.rs` → `PARALLEL_CONVICTION_USD` | Preserved |
| `scripts/gas_strategy.js` → `MIN_WALLET_ETH: 0.005` | `src/constants.rs` → `MIN_WALLET_ETH` | Preserved |

---

## 2. Modules Implemented

| Rust module | Corresponds to (old) | Status |
|---|---|---|
| `src/constants.rs` | `ADDRS` + `CONFIG` block in `monitor_base.js` | Complete |
| `src/config.rs` | `process.env.*` usage across all scripts | Complete |
| `src/gas.rs` | `gas_strategy.js` (full) | Complete + unit tests |
| `src/math.rs` | `findBestOpportunity()` profitability math | Complete + unit tests |
| `src/scanner.rs` | `monitor_base.js` → main scan loop | Scaffold — needs multicall3 + delta-neutral per-reserve check |
| `src/executor.rs` | `execute_mev.js` → `triggerUniswapLiquidation()` | Scaffold — simulation + broadcast wired |
| `src/alerts.rs` | `telegram.js` | Complete port — all 8 formatters |
| `src/main.rs` | `monitor_base.js` → `main()` event loop | Scaffold — poll loop, watchlist load, Telegram boot |
| `contracts/Huntloan.sol` | `contracts/AbdulwahidFlashLiquidator.sol` | New contract — flash loan + 60/40 profit split + settle() |

---

## 3. NOT Migrated (and Why)

| Old component | Reason |
|---|---|
| `scripts/race_controller.js` (parallel attack) | TODO: port `parallel_attack()` logic to `src/executor.rs` |
| `scripts/private_relay.js` (Flashbots/private tx) | TODO: integrate alloy's `flashbots_*` transport |
| `scripts/discover_risky.js` (subgraph discovery) | TODO: add `src/discovery.rs` with GraphQL + RPC Borrow event scan |
| `scripts/scan_goldilocks.js` (one-shot scan) | Superseded by `src/scanner.rs` which does this in the main loop |
| `scripts/financial_status.js` (health check) | TODO: add `src/status.rs` health report command |
| `scripts/verify.js` / `verify_execution.js` (JS tests) | Replaced by Rust `#[cfg(test)]` in `gas.rs` and `math.rs` |
| Hardhat simulation scripts (Ethereum fork) | Specific to Ethereum simulation environment — not needed in production Rust bot |
| `scripts/deploy_flash.js` / `deploy_base.js` | Replaced by `forge script` / Foundry deployment pipeline |
| Multi-route selection (Uniswap vs Aerodrome) | TODO: implement in `src/executor.rs` — `_swapCollateralToDebt()` in Huntloan.sol is a stub |
| WS block subscription | TODO: replace poll loop with `alloy::providers::WsConnect` for zero-latency CRITICAL tier |
| HF velocity engine (ETA prediction) | TODO: port velocity tracking to `src/scanner.rs` |

---

## 4. TODOs (Prioritized)

1. **`_swapCollateralToDebt()` in Huntloan.sol** — add Uniswap V3 + Aerodrome router calls (swap seized collateral → debt token to repay flash loan)
2. **`src/scanner.rs`** — add per-reserve breakdown via Multicall3 + `isDeltaNeutral()` check (ported as `is_delta_neutral()` in `constants.rs`)
3. **WS block subscription** — replace 400ms HTTP poll with `alloy WsConnect` provider in `main.rs`
4. **`src/executor.rs`** — add dual-route (Uniswap + Aerodrome) parallel attack for `PARALLEL_CONVICTION_USD > $15K`
5. **Watchlist path** — update `WATCHLIST_PATH` in `.env` to point at server watchlist.json (98K entries)
6. **`src/discovery.rs`** — port `discover_risky.js` (Goldsky subgraph + RPC Borrow events)
7. **ETH price feed** — replace `fetch_eth_price_usd()` stub with Chainlink on-chain oracle or Binance REST
8. **Deploy Huntloan.sol** — `forge create` + set `HUNTLOAN_CONTRACT` in `.env`

---

## 5. Key Address Reference (Base Mainnet)

| Name | Address |
|---|---|
| Aave V3 Pool | `0xA238Dd80C259a72e81d7e4664a9801593F98d1c5` |
| Aave Data Provider | `0x2d8A3C5677189723C4cB8873CfC9C8976FDF38Ac` |
| Flash Liquidator V2 (active) | `0xE5c3e80C243A6E21883E787013254BeAC829AD1E` |
| Uniswap V3 Router | `0x2626664c2603336E57B271c5C0b26F421741e481` |
| Aerodrome Router | `0xcF77a3Ba9A5CA399B7c97c74d54e5b1Beb874E43` |
| Multicall3 | `0xcA11bde05977b3631167028862bE2a173976CA11` |
| WETH | `0x4200000000000000000000000000000000000006` |
| USDC | `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913` |
| Owner | `0x3011BfD673a9D09f9761203A7fFCca757Af22587` |
