# Changelog

## v2.0.0 — 2026-03-02

Full server reset, contract hardening, and alerts rewrite.

### Server
- Factory reset of DigitalOcean VPS
- Created non-root user (`santous`) with sudo + SSH key auth
- SSH hardened: root login disabled, password auth off, `AllowUsers santous`, `MaxAuthTries 3`
- UFW firewall: deny all incoming, allow port 22 only
- fail2ban enabled
- Unattended security updates enabled
- PM2 process manager with systemd persistence

### Contract (HuntLoanFlashReceiver)
- Added `sweepToUsdc()` — operator-callable function to convert non-USDC profit tokens to USDC
- Fixed `totalProfit` tracking — only accumulates for USDC-denominated debt; non-USDC surplus stays in contract for later sweep
- Replaced all `approve()` calls with `forceApprove()` (SafeERC20) in swap routing
- Removed unused `_pendingDebtAsset` storage slot (was set but never read in callback)
- Removed redundant NatSpec comments (dead code cleanup)
- New contract deployed: `0x60d0C491dF2d35E4C95D98dF37897f908b04b46f`

### Alerts (v3 rewrite)
- Complete rewrite of `src/alerts.rs` (449 → 652 lines)
- 7 alert classes: boot, liquidation, execution failed, emergency stop, status report, target locked, target approaching
- Smart token name resolution for 12 Base mainnet tokens (WETH, USDC, cbBTC, wstETH, etc.)
- Human-readable error decoder — translates Aave V3 errors, contract reverts, and tx-level errors to plain English
- Session statistics tracker (`AlertStats`) with atomic counters
- Rate limiter with per-key throttling to prevent alert spam
- Clean vertical layout optimized for mobile Telegram
- 9 unit tests covering formatting, token resolution, error decoding, throttling

### Bug Fixes (P0–P3, carried from v1.x)
- P0: `debt_to_repay_raw` unit mismatch — now correctly in token atoms
- P0: Nonce race condition — `invalidate_nonce()` after every tx attempt
- P1: Daily budget reset using UTC midnight (`chrono`)
- P1: Gas regime detection with 3-tier pricing (stable/busy/crash)
- P2: Parallel reserve loading with proper cache invalidation
- P2: Dead code removal across all modules
- P3: Watchlist cache with Goldsky subgraph auto-refresh (~10 min)
- P3: `.env` loaded before tracing subscriber so `RUST_LOG` takes effect

### Tests
- 22 unit tests, all passing with zero warnings
- Coverage: alerts, gas, math, velocity, oracle

---

## v1.0.0 — 2026-03-01

Initial production deployment.

- Rust execution engine with Aave V3 flash loan liquidations
- Solidity flash receiver contract on Base mainnet
- Multicall3 batch scanning (500 addresses/call)
- Uniswap V3 + Aerodrome swap routing
- Telegram alerts for boot, execution, and errors
- DRY_RUN / LIVE mode toggle
- Velocity-based liquidation ETA prediction
- Dual-shot parallel execution
