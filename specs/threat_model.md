# HuntKey v1.0 — Threat Model

## Threat → Mitigation Matrix

### 1. Signature Replay

**Attack:** Reuse a valid signature on the same or different chain/contract.

**Mitigations:**
- Per-signer nonce (`nonces[signer]++`) consumed on every validated intent.
- Per-prover nonce (`delegationNonces[prover]++`) for delegation certificates.
- Per-root recovery nonce (`recoveryNonces[root]++`) incremented after finalization.
- Chain ID binding in every EIP-712 struct hash (included in domain separator and explicit in typed structs).
- `DOMAIN_SEPARATOR` is immutable, bound to `block.chainid` + `address(this)` at deployment.

### 2. Signature Malleability

**Attack:** Flip `s` to `N - s` and adjust `v` to produce an alternate valid signature for the same message.

**Mitigations:**
- `_validateSigParams(v, s)` enforces `s <= SECP256K1_N / 2` (low-s normalization).
- `v` must be exactly 27 or 28.
- Applied to all signature recovery paths: intent, delegation, recovery, session.

### 3. Calldata Mutation (Drain Attack)

**Attack:** Attacker intercepts a valid session+intent pair and modifies the calldata (e.g., changes recipient address) before submitting to the Gateway.

**Mitigations:**
- `intent.callDataHash = keccak256(callData)` is signed by the session key.
- Gateway computes `keccak256(callData)` on-chain and verifies it matches `intent.callDataHash`.
- Any single-byte change in calldata produces a completely different hash, causing revert.

### 4. Scope Escalation

**Attack:** Use a session key authorized for `transfer()` to call `approve()` or another function.

**Mitigations:**
- Session certificate binds `scope` (bytes4 function selector) and `target` (address).
- Gateway enforces: `bytes4(callData[:4]) == intent.functionSig == session.scope`.
- Gateway enforces: `target == intent.targetContract == session.target`.

### 5. Session Key Reuse

**Attack:** Use the same ephemeral session key for multiple transactions.

**Mitigations:**
- `usedSessionKeys[session.session]` is set to `true` after the first `execute()`.
- Subsequent calls with the same session address revert with "session key already used".
- Even with a different nonce, the session address itself is burned.

### 6. Unauthorized Delegation

**Attack:** Forge a session certificate from an unauthorized parent key.

**Mitigations:**
- Session certificate signer is recovered via `ecrecover` and must match `session.parent`.
- `authorizedKeys[sessionSigner]` must be `true`.
- Rogue keys fail the authorization check.

### 7. Recovery Hijack

**Attack:** Malicious guardians initiate recovery to steal an identity.

**Mitigations:**
- 2-of-N threshold (minimum 3 guardians, up to 5).
- 48-hour timelock between threshold met and finalization.
- Original root can cancel recovery at any time during the window via `cancelRecovery()`.
- Identity state transitions to `RecoveryPending` on initiation, blocking all `execute()` calls.
- Recovery nonce prevents replay of old guardian signatures after finalization.

### 8. Breach Response

**Attack:** Action key or session key compromise detected.

**Mitigations:**
- `cancelAllSessions(root)` increments session epoch, invalidating all active sessions in one tx.
- `revokeKey(key)` removes authorization from compromised action keys.
- `freezeIdentity(root)` sets identity to `Frozen`, blocking all execution.
- Root identity (cold storage) is never exposed on-chain.

### 9. Cross-Chain Replay

**Attack:** Submit a signature valid on chain A to chain B.

**Mitigations:**
- `chainId` is a field in every EIP-712 typed struct (Intent, Delegation, Session, Recovery).
- `DOMAIN_SEPARATOR` includes `block.chainid` at deployment.
- Session certificate explicitly checks `session.chainId == uint64(block.chainid)`.

### 10. Value Extraction

**Attack:** Drain funds by submitting intents with inflated value parameters.

**Mitigations:**
- Intent `maxValue` caps the `msg.value` at the intent level.
- Session `maxValue` caps the intent `maxValue` at the session level.
- Delegation `maxValue` caps the intent value at the delegation level.
- All three layers enforce: `msg.value <= intent.maxValue <= session.maxValue`.
