# HuntKey v1.1 — Formal Invariants

## INV-1: Session Key Single-Use

```
∀ sessionKey ∈ address:
  usedSessionKeys[sessionKey] == true
  ⟹ no future call to execute() with session.session == sessionKey can succeed
```

Once `execute()` marks `usedSessionKeys[session] = true`, any subsequent `execute()` call referencing the same session address reverts with `SessionKeyAlreadyUsed()`. This holds regardless of nonce, expiration, or parent key — the session address itself is permanently burned.

## INV-2: Identity State Gate

```
∀ root ∈ address:
  identityState[root] ≠ Active
  ⟹ execute(session{parent: root}, ...) reverts with IdentityNotActive()
```

The `execute()` function checks `identityState[session.parent]` before any signature verification. If the identity is `RecoveryPending` (set by `initiateRecovery`) or `Frozen` (set by `freezeIdentity`), all execution is blocked. This prevents an attacker from draining funds during a recovery window or after a freeze.

## INV-3: CallData Hash Binding

```
∀ execute(session, intent, target, callData):
  keccak256(callData) ≠ intent.callDataHash
  ⟹ revert CalldataHashMismatch()
```

The session key signs `intent.callDataHash = keccak256(callData)` off-chain. The gateway recomputes `keccak256(callData)` on-chain and verifies equality. Any single-byte mutation in the submitted calldata produces a different hash, causing revert. This prevents calldata-mutation drain attacks where an attacker intercepts a valid signature and swaps the recipient address.

## INV-4: Session Epoch Invalidation

```
∀ root ∈ address:
  cancelAllSessions(root) increments sessionEpoch[root]
  ⟹ all previously issued SessionCertificates for that root are logically void
```

While the current implementation burns session keys individually via `usedSessionKeys`, the `sessionEpoch` provides a mass-invalidation mechanism. When `cancelAllSessions(root)` is called, `sessionEpoch[root]++` increments the epoch counter, and the `SessionInvalidated` event signals off-chain systems to discard all certificates issued under the previous epoch. This enables instant breach response: a single transaction invalidates all active sessions for a compromised identity.
