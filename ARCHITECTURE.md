# HuntKey Architecture

## Defense-in-Depth: 4-Layer Execution Model

HuntKey enforces a strict separation of authority across four layers. Each layer constrains the next, and no single layer compromise can drain funds or hijack identity.

```
Layer 1: Identity        Cold storage. Signs delegation certificates. Never on-chain.
Layer 2: Delegation      Scoped authority. Binds action keys to selectors + value caps.
Layer 3: Ephemeral       HKDF-derived session keys. One-time use. Burned after execute().
Layer 4: Execution       On-chain policy firewall. Validates the full 3-layer chain.
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

### Layer 4 — Execution (On-Chain Policy Firewall)

`ExecutionGateway.sol` inherits `IdentityStore.sol` and validates the complete chain before forwarding any call:

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

All validation failures revert with gas-efficient custom errors (no string storage).

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

- **Active**: Normal operation. `execute()` proceeds.
- **RecoveryPending**: Set by `initiateRecovery()`. All `execute()` calls revert. The original root can cancel at any time.
- **Frozen**: Set by `freezeIdentity()`. All `execute()` calls revert. Only the owner can unfreeze.

`cancelAllSessions(root)` increments `sessionEpoch[root]`, providing a mass-invalidation mechanism that logically voids all active session certificates in a single transaction.

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
├── lib.rs              Crate root, re-exports, integration tests
├── core/mod.rs         DerivedKey, key derivation, keccak256, ABI encoding
├── intents/mod.rs      SovereignIntent, DelegationCertificate, EIP-712 signing
├── sessions/mod.rs     SessionKey (Zeroize), HKDF derivation, session certs
├── recovery/mod.rs     RecoveryRequest, PendingRecovery, guardian signing
└── wasm_api/mod.rs     WASM bindings (feature-gated: "wasm")

contracts/
├── src/IdentityStore.sol      Identity state, delegation, recovery (abstract)
├── src/ExecutionGateway.sol   Session validation, scope enforcement, execution
└── test/PolicyGuard.t.sol     31 tests across all protocol layers
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
