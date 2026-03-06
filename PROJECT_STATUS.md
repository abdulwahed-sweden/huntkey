# HuntLoan — Project Status

## What This Is
Aave V3 flash-loan liquidation bot for Base mainnet.
Rust engine + Solidity smart contract (HuntLoanFlashReceiver).

## Current State: PAUSED
- Bot stopped, VPS destroyed
- Code preserved on GitHub
- Contract deployed but inactive: 0x60d0C491dF2d35E4C95D98dF37897f908b04b46f (Base mainnet)
- Operator wallet: 0x3011BfD673a9D09f9761203A7fFCca757Af22587

## To Resume
1. Provision new VPS (Ubuntu 24, 1GB RAM minimum)
2. Install Rust + PM2
3. Clone repo, restore .env from backup
4. cargo build --release
5. pm2 start target/release/huntloan
6. Fund wallet with 0.1+ ETH on Base

## Architecture
WebSocket block sub → Scanner (44K candidates) → Hotlist (HF < 2.0) → Simulator → Executor → Flash loan callback

## Key Stats
- 26 passing tests, 0 warnings
- ~14 Rust source files, ~4500 LOC
- 1 Solidity contract (verified on BaseScan)
- Wolfpack dynamic bidding (3-tier)
- RBF escalation (40% → 60% → 80% → 90%)
- Crash-mode hotset scanning
