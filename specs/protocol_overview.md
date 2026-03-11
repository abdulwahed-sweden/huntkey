# HuntKey v1.0 — Protocol Overview

## 4-Layer Architecture

### Layer 1: Root Identity (Cold)

- Derivation path: `m/999'/0'`
- Never touches the network. Signs delegation certificates off-chain.
- Authorizes action keys via `DelegationCertificate` (EIP-712 typed struct).
- Can freeze/unfreeze identities and cancel all active sessions.

### Layer 2: Action Keys (Warm)

- Derivation path: `m/999'/1'/i` (auto-incrementing index)
- Registered on-chain via `authorizeKey()` by the contract owner.
- Signs `SessionCertificate` structs to authorize ephemeral session keys.
- Scoped by function selector, target contract, value cap, and expiration.

### Layer 3: Session Keys (Ephemeral)

- Derived via HKDF-SHA256 from action key private key + nonce.
  - Salt: `HuntKey-V1-Session-Key`
  - Info: `parent_pubkey || nonce` (ensures global uniqueness)
- One-time use: burned on-chain after a single `execute()` call.
- Signs `SovereignIntent` structs binding the exact call parameters.
- All secret material implements `Zeroize` / `ZeroizeOnDrop`.

### Layer 4: Execution Gateway (On-Chain)

- `ExecutionGateway.sol` inherits `IdentityStore.sol` (abstract).
- `execute()` validates the full 3-layer chain:
  1. Session certificate signed by an authorized action key.
  2. Intent signed by the declared session key.
  3. Scope enforcement: `bytes4(callData[:4]) == intent.functionSig`.
  4. Target enforcement: `target == intent.targetContract`.
  5. CallData integrity: `keccak256(callData) == intent.callDataHash`.
  6. Identity state: must be `Active` (not `RecoveryPending` or `Frozen`).
  7. Session burn: `usedSessionKeys[session] = true`.
- Forwards the validated call with `target.call{value: msg.value}(callData)`.

## EIP-712 Type Strings

```
SovereignIntent(address targetContract,bytes4 functionSig,address recipient,address assetAddress,bytes32 callDataHash,uint128 maxValue,uint64 expiration,uint64 chainId,uint64 nonce)

DelegationCertificate(address delegate,bytes4 scope,uint128 maxValue,uint64 expiration,uint64 chainId,uint64 nonce)

SessionCertificate(address session,address parent,bytes4 scope,address target,uint128 maxValue,uint64 expiration,uint64 chainId)

RecoveryRequest(address oldRoot,address newRoot,uint64 chainId,uint64 nonce)
```

## Domain Separator

```
EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)
name    = "HuntKey"
version = "1"
```

Chain ID and verifying contract address are bound at deployment, preventing cross-chain and cross-contract replay.

## Identity State Machine

```
Active ──[initiateRecovery]──> RecoveryPending
RecoveryPending ──[cancelRecovery]──> Active
RecoveryPending ──[finalizeRecovery]──> Active (new root)
Active ──[freezeIdentity]──> Frozen
Frozen ──[unfreezeIdentity]──> Active
```

Execution via `execute()` is only permitted when identity state is `Active`.

## Session Epoch

`cancelAllSessions(root)` increments `sessionEpoch[root]`, allowing the root or owner to invalidate all existing session certificates in a single transaction if a breach is suspected.
