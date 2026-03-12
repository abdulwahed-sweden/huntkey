# ERC-4337 Paymaster

## Overview

HuntKeyPaymaster implements the ERC-4337 `IPaymaster` interface, supporting three gas payment modes:

| Mode | Name | Description |
|------|------|-------------|
| 0 | `SelfFunded` | No paymaster; user pays gas directly |
| 1 | `Sponsored` | Paymaster sponsors gas in ETH from its EntryPoint deposit |
| 2 | `TokenPay` | User pays gas equivalent in ERC20 tokens post-execution |

## Architecture

```
UserOperation.paymasterAndData
  = [paymaster address (20 bytes)] [mode (1 byte)] [token address (20 bytes, mode 2 only)]

EntryPoint
  ├── validatePaymasterUserOp() → context, validationData
  └── postOp() → collect tokens (mode 2) or no-op (mode 1)
```

## Intent Binding

SovereignIntent v2.3 binds the paymaster into the EIP-712 signed data:

```
SovereignIntent {
  ...
  paymasterMode: uint8     -- 0, 1, or 2
  paymaster:     address   -- paymaster contract (zero if self-funded)
}
```

This prevents:
- **Mode downgrade attacks**: Changing from sponsored to self-funded after signing
- **Paymaster substitution**: Swapping to a malicious paymaster that extracts value

## Contracts

### HuntKeyPaymaster.sol

- `setSponsoredAccount(address, bool)` — approve/revoke sponsorship
- `setTokenGasPrice(address token, uint256 pricePerGas)` — configure ERC20 gas rates
- `validatePaymasterUserOp(...)` — validates mode and sponsorship status
- `postOp(...)` — collects ERC20 tokens for mode 2
- `deposit()` / `withdraw(to, amount)` / `getDeposit()` — deposit management

### IPaymaster.sol

Standard ERC-4337 paymaster interface with `validatePaymasterUserOp()` and `postOp()`.

## SDK Integration

```typescript
import { PaymasterClient, PaymasterMode } from "@huntkey/sdk";

const pm = new PaymasterClient(provider, paymasterAddress);

// Check sponsorship
const sponsored = await pm.isSponsored(account);

// Build paymasterAndData for a UserOp
const pmData = pm.buildPaymasterAndData(PaymasterMode.SPONSORED);
const pmDataToken = pm.buildPaymasterAndData(PaymasterMode.TOKEN_PAY, tokenAddress);

// Check paymaster deposit
const deposit = await pm.getDeposit();
```

## Token Payment Flow

1. Paymaster owner calls `setTokenGasPrice(token, price)`
2. User approves paymaster to spend tokens: `token.approve(paymaster, amount)`
3. User submits UserOp with `paymasterAndData = [paymaster][0x02][token]`
4. `validatePaymasterUserOp` verifies token is configured, returns context
5. After execution, `postOp` transfers `(actualGasCost * tokenGasPrice) / 1e18` tokens from user to paymaster
