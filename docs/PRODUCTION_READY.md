# HuntLoan — Pre-Mainnet Audit & Production Readiness Report

**Audit Date:** 2026-03-01
**Auditor:** Internal (HuntLoan team)
**System version:** v1.0.0
**Network:** Base Mainnet (Chain ID 8453)
**Contract:** `0x0A0fE1f59D56716aF5c4C9D7688df742EE5949D3`

---

## Phase 1 — Flash Loan Flow Audit

### 1.1 Flash Loan Entry

**Function:** `HuntLoanFlashReceiver::requestFlashLiquidation()`

**Findings:**

| Check | Result | Notes |
|---|---|---|
| Caller restriction | PASS | `msg.sender != operator → revert OnlyOperator()` |
| Settlement guard | PASS | `if (settled) revert ContractSettled()` |
| Context storage | PASS | `_pendingDebtAsset / _pendingCollateralAsset / _pendingBorrower` set before `flashLoanSimple()` |
| Context cleared | PASS | All three set to `address(0)` at end of `executeOperation()` |
| Flash loan amount | PASS | Amount passed by Rust engine = `debt_to_repay = total_debt / 2` (Aave max per call) |

### 1.2 Flash Loan Callback

**Function:** `HuntLoanFlashReceiver::executeOperation()`

**Findings:**

| Check | Result | Notes |
|---|---|---|
| Pool-only gate | PASS | `msg.sender != address(POOL) → revert OnlyAavePool()` |
| Liquidation call | PASS | `POOL.liquidationCall(coll, asset, borrower, amount, false)` — receivePToken=false |
| Collateral accounting | PASS | `collBefore` snapshot before, `collSeized = balance - collBefore` after |
| Owed amount | PASS | `owed = amount + premium` — Aave V3 premium is 0.05% (`9` in bps) |
| Swap enforces minOut | PASS | `minAmountOut = owed` passed to `_swapCollateralToDebt` |
| Safety revert | PASS | `if (received < owed) revert LiquidationUnprofitable(...)` |
| Repayment approval | PASS | `approve(POOL, owed)` before returning `true` |
| Profit accumulation | PASS | `totalProfit += profit` — only runs after all safety checks pass |
| Return value | PASS | Returns `true` — required by Aave V3 `IFlashLoanSimpleReceiver` |

### 1.3 Swap Cascade

**Function:** `_swapCollateralToDebt()`

**Findings:**

| Check | Result | Notes |
|---|---|---|
| 5-route fallback | PASS | Uniswap V3 × 3 → Aerodrome volatile → Aerodrome stable |
| Approval hygiene | PASS | Approval reset to 0 on success and after all tiers fail |
| Slippage protection | PASS | `amountOutMinimum = owed` on every route (hard floor = loan repayment) |
| Deadline | PASS | Aerodrome: `block.timestamp + 120` seconds |
| SwapFailed revert | PASS | All routes exhausted → `revert SwapFailed(...)` — full tx reverts, loan not taken |
| Direct pool interaction | NOTE | No intermediate hops — only direct token pairs. Multi-hop not supported. |

**Known limitation:** If the collateral/debt pair has no direct Uniswap V3 or Aerodrome pool,
all 5 routes fail and `SwapFailed` is thrown. This is caught by `eth_call` simulation
(Stage 2) before broadcast, so no gas is wasted in practice.

---

## Phase 2 — Risk Analysis

### 2.1 Financial Risks

| Risk | Severity | Likelihood | Mitigation |
|---|---|---|---|
| Flash loan reverts → lose gas | Medium | Medium | eth_call simulation catches reverts before broadcast |
| Swap slippage exceeds bonus | High | Low | `minAmountOut = owed` — tx reverts, loan not taken |
| Liquidation bonus hardcoded 500bps in simulator | Medium | Medium | Actual bonus from reserve_cache passed to Opportunity; simulator uses 500bps approximation — may overestimate profit by 0–2% |
| Front-running by competitors | High | High | Private RPC URL reduces MEV exposure; parallel dual-shot raises capture probability |
| Gas cost spike | Medium | Medium | `MAX_GAS_COST_WEI = 0.008 ETH` hard cap; `validate_caps()` blocks if exceeded |
| Wallet ETH runs dry | Critical | Low | `MIN_WALLET_ETH = 0.005 ETH` floor check; maintain > 0.1 ETH recommended |
| Oracle stale / manipulated | High | Very Low | Chainlink + Binance fallback; used for profit math only, not for security |
| Multi-hop needed, not supported | Medium | Low | eth_call simulation will revert — caught before broadcast |

### 2.2 Operational Risks

| Risk | Severity | Mitigation |
|---|---|---|
| Old system (Bitcoin-Sentinel) still running | **Critical** | See `docs/DECOMMISSION_OLD_SYSTEM.md` — must be stopped first |
| Shared private key between old/new system | **Critical** | Same as above |
| DRY_RUN=false without decommission | **Critical** | Pre-flight gate in `SAFETY_GUIDE.md` |
| Nonce conflict between dual-shot and retry | Low | Nonce cache bumped by 2 atomically before any send |
| WS disconnection silently stops engine | Medium | Engine logs warning and exits — must be monitored with a process supervisor |
| Watchlist stale (no Goldsky refresh) | Low | Boot refresh + 300-block periodic refresh |
| Telegram bot rate limit | Negligible | Per-borrower 5-min throttle on critical alerts |

### 2.3 Smart Contract Risks

| Risk | Severity | Finding |
|---|---|---|
| Reentrancy via Aave callback | None | Flash loan is linear: Aave calls back exactly once |
| Reentrancy via swap routes | None | No ETH-valued calls; ERC-20 only |
| Ownership griefing | None | `onlyOwner` only on `rescueToken`; operator is owner |
| Unauthorized liquidation trigger | None | `OnlyOperator` guard on entry point |
| `settle()` callable prematurely | None | `block.timestamp >= maturityTime` check (180 days) |
| Profit accounting overflow | None | Solidity 0.8.x checked arithmetic |
| Token approval residue | None | All approvals reset to 0 after each route attempt |

### 2.4 Open Issues

| ID | Severity | Description | Status |
|---|---|---|---|
| RISK-01 | ~~Medium~~ | ~~Simulator uses hardcoded 500bps liquidation bonus instead of per-reserve value~~ | **FIXED 2026-03-01** — `simulator.rs` now uses `opp.liquidation_bonus_bps` from `ReserveCache`. Regression test `math::tests::test_bonus_bps_is_not_hardcoded` added (13/13 pass). |
| RISK-02 | Low | Gas regime always set to `Stable` (executor.rs, engine.rs) — `detect_regime()` is implemented but not called | Open — conservative, errs on side of lower fees |
| RISK-03 | Low | `_swapCollateralToDebt` only supports direct pairs — no multi-hop routing | Known limitation, acceptable for current asset universe |
| RISK-04 | Low | `hf_chunk()` function in scanner.rs is dead code (replaced by `hf_chunk_full`) | Low priority, no functional impact |

---

## Phase 3 — Dry Run Validation

### 3.1 DRY_RUN Guard (executor.rs)

```rust
// executor.rs:execute()
if self.config.dry_run {
    info!(mode = "DRY_RUN", borrower = %opp.borrower, ..., "DRY_RUN — tx NOT sent");
    return Ok(ExecutionResult {
        tx_hash: TxHash::ZERO, block_number: 0, gas_used: 0, ...
    });
}

// executor.rs:execute_parallel()
if self.config.dry_run {
    info!(mode = "DRY_RUN", ..., "DRY_RUN — parallel dual-shot NOT sent");
    return (None, None);
}
```

**Validation:** Both execution paths have an early `dry_run` return at the top of the function,
before any provider is constructed or wallet is touched. No transaction can be sent when
`DRY_RUN=true`.

### 3.2 Expected Log Pattern in DRY_RUN Mode

```
[HuntLoanEngine] Pipeline active — DRY_RUN=true
INFO scan complete candidates=150 opportunities=2
INFO Best opportunity selected borrower=0x... hf=0.97 net_profit_usd=234
INFO DRY_RUN — tx NOT sent borrower=0x...
```

The `tx_hash = TxHash::ZERO` sentinel in the returned `ExecutionResult` ensures the engine
logs a successful "execution" with a zero hash, making dry-run activity clearly identifiable
in logs without any external state change.

---

## Phase 4 — Final Readiness Verdict

### Pre-Flight Checklist

| Gate | Status | Action Required |
|---|---|---|
| Old system decommissioned | **REQUIRED** | See `docs/DECOMMISSION_OLD_SYSTEM.md` |
| `.env DRY_RUN=true` | PASS | Currently set |
| `.env HUNTLOAN_CONTRACT` set | PASS | `0x0A0fE1f59D56716aF5c4C9D7688df742EE5949D3` |
| Operator wallet has ETH | CHECK | Verify > 0.1 ETH at `0x3011BfD673a9D09f9761203A7fFCca757Af22587` |
| `cargo test` passes | PASS | 12/12 tests |
| `cargo build --release` compiles | PASS | Zero errors |
| Watchlist populated | CHECK | Run once in dry mode to verify Goldsky fetch works |
| Telegram alerts working | CHECK | Set `TELEGRAM_BOT_TOKEN` and `TELEGRAM_CHAT_ID` |
| Private RPC configured | RECOMMENDED | Set `PRIVATE_RPC_URL` for MEV protection |

### Verdict

```
⚠️  LIMITED LIVE TESTING RECOMMENDED before full production deployment

Current state:
  - DRY_RUN=true (safe — no transactions sent)
  - Contract deployed and verified on Base mainnet
  - All 12 unit tests pass
  - Pipeline compiles clean

Blockers for DRY_RUN=false:
  1. [CRITICAL] Decommission old Bitcoin-Sentinel system (shared private key)
  2. [REQUIRED]  Verify operator wallet ETH balance
  3. [RECOMMENDED] Run 24h in DRY_RUN=true to validate scanner/simulator output

Recommended path to production:
  1. Complete Phase 0 decommission (1 hour)
  2. Run 24h dry-run, review logs (1 day)
  3. Switch DRY_RUN=false with small watchlist (10–20 addresses)
  4. Monitor first 48h in live mode
  5. Expand watchlist to full Goldsky dataset
```

---

## Phase 5 — Test Coverage Summary

| Module | Tests | Status |
|---|---|---|
| `gas.rs` | 4 | PASS — tier ordering, regime multipliers, tier selection, bribe cap |
| `math.rs` | 3 | PASS — profitability, flash fee, gas cost |
| `velocity.rs` | 4 | PASS — record/ETA, falling HF, rising HF (no ETA), GC |
| `oracle.rs` | 1 | PASS — fallback path (Chainlink call expected to fail in test env) |
| `scanner.rs` | 0 | Integration test — requires live RPC |
| `simulator.rs` | 0 | Integration test — requires live RPC |
| `executor.rs` | 0 | Integration test — requires live RPC |
| **Total** | **12** | **12/12 PASS** |
