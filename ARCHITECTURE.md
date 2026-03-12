# HuntKey Architecture

## HuntKey: The Intent-Based Account Abstraction Protocol

HuntKey is a policy-enforced identity protocol that combines ERC-4337 Account Abstraction with a 4-layer defense-in-depth execution model. The master key never touches the network — ephemeral session keys handle constrained operations, verified through typed structured data signing and on-chain policy enforcement.

## Defense-in-Depth: 4-Layer Execution Model

HuntKey enforces a strict separation of authority across four layers. Each layer constrains the next, and no single layer compromise can drain funds or hijack identity.

```
Layer 1: Identity        Cold storage. Signs delegation certificates. Never on-chain.
Layer 2: Delegation      Scoped authority. Binds action keys to selectors + value caps.
Layer 3: Ephemeral       HKDF-derived session keys. One-time use. Burned after execute().
Layer 4: Execution       On-chain policy firewall + ERC-4337 Account Abstraction.
```

### Layer 1 — Identity (Cold)

The Root Identity key lives at derivation path `m/999'/0'`. It never touches the network. Its sole purpose is to sign `DelegationCertificate` structs off-chain, authorizing action keys for constrained operations.

**Recovery keys** (`m/999'/3'/i`) serve as guardians for social recovery. A 2-of-N threshold with a 48-hour timelock protects against guardian collusion.

Key properties:
- Root key is never transmitted on-chain or exposed to any network interface.
- Recovery requires 2-of-N guardian signatures + 48h timelock.
- The original root can cancel recovery at any point during the window.
- All key material implements `Zeroize` and `ZeroizeOnDrop`.

### Layer 2 — Delegation (Warm)

Action keys (`m/999'/1'/i`) are registered on-chain via `authorizeKey()`. The root endorses each action key with a `DelegationCertificate` containing:

| Field | Purpose |
|-------|---------|
| `delegate` | Action key address |
| `scope` | bytes4 function selector |
| `maxValue` | Wei cap per intent |
| `expiration` | Unix timestamp |
| `chainId` | Cross-chain replay prevention |
| `nonce` | Per-prover replay prevention |

The delegation certificate is an EIP-712 typed struct. The contract recovers the signer via `ecrecover` and verifies it matches a registered prover.

### Layer 3 — Ephemeral (Session Keys)

Session keys are derived deterministically from an action key using HKDF-SHA256:

```
HKDF-SHA256(
    IKM:  action_private_key (32 bytes)
    Salt: "HuntKey-V1-Session-Key"
    Info: parent_compressed_pubkey (33 bytes)
          || nonce (8 bytes BE)
          || chain_id (8 bytes BE)
)
```

**Why HKDF-SHA256 over BIP-32 child derivation:**
- BIP-32 public child derivation leaks the parent public key if a child private key is compromised. HKDF has no such algebraic relationship.
- HKDF info binding includes `chain_id`, ensuring the same action key + nonce produces different session keys on different networks. This provides absolute cross-chain context isolation.
- Session keys are burned on-chain after a single `execute()` call. The one-time-use property eliminates the need for nonce management at the session level.

Session keys sign `SovereignIntent` structs that bind:
- Target contract and function selector
- Recipient and asset address
- `callDataHash` — keccak256 of the exact calldata, verified on-chain
- Value cap and expiration
- Chain ID and nonce
- Gas limit and max fee per gas (ERC-4337)
- Required credential claim (bytes32)

### Layer 4 — Execution (On-Chain Policy Firewall + AA)

The execution layer provides two entry paths:

#### Direct Execution (`ExecutionGateway.execute()`)

Inherits `IdentityStore.sol` and validates the complete chain before forwarding any call:

```
execute(session, intent, target, callData):
  1. Identity state == Active         (blocks RecoveryPending / Frozen)
  2. Session certificate not expired  (block.timestamp <= session.expiration)
  3. Chain ID matches                 (session.chainId == block.chainid)
  4. Session signer is authorized     (authorizedKeys[signer] == true)
  5. Signer matches declared parent   (signer == session.parent)
  6. Session key not reused           (usedSessionKeys[session] == false)
  7. Intent not expired               (block.timestamp <= intent.expiration)
  8. Value within cap                 (msg.value <= intent.maxValue)
  9. Intent signed by session key     (ecrecover == session.session)
 10. Scope enforcement                (intent.functionSig == session.scope)
 11. Target enforcement               (target == intent.targetContract)
 12. Selector enforcement             (callData[:4] == intent.functionSig)
 13. CallData integrity               (keccak256(callData) == intent.callDataHash)
 14. Value bounds                     (intent.maxValue <= session.maxValue)
 15. Forward call                     (target.call{value}(callData))
```

#### ERC-4337 Account Abstraction (`HuntKeyAccount.validateUserOp()`)

`HuntKeyAccount.sol` inherits `ExecutionGateway` and implements the ERC-4337 `IAccount` interface. The `UserOperation.signature` field carries the 3-layer chain:

```
signature = abi.encode(SessionParams, IntentParams)
```

`validateUserOp` performs the same validation as `execute()` but returns a validation result instead of forwarding the call. The EntryPoint then calls the account's execution function.

Additional v2.0 validation:
- **Identity state gate**: `RecoveryPending` triggers `RecoveryBlocksUserOp` revert (hard block, not soft fail)
- **Credential/claim check**: `checkClaim(parent, intent.requiredClaim)` — reverts if required claim not held
- **Pre-funding**: Automatically transfers `missingAccountFunds` to the EntryPoint

#### Multicall Execution (`HuntKeyAccount.executeMulticall()`)

Supports batched calls with calldata hash verification across the entire batch:

```
executeMulticall(session, intent, calls[]):
  - Standard 3-layer chain validation
  - intent.callDataHash == keccak256(abi.encode(calls))
  - Each call target must match intent.targetContract
  - Credential/claim check enforced
```

All validation failures revert with gas-efficient custom errors (no string storage).

## SovereignIntent v2.3

The v2.3 intent struct includes gas parameters, credential hooks, ZK claims, and paymaster binding:

| Field | Type | Purpose |
|-------|------|---------|
| `targetContract` | address | Target contract for the call |
| `functionSig` | bytes4 | Function selector |
| `recipient` | address | Operation recipient |
| `assetAddress` | address | Asset contract (zero for native ETH) |
| `callDataHash` | bytes32 | keccak256 of exact calldata |
| `maxValue` | uint128 | Wei cap for this intent |
| `expiration` | uint64 | Unix timestamp expiry |
| `chainId` | uint64 | Chain binding |
| `nonce` | uint64 | Per-signer replay prevention |
| `sessionEpoch` | uint64 | Must match on-chain epoch (mass invalidation) |
| `gasLimit` | uint64 | Gas limit for ERC-4337 UserOp |
| `maxFeePerGas` | uint128 | Max fee per gas unit (wei) |
| `maxPriorityFeePerGas` | uint128 | Max priority fee (anti-siphoning) |
| `requiredClaim` | bytes32 | Required credential (zero = none) |
| `claimProofHash` | bytes32 | ZK proof hash binding (zero = none) |
| `paymasterMode` | uint8 | 0=self-funded, 1=sponsored, 2=token |
| `paymaster` | address | Paymaster contract (zero = none) |

## Credential/Claim System

HuntKeyAccount supports a credential hook for gating operations on verifiable claims:

- `userClaims[account][claim]` — mapping of granted credentials
- `setClaim(account, claim, value)` — owner-managed credential assignment
- `checkClaim(account, claim)` — returns true if `claim == bytes32(0)` or if the account holds the claim
- When `intent.requiredClaim != bytes32(0)`, both `validateUserOp` and `executeMulticall` enforce the claim check

This enables compliance-gated operations (e.g., KYC verification) without modifying the core signing chain.

## Identity State Machine

```
                 initiateRecovery()
    Active ─────────────────────────> RecoveryPending
      ^                                     │
      │        cancelRecovery()             │
      ├─────────────────────────────────────┘
      │        finalizeRecovery()           │
      ├─────────────────────────────────────┘
      │
      │        freezeIdentity()
      └─────────────────────────> Frozen
      ^                              │
      │        unfreezeIdentity()    │
      └──────────────────────────────┘
```

- **Active**: Normal operation. `execute()` and `validateUserOp()` proceed.
- **RecoveryPending**: Set by `initiateRecovery()`. All `execute()` calls revert. `validateUserOp()` reverts with `RecoveryBlocksUserOp`. The original root can cancel at any time.
- **Frozen**: Set by `freezeIdentity()`. All `execute()` calls revert. Only the owner can unfreeze.

`cancelAllSessions(root)` increments `sessionEpoch[root]`, providing a mass-invalidation mechanism that logically voids all active session certificates in a single transaction.

## Identity Monitoring

The `monitor` module (`src/monitor/mod.rs`) provides an `IdentityWatcher` that tracks on-chain events and generates security alerts:

| Event | Alert Severity | Condition |
|-------|---------------|-----------|
| `RecoveryStateChanged` → RecoveryPending | Critical | Unknown guardian |
| `RecoveryStateChanged` → RecoveryPending | Warning | Known guardian |
| `RecoveryStateChanged` → Frozen | Warning | Always |
| `DelegationEndorsed` | Warning | Unknown delegate |
| `DelegationEndorsed` | Info | Known delegate |
| `SessionInvalidated` | Warning | Always |
| `IntentExecuted` | Info | Always |
| Offline session detected | Critical | Session used without prior registration |

The watcher supports filtering by severity, identity, and category. In production, it integrates with event subscription systems (ethers-rs, alloy) and notification services.

## Key Hierarchy

```
BIP-39 Mnemonic (12/24 words)
  └─ BIP-32 Root (XPriv)
       ├─ m/44'/0'/0'/0/{i}   Bitcoin (legacy compatibility)
       ├─ m/44'/60'/0'/0/{i}  Ethereum (legacy compatibility)
       └─ m/999'              Sovereign Identity Namespace
            ├─ 0'             Root Identity (single key, cold storage)
            ├─ 1'/{i}         Action Keys (auto-incrementing, warm)
            ├─ 2'/{i}         Proof Keys (reserved for ZK)
            └─ 3'/{i}         Recovery Keys (guardian operations)
```

All paths under `m/999'` use hardened derivation, isolating the sovereign identity namespace from standard BIP-44 derivations.

## Zeroize Policy

All private key material implements `Zeroize` and/or `ZeroizeOnDrop`:
- `DerivedKey.private_key: Vec<u8>` — zeroed on drop
- `SessionKey.private_key: [u8; 32]` — zeroed on drop
- All signing functions wrap key bytes in `Zeroizing<[u8; 32]>` and explicitly zeroize after use
- HKDF output key material (`okm`) is zeroized after constructing the signing key
- HKDF info buffer is manually zeroed before deallocation

## Module Structure

```
src/
├── lib.rs              Crate root, re-exports, 60 integration tests
├── core/mod.rs         DerivedKey, key derivation, keccak256, ABI encoding
├── intents/mod.rs      SovereignIntent (v2.3), DelegationCertificate, EIP-712, UserOperationBuilder
├── sessions/mod.rs     SessionKey (Zeroize), HKDF derivation, session certs
├── recovery/mod.rs     RecoveryRequest, PendingRecovery, guardian signing
├── monitor/mod.rs      IdentityWatcher, SecurityAlert, EventLog, DashboardState
└── wasm_api/mod.rs     WASM bindings (feature-gated: "wasm")

contracts/
├── src/IdentityStore.sol      Identity state, delegation, social recovery
├── src/ExecutionGateway.sol   Session validation, scope enforcement, execution
├── src/IAccount.sol           ERC-4337 IAccount interface
├── src/HuntKeyAccount.sol     ERC-4337 account + claims + multicall
├── src/ClaimVerifier.sol      ZK claim commitment verification
├── src/IPaymaster.sol         ERC-4337 IPaymaster interface
├── src/HuntKeyPaymaster.sol   Paymaster: sponsored + ERC20 token payment
└── test/PolicyGuard.t.sol     66 tests across all protocol layers

sdk/ts/src/
└── index.ts               TypeScript SDK (MnemonicManager, IntentSigner, SessionManager,
                           ProtocolAuditor, ClaimManager, PaymasterClient, ProtocolDashboard)

specs/
├── protocol_overview.md   4-layer architecture, EIP-712 types, state machine
├── threat_model.md        10 threats mapped to mitigations
├── key_hierarchy.md       Derivation paths, HKDF spec, trust chain
└── invariants.md          4 formal invariants
```

## Threat Model Summary

| Threat | Mitigation |
|--------|-----------|
| Signature replay | Per-signer nonce, per-prover nonce, per-root recovery nonce |
| Signature malleability | Low-s normalization (`s <= N/2`), `v` must be 27 or 28 |
| Calldata mutation | `keccak256(callData)` verified on-chain against signed hash |
| Scope escalation | Session cert binds `scope` (selector) and `target` (address) |
| Session key reuse | `usedSessionKeys[session] = true` after first `execute()` |
| Cross-chain replay | `chainId` in every EIP-712 struct + domain separator |
| Recovery hijack | 2-of-N threshold + 48h timelock + root cancellation |
| Key compromise | `cancelAllSessions()`, `revokeKey()`, `freezeIdentity()` |
| Unauthorized UserOp | 3-layer chain packed in signature, validated by `validateUserOp` |
| Credential bypass | `requiredClaim` check enforced before execution and validation |
| Multicall mutation | `keccak256(abi.encode(calls))` verified against intent `callDataHash` |
| Gas siphoning | `maxPriorityFeePerGas` signed into EIP-712 struct, binds bundler tip |
| Claim proof reuse | `usedProofs[proofHash]` prevents replay; `claimProofHash` binding in intent |
| Paymaster substitution | `paymasterMode` + `paymaster` signed into EIP-712 struct |
| Mode downgrade | Paymaster mode bound in signed intent; cannot change after signing |
