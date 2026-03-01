# HuntLoan

Automated Aave V3 flash-loan liquidation engine for Base mainnet.

**Stack:** Rust (execution engine) + Solidity (on-chain flash receiver)
**Network:** Base Mainnet (Chain ID 8453)
**Contract:** `0x0A0fE1f59D56716aF5c4C9D7688df742EE5949D3`

---

## What It Does

1. **Monitors** Aave V3 borrowing positions via Multicall3 (500 addresses per RPC call)
2. **Detects** undercollateralised positions (Health Factor < 1.0)
3. **Simulates** liquidation profitability via `eth_call` before any broadcast
4. **Executes** flash-loan liquidations atomically — borrow, liquidate, swap, repay
5. **Alerts** via Telegram at every stage of the pipeline

No pre-positioned capital required. Each liquidation is self-funded within a single atomic transaction.

---

## Architecture

```
WebSocket block header
  └─► scanner (Multicall3 batch)
        └─► simulator (eth_call)
              └─► executor (EIP-1559 tx → Base)
                    └─► HuntLoanFlashReceiver.sol
                          └─► Aave V3 flashLoanSimple → liquidate → swap → repay
```

See `docs/ARCHITECTURE.md` for the full module reference and design decisions.

---

## Quickstart

### Prerequisites

- Rust 1.78+ (`rustup update stable`)
- An Alchemy (or compatible) Base mainnet RPC with WebSocket support
- A funded operator wallet (> 0.1 ETH on Base for gas)

### Setup

```bash
git clone <repo>
cd huntloan

# Copy and fill in your credentials
cp .env.example .env
# Edit .env — set RPC_URL, WS_RPC_URL, PRIVATE_KEY, HUNTLOAN_CONTRACT

# Build
cargo build --release
```

### Run (dry mode — safe, no transactions sent)

```bash
DRY_RUN=true cargo run --release
```

### Run (live mode)

**Read `docs/SAFETY_GUIDE.md` first — mandatory pre-launch checklist.**

```bash
DRY_RUN=false cargo run --release
```

---

## Environment Variables

```bash
# Network
RPC_URL=https://base-mainnet.g.alchemy.com/v2/YOUR_KEY
WS_RPC_URL=wss://base-mainnet.g.alchemy.com/v2/YOUR_KEY

# Wallet
PRIVATE_KEY=0x...

# Contracts
HUNTLOAN_CONTRACT=0x0A0fE1f59D56716aF5c4C9D7688df742EE5949D3

# Bot settings
DRY_RUN=true                    # Set to false for live execution
MIN_PROFIT_USD=10               # Minimum net profit to attempt
WATCHLIST_PATH=watchlist.json   # Path to borrower watchlist

# Optional — MEV protection (recommended for live)
PRIVATE_RPC_URL=https://...

# Alerts
TELEGRAM_BOT_TOKEN=...
TELEGRAM_CHAT_ID=...
```

---

## Tests

```bash
cargo test
```

12 unit tests across `gas.rs`, `math.rs`, `velocity.rs`, `oracle.rs`.

---

## Watchlist

The engine watches borrowers listed in `watchlist.json`:

```json
[
  "0xabc...",
  "0xdef..."
]
```

On startup and every ~10 minutes, the engine queries the Goldsky subgraph to refresh
this list with current active borrowers from Aave V3 Base.

To seed manually:
```bash
echo '["0x..."]' > watchlist.json
```

---

## Key Addresses (Base Mainnet)

| Contract | Address |
|---|---|
| HuntLoanFlashReceiver | `0x0A0fE1f59D56716aF5c4C9D7688df742EE5949D3` |
| Aave V3 Pool | `0xA238Dd80C259a72e81d7e4664a9801593F98d1c5` |
| Uniswap V3 Router | `0x2626664c2603336E57B271c5C0b26F421741e481` |
| Aerodrome Router | `0xcF77a3Ba9A5CA399B7c97c74d54e5b1Beb874E43` |
| Multicall3 | `0xcA11bde05977b3631167028862bE2a173976CA11` |

---

## Documentation

| Doc | Description |
|---|---|
| `docs/ARCHITECTURE.md` | Full module reference, pipeline diagram, design decisions |
| `docs/PRODUCTION_READY.md` | Pre-mainnet audit report, risk analysis, readiness verdict |
| `docs/SAFETY_GUIDE.md` | Mandatory pre-launch checklist, monitoring, emergency stop |
| `docs/DECOMMISSION_OLD_SYSTEM.md` | Safe shutdown procedure for legacy Bitcoin-Sentinel bot |
| `MIGRATION_REPORT.md` | Full migration log from Node.js to Rust |

---

## Profit Distribution

The HuntLoanFlashReceiver contract accumulates profit in USDC.
After 6 months (maturity), `settle()` distributes:
- **Financier:** capital recovery + 60% of net profit
- **Operator:** 40% of net profit

Check accumulated profit:
```bash
cast call 0x0A0fE1f59D56716aF5c4C9D7688df742EE5949D3 \
  "totalProfit()(uint256)" --rpc-url $RPC_URL
```
