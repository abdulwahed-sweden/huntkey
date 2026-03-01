# MIGRATION REPORT — HuntLoan

Migration Phase 1: `Bitcoin-Sentinel/eth_forensics/simulation/` (Node.js) → `huntloan/` (Rust + Solidity)
Migration Phase 2: Rebranding + Execution Engine + Event-Driven Scanner
Migration Phase 3: Reserve resolution, delta-neutral filter, parallel execution, discovery, alerts, velocity

Date Phase 1: 2026-03-01
Date Phase 2: 2026-03-01
Date Phase 3: 2026-03-01

---

## Phase 1 — Reused Assets

| Old file | New location | How |
|---|---|---|
| `scripts/monitor_base.js` → `CONFIG` block | `src/config.rs` + `src/constants.rs` | All thresholds, addresses, env-var names ported |
| `scripts/monitor_base.js` → `ADDRS` block | `src/constants.rs` | All Base mainnet addresses copied exactly |
| `scripts/monitor_base.js` → `ETH_FAMILY`, `STABLE_FAMILY`, `BTC_FAMILY`, `isDeltaNeutral()` | `src/constants.rs` → `asset_family()`, `is_delta_neutral()` | Direct port — same symbol sets, same logic |
| `scripts/gas_strategy.js` → `CAPS`, `REGIME_MULT`, `TIER_CONFIG`, `computeGasTiers()`, `selectTier()`, `computeBribeWei()`, `validateCaps()` | `src/gas.rs` | Full port — tiers, regime multipliers, bribe fractions, caps all preserved |
| `scripts/telegram.js` → all formatters | `src/alerts.rs` | Rust port — all 8 formatters, no emoji, clean structured text |
| `scripts/monitor_base.js` → HF tier thresholds | `src/constants.rs` → `HF_COLD/WARM/HOT/CRITICAL` | Values: 1.50 / 1.15 / 1.07 / 1.04 |
| `scripts/monitor_base.js` → BRIBE_* constants | `src/constants.rs` → `BRIBE_STABLE/VOLATILE/CRASH/ULTRA` | Values: 0.62 / 0.78 / 0.90 / 0.94 |
| `scripts/monitor_base.js` → Goldilocks debt range | `src/constants.rs` → `GOLDILOCKS_MIN/MAX_DEBT_USD` | $5K–$500K |
| `deployment_flash.json` → contract address | `src/constants.rs` → `LEGACY_FLASH_LIQUIDATOR` | `0xE5c3e80C243A6E21883E787013254BeAC829AD1E` |
| `artifacts/contracts/AbdulwahidFlashLiquidator.sol/...json` | `src/abi/HuntLoanFlashReceiver.json` | Copied + contractName updated |
| `scripts/monitor_base.js` → `POLL_INTERVAL: 400ms` | Replaced by WS block subscription | Event-driven, not polling |
| `scripts/monitor_base.js` → watchlist loading logic | `src/main.rs` → `load_candidates()` | Same JSON schema |
| `scripts/monitor_base.js` → `PARALLEL_CONVICTION_USD: 15_000` | `src/constants.rs` → `PARALLEL_CONVICTION_USD` | Preserved |
| `scripts/gas_strategy.js` → `MIN_WALLET_ETH: 0.005` | `src/constants.rs` → `MIN_WALLET_ETH` | Preserved |

---

## Phase 2 — Rebranding Changes

| Old name | New name | Location |
|---|---|---|
| `Huntloan` (contract) | `HuntLoanFlashReceiver` | `contracts/HuntLoanFlashReceiver.sol` |
| `IHuntloan` (Rust interface) | `IHuntLoanReceiver` | `src/executor.rs`, `src/simulator.rs` |
| `AbdulwahidFlashLiquidator.json` | `HuntLoanFlashReceiver.json` | `src/abi/` |
| `FLASH_LIQUIDATOR_V2` / `BASE_ALPHA` | `LEGACY_FLASH_LIQUIDATOR` / `LEGACY_BASE_ALPHA` | `src/constants.rs` |
| `OWNER` | `OPERATOR` | `src/constants.rs` |
| Module doc comments | Updated to "HuntLoan" system identity | All `src/*.rs` |
| `BASE_RPC_URL` env var | `RPC_URL` | `.env.example`, `src/config.rs` |
| `BASE_WS_URL` env var | `WS_RPC_URL` | `.env.example`, `src/config.rs` |
| `EXECUTOR_ADDRESS` env var | `HUNTLOAN_CONTRACT` | `.env.example`, `src/config.rs` |

---

## Phase 2 — Environment Migration Mapping

| Old variable | New variable | Notes |
|---|---|---|
| `BASE_RPC_URL` | `RPC_URL` | Legacy fallback still read |
| `BASE_WS_URL` | `WS_RPC_URL` | Legacy fallback still read |
| `PRIVATE_KEY` | `PRIVATE_KEY` | Unchanged |
| `EXECUTOR_ADDRESS` | `HUNTLOAN_CONTRACT` | Legacy fallback still read |
| `HUNTLOAN_CONTRACT` | `HUNTLOAN_CONTRACT` | Unchanged |
| `TELEGRAM_BOT_TOKEN` | `TELEGRAM_BOT_TOKEN` | Unchanged |
| `TELEGRAM_CHAT_ID` | `TELEGRAM_CHAT_ID` | Unchanged |
| `DRY_RUN` | `DRY_RUN` | Unchanged |
| `MIN_PROFIT_USD` | `MIN_PROFIT_USD` | Unchanged |
| `WATCHLIST_PATH` | `WATCHLIST_PATH` | Unchanged |
| `RUST_LOG` | `RUST_LOG` | Unchanged |
| *(new)* | `AAVE_POOL` | Override default Aave V3 Pool address |
| *(new)* | `AAVE_ADDRESSES_PROVIDER` | Override PoolAddressesProvider |
| *(new)* | `MAX_GAS_COST_WEI` | Override gas cost cap (default 0.008 ETH) |
| *(new)* | `MAX_BRIBE_WEI` | Override bribe cap (default 0.05 ETH) |

---

## Phase 2 — Execution Pipeline

Full pipeline implemented and connected:

```
WebSocket block event
  └─ HuntLoanEngine::process_block()
       ├─ 1. scanner::scan()          — IAavePool.getUserAccountData per candidate
       ├─ 2. simulator::simulate_on_chain()  — eth_call + gas estimate
       └─ 3. executor::HuntLoanExecutor::execute()
                ├─ DRY_RUN=true  → log + return
                └─ DRY_RUN=false → broadcast with retry
                     ├─ EIP-1559 fee calculation (gas.rs tier × regime)
                     ├─ Nonce management (cached + chain resync on error)
                     └─ Up to 3 retries with +15% fee bump per attempt
```

**Timing metrics logged per block:**

| Metric | Source |
|---|---|
| `scan_ms` | scanner::scan() wall time |
| `sim_ms` | simulator::simulate_on_chain() wall time |
| `exec_ms` | executor::execute() to confirmed receipt |
| `total_ms` | Full block-to-confirmation latency |

---

## Phase 3 — New Modules

| Rust module | Description | Status |
|---|---|---|
| `src/reserves.rs` | On-chain reserve resolution via Multicall3; loads Aave V3 reserve list at startup; resolves actual collateral/debt assets per borrower | Complete |
| `src/discovery.rs` | Goldsky subgraph paginator; cursor-based pagination (1000/page); rewrites watchlist JSON; triggered at boot + every 300 blocks | Complete |
| `src/velocity.rs` | HF velocity engine; linear regression ETA prediction; feeds from scanner results; drives alert tier label | Complete + 4 unit tests |

---

## Modules Implemented

| Rust module | Corresponds to (old) | Status |
|---|---|---|
| `src/constants.rs` | `ADDRS` + `CONFIG` block in `monitor_base.js` | Complete — added `asset_family_by_addr()` |
| `src/config.rs` | `process.env.*` usage across all scripts | Complete — normalized env names |
| `src/gas.rs` | `gas_strategy.js` (full) | Complete + 4 unit tests |
| `src/math.rs` | `findBestOpportunity()` profitability math | Complete + 3 unit tests |
| `src/oracle.rs` | *(new)* — Chainlink ETH/USD + Binance REST fallback | Complete + 1 unit test |
| `src/scanner.rs` | `monitor_base.js` → main scan loop | Complete — Multicall3, reserve resolution, delta-neutral filter |
| `src/reserves.rs` | *(new)* — Aave V3 reserve cache + position resolver | Complete |
| `src/simulator.rs` | *(new)* — on-chain simulation layer | Complete — eth_call + gas estimate |
| `src/executor.rs` | `execute_mev.js` → `triggerUniswapLiquidation()` | Complete — EIP-1559 + nonce + retry + parallel dual-shot |
| `src/engine.rs` | *(new)* — pipeline coordinator | Complete — WS subscription, velocity feed, Telegram alerts |
| `src/alerts.rs` | `telegram.js` | Complete — all 8 formatters, wired into engine pipeline |
| `src/discovery.rs` | `discover_risky.js` | Complete — Goldsky subgraph + periodic refresh |
| `src/velocity.rs` | *(new)* — HF velocity engine | Complete — linear regression ETA |
| `src/main.rs` | `monitor_base.js` → `main()` | Complete — delegates to HuntLoanEngine |
| `contracts/HuntLoanFlashReceiver.sol` | `contracts/AbdulwahidFlashLiquidator.sol` | Complete — 5-route swap cascade (Uniswap V3 × 3 + Aerodrome × 2) |

---

## NOT Migrated (Remaining TODOs)

| Component | Priority | Description |
|---|---|---|
| Deploy HuntLoanFlashReceiver.sol | — | ✅ Deployed `0x0A0fE1f59D56716aF5c4C9D7688df742EE5949D3` (Base mainnet, 2026-03-01) |

---

## Phase 4 — Pre-Mainnet Audit + Old System Decommission

Date: 2026-03-01

### Audit Outcome

Full internal audit completed covering:
- Flash loan flow correctness (Solidity)
- Swap cascade safety (5-route fallback, approval hygiene)
- Executor correctness (DRY_RUN guard, nonce management, retry logic)
- Risk analysis (14 financial risks, 7 operational risks, 6 smart contract risks)
- Dry run validation

**Verdict:** ⚠️ LIMITED LIVE TESTING RECOMMENDED

See `docs/PRODUCTION_READY.md` for full findings.

### Open Risk Items

| ID | Severity | Description |
|---|---|---|
| RISK-01 | Medium | Simulator hardcodes 500bps liquidation bonus — actual per-reserve bonus not plumbed in |
| RISK-02 | Low | Gas regime always `Stable` — `detect_regime()` not wired to price feed |
| RISK-03 | Low | Direct swap pairs only — multi-hop routing not supported |
| RISK-04 | Low | `hf_chunk()` in scanner.rs is dead code |

### Decommission Plan

Old system (Bitcoin-Sentinel Node.js bot) runs on DigitalOcean VPS 159.89.21.106.
It shares the same private key as HuntLoan.

**Required before DRY_RUN=false:**
1. SSH to 159.89.21.106 and run: `pm2 stop mev-bot && pm2 delete mev-bot && pm2 save --force`
2. Verify nonce stability: `cast nonce 0x3011BfD673a9D09f9761203A7fFCca757Af22587 --rpc-url $RPC_URL`
3. Archive old repo: `cp -r /root/Bitcoin-Sentinel /home/santous/archive/bitcoin-sentinel-$(date +%Y%m%d)`

Full procedure: `docs/DECOMMISSION_OLD_SYSTEM.md`

### Documentation Created (2026-03-01)

| File | Description |
|---|---|
| `README.md` | Investor-grade project overview |
| `docs/ARCHITECTURE.md` | Full module reference + pipeline diagram |
| `docs/PRODUCTION_READY.md` | Full audit report + readiness verdict |
| `docs/SAFETY_GUIDE.md` | Pre-launch checklist + monitoring + emergency stop |
| `docs/DECOMMISSION_OLD_SYSTEM.md` | Old system shutdown procedure |

---

## Key Address Reference (Base Mainnet)

| Name | Address |
|---|---|
| **HuntLoanFlashReceiver (active)** | **`0x0A0fE1f59D56716aF5c4C9D7688df742EE5949D3`** |
| Aave V3 Pool | `0xA238Dd80C259a72e81d7e4664a9801593F98d1c5` |
| Aave Data Provider | `0x2d8A3C5677189723C4cB8873CfC9C8976FDF38Ac` |
| Aave PoolAddressesProvider | `0xe20fCBdBfFC4Dd138cE8b2E6FBb6CB49777ad64D` |
| Legacy Flash Liquidator (active fallback) | `0xE5c3e80C243A6E21883E787013254BeAC829AD1E` |
| Legacy Base Alpha (capital contract) | `0xF8B715bC559032316B56cE41E7fcF7F008a5E093` |
| Uniswap V3 Router | `0x2626664c2603336E57B271c5C0b26F421741e481` |
| Aerodrome Router | `0xcF77a3Ba9A5CA399B7c97c74d54e5b1Beb874E43` |
| Multicall3 | `0xcA11bde05977b3631167028862bE2a173976CA11` |
| WETH | `0x4200000000000000000000000000000000000006` |
| USDC | `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913` |
| Operator | `0x3011BfD673a9D09f9761203A7fFCca757Af22587` |
