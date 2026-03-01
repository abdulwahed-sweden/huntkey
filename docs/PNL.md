# HuntLoan — PnL Accounting Guide

## How Profit Is Estimated

HuntLoan cannot directly observe its wallet balance change from flash liquidations
because profits flow through the smart contract (`totalProfit` accumulates there,
not in the wallet). The following method constructs a best-effort PnL estimate.

---

## Data Sources

### 1. `logs/trades.csv` (primary)

Every confirmed broadcast appends one row. Columns:

| Column | Type | Description |
|---|---|---|
| `timestamp` | ISO-8601 UTC | When the tx was confirmed |
| `tx_hash` | hex | On-chain tx identifier |
| `target` | address | Liquidated borrower |
| `debt_asset` | address | Token borrowed (repaid via flash loan) |
| `collateral_asset` | address | Token seized as collateral |
| `debt_usd` | u128 | Debt USD value at scan time |
| `sim_net_profit_usd` | i128 | **Simulated net profit** (see formula below) |
| `estimated_gas` | u64 | Gas units estimated by eth_estimateGas |
| `gas_used` | u64 | Actual gas units from receipt |
| `base_fee_wei` | u128 | Block base fee at execution time |
| `bribe_wei` | u128 | Priority fee paid |
| `block_number` | u64 | Confirmed block |
| `status` | u8 | 1 = success, 0 = on-chain revert |
| `scan_ms` | u128 | Stage 1 latency |
| `sim_ms` | u128 | Simulation latency |
| `exec_ms` | u128 | Tx send → receipt latency |

### 2. BaseScan / cast (ground truth)

```bash
# On-chain accumulated profit in the contract:
cast call 0x0A0fE1f59D56716aF5c4C9D7688df742EE5949D3 \
  "totalProfit()(uint256)" \
  --rpc-url $RPC_URL

# Operator wallet ETH balance:
cast balance 0x3011BfD673a9D09f9761203A7fFCca757Af22587 --rpc-url $RPC_URL --ether
```

---

## Profit Formula

### Simulated net profit (stored in CSV)

```
gross_usd   = debt_to_repay × liquidation_bonus_bps / 10_000
flash_fee   = debt_to_repay × 5 / 10_000   (0.05% Aave V3 flash premium)
gas_cost    = estimated_gas × base_fee_wei × eth_price_usd / 1e18
net_profit  = gross_usd - flash_fee - gas_cost
```

This is an **estimate** computed before execution. It uses:
- `estimated_gas` (typically 10-20% higher than actual `gas_used`)
- `eth_price_usd` from oracle at time of simulation
- No slippage buffer (actual swap may lose 0.01%-0.3% to V3 fee tier)

### Actual gas cost (computable from CSV)

```python
actual_gas_cost_wei = gas_used * base_fee_wei + bribe_wei
actual_gas_cost_usd = actual_gas_cost_wei / 1e18 * eth_price_at_time
```

`eth_price_at_time` is not stored in CSV — use oracle data or a historical price API.

### Conservative net estimate

Since `gas_used < estimated_gas` in practice (~15-25% lower), the actual profit
tends to be **higher** than `sim_net_profit_usd` by the gas overestimate margin:

```
actual_net ≈ sim_net_profit_usd + (estimated_gas - gas_used) × base_fee_wei × eth_price / 1e18
```

---

## Session Summary (Telegram 📊)

The hourly summary fires automatically and reports:

- **Net PnL**: sum of `sim_net_profit_usd` for all confirmed txs this session
- **Gas cost**: sum of `gas_used × base_fee_wei` across confirmed txs (in gwei, displayed as approx USD)
- **Win rate**: `execs_succeeded / execs_attempted`
- **Top revert reasons**: from `AlertStats.revert_reasons`

---

## CSV Analysis Examples

```bash
# Total simulated profit (status=1 rows only):
awk -F, '$13==1 {sum += $7} END {print "Sim net USD:", sum}' logs/trades.csv

# Average gas used per tx:
awk -F, 'NR>1 && $13==1 {sum += $9; n++} END {print "Avg gas used:", sum/n}' logs/trades.csv

# Efficiency: gas overestimate margin
awk -F, 'NR>1 && $13==1 {sum += ($8-$9)/$8; n++} END {printf "Gas overestimate: %.1f%%\n", sum/n*100}' logs/trades.csv

# Total txs by status:
awk -F, 'NR>1 {print $13}' logs/trades.csv | sort | uniq -c
```

---

## Reconciliation

To reconcile simulated vs actual PnL:

1. Query `cast call ... "totalProfit()(uint256)"` before and after a session.
2. Convert from token units (USDbC has 6 decimals, WETH has 18).
3. Compare delta to sum of `sim_net_profit_usd` for that session.
4. Difference = slippage + oracle price drift + gas overestimate margin.

In practice the contract's `totalProfit` is the ground truth.
