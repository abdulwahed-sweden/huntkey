# HuntKey User Flow

Identity -> Delegation -> Session -> Intent -> Execution

## 1. Identity Setup

The user generates a BIP-39 mnemonic. From the mnemonic, the key hierarchy derives four role-specific keys:

```
BIP-39 Mnemonic
  |
  v
Root Identity (m/999'/0')     -- cold storage, never touches the network
  ├── Action Key (m/999'/1'/i)  -- warm key, signs delegation + session certs
  ├── Proof Key (m/999'/2'/i)   -- reserved for ZK proof generation
  └── Recovery Key (m/999'/3'/i) -- guardian key for social recovery
```

The Root Identity address is the on-chain identity anchor. It is registered in `IdentityStore.sol` and never used for transaction signing directly.

## 2. Delegation

The Root signs an EIP-712 `DelegationCertificate` off-chain, authorizing an Action Key for a scoped set of operations:

```
DelegationCertificate {
  delegate:   action key address
  scope:      bytes4 function selector
  maxValue:   uint128 wei cap per intent
  expiration: uint64 unix timestamp
  chainId:    uint64
  nonce:      uint64 replay protection
}
```

The signed delegation is submitted to `IdentityStore.endorseDelegation()`, which recovers the root address, verifies it owns the identity, and marks the action key as `authorizedKeys[delegate] = true`.

The delegation is chain-bound (`chainId`) and replay-protected (`nonce`). Each delegation is a one-time endorsement -- revoking it removes the action key from `authorizedKeys`.

## 3. Session Key Derivation

The Action Key derives ephemeral session keys using HKDF-SHA256:

```
HKDF-SHA256(
  ikm:  action_key_private_bytes
  salt: keccak256(action_key_address)
  info: "huntkey-session-v1" || chain_id (8 bytes BE) || nonce (8 bytes BE)
)
```

Each session key is unique per (action key, chain, nonce) tuple. Cross-chain isolation is enforced by including `chain_id` in the HKDF info string -- the same action key + nonce on different networks produces different session keys.

The Action Key then signs an EIP-712 `SessionCertificate` binding the session to a specific scope:

```
SessionCertificate {
  session:    session key address
  parent:     action key address (signer)
  scope:      bytes4 function selector
  target:     address target contract
  maxValue:   uint128 wei cap
  expiration: uint64 unix timestamp
  chainId:    uint64
}
```

Session keys are ephemeral -- designed for single-use execution, then discarded. All key material implements `Zeroize`/`ZeroizeOnDrop`.

## 4. Intent Signing

The session key signs a `SovereignIntent` -- a fully constrained description of the on-chain action:

```
SovereignIntent v2.2 {
  targetContract:      address
  functionSig:         bytes4
  recipient:           address
  assetAddress:        address
  callDataHash:        bytes32    -- keccak256(calldata)
  maxValue:            uint128
  expiration:          uint64
  chainId:             uint64
  nonce:               uint64
  sessionEpoch:        uint64     -- must match on-chain sessionEpoch[root]
  gasLimit:            uint64
  maxFeePerGas:        uint128
  maxPriorityFeePerGas: uint128   -- anti-siphoning: binds bundler tip
  requiredClaim:       bytes32    -- credential binding (zero = none)
}
```

Key properties:

- **callDataHash** binds the exact calldata bytes. Any single-byte mutation causes on-chain revert.
- **sessionEpoch** enables mass invalidation. Incrementing the root's epoch instantly voids all outstanding intents without per-key revocation.
- **maxPriorityFeePerGas** binds the bundler tip, preventing gas siphoning attacks where a malicious bundler inflates the priority fee to extract value from the user's gas deposit.
- **requiredClaim** gates execution on verifiable credentials stored in the `userClaims` mapping.

The intent is signed using EIP-712 typed structured data (`\x19\x01 || domainSeparator || structHash`).

## 5. Execution

### Direct Execution (ExecutionGateway)

The signed 3-layer chain is submitted to `ExecutionGateway.execute()`:

```
execute(
  SessionParams {
    session, parent, scope, target, maxValue, expiration, chainId,
    certV, certR, certS              -- session cert signature
  },
  IntentParams {
    targetContract, functionSig, recipient, assetAddress,
    callDataHash, maxValue, expiration, chainId, nonce,
    sessionEpoch, gasLimit, maxFeePerGas, requiredClaim,
    v, r, s                          -- intent signature
  },
  target,                            -- execution target address
  callData                           -- raw calldata bytes
)
```

The gateway validates 15 checks in sequence:

| # | Check | Revert |
|---|-------|--------|
| 1 | Session cert signature recovers to `parent` | InvalidSessionSignature |
| 2 | `parent` is in `authorizedKeys` | UnauthorizedKey |
| 3 | Session cert not expired | SessionExpired |
| 4 | Value within session cert cap | SessionValueExceeded |
| 5 | Intent signature recovers to `session` | InvalidIntentSignature |
| 6 | Intent not expired | IntentExpired |
| 7 | Value within intent cap | ValueExceedsCap |
| 8 | Intent nonce matches `nonces[session]` | InvalidNonce |
| 9 | `sessionEpoch` matches `sessionEpoch[parent]` | SessionEpochMismatch |
| 10 | Target address matches intent | TargetMismatch |
| 11 | Function selector matches intent | SelectorMismatch |
| 12 | `keccak256(callData) == intent.callDataHash` | CallDataMismatch |
| 13 | Identity not in RecoveryPending | RecoveryBlocksExecution |
| 14 | Required claim satisfied (or zero) | ClaimRequired |
| 15 | External call succeeds | ExecutionFailed |

On success: nonce incremented, call forwarded to target, `IntentExecuted` event emitted.

### ERC-4337 Execution (HuntKeyAccount)

For Account Abstraction, the 3-layer chain is packed into `UserOperation.signature`:

```
UserOperation.signature = abi.encode(SessionParams, IntentParams)
```

`HuntKeyAccount.validateUserOp()` performs the same validation chain but returns packed `validationData` instead of reverting:

```
validationData = authorizer (160 bits) | validUntil (48 bits) | validAfter (48 bits)

Success: _packValidationData(false, uint48(session.expiration), 0)
Failure: _packValidationData(true, 0, 0)
```

**Recovery Exception:** During `RecoveryPending`, all UserOps are blocked with `RecoveryBlocksUserOp` revert -- except recovery management functions (`cancelRecovery`, `supportRecovery`, `finalizeRecovery`), which bypass the 3-layer chain validation since they enforce their own authorization.

### Multicall Execution

`executeMulticall()` supports batched calls bound to a single intent:

```
executeMulticall(SessionParams, IntentParams, Call[] calls)
  where intent.callDataHash == keccak256(abi.encode(calls))
```

Each call in the batch is executed sequentially. The batch is atomic -- any single call failure reverts the entire operation.

## 6. Mass Invalidation

To invalidate all outstanding sessions and intents for an identity:

```solidity
IdentityStore.incrementSessionEpoch(rootAddress)
```

This increments `sessionEpoch[root]`. All intents signed with the previous epoch will fail the `SessionEpochMismatch` check. No per-key revocation needed.

## 7. Identity Monitoring

The `IdentityWatcher` (Rust) monitors on-chain events and generates security alerts:

| Event | Alert Level | Action |
|-------|-------------|--------|
| Recovery by known guardian | Warning | Notify all guardians |
| Recovery by unknown guardian | Critical | Notify all guardians |
| Identity frozen | Warning | Notify all guardians |
| Unknown delegation endorsed | Warning | Log |
| Sessions mass-invalidated | Warning | Log |
| Offline session detected | Critical | Log |
| High-value intent (above threshold) | Warning | Notify all guardians |

Guardian notifications use a drain-based pattern -- a consumer (push notification service, webhook dispatcher) calls `drain_notifications()` periodically to deliver alerts.

## 8. Social Recovery

If the root key is lost:

1. A guardian calls `initiateRecovery(identity, newRoot)` -- starts 48-hour timelock
2. Additional guardians call `supportRecovery(identity, ...)` -- 2-of-N threshold
3. After timelock: anyone calls `finalizeRecovery(identity)` -- root transferred
4. During timelock: original root can call `cancelRecovery(identity)` to abort

During `RecoveryPending`, all execution is blocked except recovery management functions.
