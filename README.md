# HuntLoan

Aave V3 flash-loan liquidation bot for Base mainnet.

Monitors 44,000+ borrowing positions, detects undercollateralised accounts, simulates profitability, and executes atomic flash-loan liquidations — all within a single block.

**Stack:** Rust (execution engine) + Solidity (on-chain flash receiver)
**Network:** Base Mainnet (Chain ID 8453)
**Contract:** [`0x60d0C491dF2d35E4C95D98dF37897f908b04b46f`](https://basescan.org/address/0x60d0C491dF2d35E4C95D98dF37897f908b04b46f)

---

## Architecture

```
WebSocket block header
  └─► Scanner (Multicall3 batch — 500 addresses/call)
        └─► Simulator (eth_call profitability check)
              └─► Executor (EIP-1559 tx → Base)
                    └─► HuntLoanFlashReceiver.sol
                          └─► Aave V3 flashLoanSimple → liquidate → swap → repay
```

No pre-positioned capital required. Each liquidation is self-funded within a single atomic transaction via Aave V3 flash loans.

---

## Key Features

- **Velocity-based ETA prediction** — tracks health factor trajectory to anticipate liquidations before they happen
- **Dual-shot parallel execution** — submits two gas tiers simultaneously for time-critical opportunities
- **Regime-aware gas pricing** — adjusts gas strategy based on network congestion (stable / busy / crash)
- **Multi-DEX swap routing** — Uniswap V3 (3 fee tiers) + Aerodrome (volatile + stable) with automatic fallback
- **Circuit breaker** — halts execution on repeated failures, sends emergency Telegram alert
- **7-class Telegram alerts** — boot, liquidation, execution failed, emergency stop, status report, target locked, target approaching
- **Smart token resolution** — displays human-readable token names (WETH, USDC, cbBTC) instead of hex addresses
- **Human-readable error decoding** — translates Aave/contract revert codes into plain English
- **Watchlist auto-refresh** — pulls ~45K active borrowers from Goldsky subgraph every ~10 minutes

---

## Source Layout

```
src/
  main.rs          — entry point, config loading, boot alert
  engine.rs        — main orchestration loop (block → scan → simulate → execute)
  scanner.rs       — Multicall3 batch scanning
  simulator.rs     — eth_call profitability simulation
  executor.rs      — EIP-1559 transaction broadcast
  alerts.rs        — Telegram notifications (v3 — 7 alert classes)
  config.rs        — environment config loading
  constants.rs     — addresses and constants
  discovery.rs     — Goldsky subgraph watchlist refresh
  gas.rs           — regime-aware gas estimation
  math.rs          — profit math, fixed-point arithmetic
  oracle.rs        — price oracle with REST fallback
  reserves.rs      — Aave V3 reserve data cache
  trades.rs        — trade record keeping, daily budget reset
  velocity.rs      — health factor velocity tracking + ETA prediction
  abi/             — ABI definitions

contracts/
  HuntLoanFlashReceiver.sol  — on-chain flash receiver + swap routing + profit tracking

script/
  Deploy.s.sol               — Foundry deployment script
```

---

## Smart Contract

**HuntLoanFlashReceiver** handles the on-chain execution:

1. Receives flash loan from Aave V3
2. Executes `liquidationCall` on the underwater position
3. Swaps seized collateral back to debt token via best available DEX route
4. Repays flash loan + 0.05% premium
5. Keeps surplus as profit

Additional features:
- `sweepToUsdc()` — operator converts accumulated non-USDC tokens to USDC
- `settle()` — distributes profits after 6-month maturity (60% financier / 40% operator)
- `rescueToken()` — emergency token recovery (owner only)
- ReentrancyGuard on all entry points
- `forceApprove` (SafeERC20) on all token approvals

---

## Quick Start

### Prerequisites

- Rust 1.85+ (`rustup update stable`)
- Base mainnet RPC with WebSocket support (Alchemy, Infura, etc.)
- Funded operator wallet (ETH on Base for gas)

### Build

```bash
git clone git@github.com:abdulwahed-sweden/huntloan.git
cd huntloan
cargo build --release
```

### Configure

Create `.env` (see Environment Variables below), then:

```bash
echo '[]' > watchlist.json
chmod 600 .env
```

### Run

```bash
# Dry run — monitors and simulates, never sends transactions
DRY_RUN=true ./target/release/huntloan

# Production — with PM2
pm2 start ecosystem.config.js
pm2 save && pm2 startup
```

---

## Operating Modes

| Mode | Behavior |
|---|---|
| `DRY_RUN=true` | Scans and simulates only. No transactions sent. Safe for testing. |
| `SOFT_LIVE=true` | Sends transactions but with extra conservative thresholds. |
| `DRY_RUN=false` | Full live execution. Sends real transactions on Base mainnet. |

Recommended progression: DRY_RUN → SOFT_LIVE → LIVE.

---

## Environment Variables

```bash
# Network
RPC_URL=                    # Base mainnet HTTPS RPC endpoint
WS_RPC_URL=                 # Base mainnet WebSocket RPC endpoint

# Wallet
PRIVATE_KEY=                # Operator wallet private key (hex, with 0x prefix)
OPERATOR_ADDRESS=           # Operator wallet address

# Contracts
HUNTLOAN_CONTRACT=          # Deployed HuntLoanFlashReceiver address
AAVE_POOL=                  # Aave V3 Pool address on Base
AAVE_ADDRESSES_PROVIDER=    # Aave V3 PoolAddressesProvider address

# Telegram Alerts
TELEGRAM_BOT_TOKEN=         # Telegram bot token from @BotFather
TELEGRAM_CHAT_ID=           # Telegram chat ID for alerts

# Bot Settings
DRY_RUN=true                # true = simulate only, false = live execution
WATCHLIST_PATH=watchlist.json
MIN_PROFIT_USD=10           # Minimum net profit (USD) to attempt liquidation
MAX_GAS_COST_WEI=8000000000000000   # Max gas cost per tx (wei)
MAX_BRIBE_WEI=50000000000000000     # Max priority fee for MEV protection

# Logging
RUST_LOG=huntloan=info      # Tracing filter
```

---

## Key Addresses (Base Mainnet)

| Contract | Address |
|---|---|
| HuntLoanFlashReceiver | `0x60d0C491dF2d35E4C95D98dF37897f908b04b46f` |
| Aave V3 Pool | `0xA238Dd80C259a72e81d7e4664a9801593F98d1c5` |
| Uniswap V3 Router | `0x2626664c2603336E57B271c5C0b26F421741e481` |
| Aerodrome Router | `0xcF77a3Ba9A5CA399B7c97c74d54e5b1Beb874E43` |
| Multicall3 | `0xcA11bde05977b3631167028862bE2a173976CA11` |

---

## Tests

```bash
cargo test
```

22 unit tests across `alerts.rs`, `gas.rs`, `math.rs`, `velocity.rs`, `oracle.rs`.

---

## Deployment

Production runs on a hardened Ubuntu VPS:
- Non-root user with sudo
- SSH key-only auth, root login disabled
- UFW firewall (port 22 only)
- fail2ban active
- PM2 process manager with auto-restart
- `.env` file with `chmod 600`

---

## License

MIT
