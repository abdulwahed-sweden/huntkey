# HuntKey: The Intent-Based Account Abstraction Protocol

**No blind signing. No long-lived execution keys. Recoverable identity.**

HuntKey is a policy-enforced identity protocol that combines ERC-4337 Account Abstraction with a 4-layer defense-in-depth execution model. The master key never touches the network — ephemeral session keys handle constrained operations, verified through typed structured data signing and on-chain policy enforcement.

## How It Works

```
BIP-39 Mnemonic
  └─ Root Identity (m/999'/0') ── cold storage, signs delegation certs
       ├─ Action Key (m/999'/1'/i) ── warm, scoped by selector + value cap
       │    └─ Session Key (HKDF) ── ephemeral, one-time use, burned after execute()
       │         └─ Signs SovereignIntent v2.0 ── calldata hash + gas params + credential binding
       │              ├─ ExecutionGateway.execute() ── direct on-chain policy firewall
       │              └─ HuntKeyAccount.validateUserOp() ── ERC-4337 Account Abstraction
       └─ Recovery Keys (m/999'/3'/i) ── 2-of-N guardians + 48h timelock
```

Every transaction passes through 15 validation checks before the gateway forwards the call. A single failed check reverts the entire operation. ERC-4337 UserOps carry the full 3-layer signing chain packed into the `signature` field.

## Architecture

| Layer | Component | Purpose |
|-------|-----------|---------|
| 1 | Root Identity | Cold key. Signs delegation certificates. Never on-chain. |
| 2 | Action Keys | Warm keys. Scoped by function selector, value cap, expiration. |
| 3 | Session Keys | HKDF-SHA256 derived. One-time use. Chain-isolated. |
| 4 | Execution | On-chain policy firewall + ERC-4337 Account Abstraction. |

See [ARCHITECTURE.md](ARCHITECTURE.md) for the complete technical specification.

## v2.0 Features

- **ERC-4337 Account Abstraction** — `HuntKeyAccount.sol` implements `IAccount.validateUserOp()`. The 3-layer signing chain (delegation cert + session cert + intent signature) is packed into `UserOperation.signature` as `abi.encode(SessionParams, IntentParams)`.
- **Multicall Execution** — `executeMulticall()` supports batched calls with `keccak256(abi.encode(calls))` hash verification across the entire batch, bound to the intent's `callDataHash`.
- **Credential/Claim System** — `requiredClaim` field in SovereignIntent gates operations on verifiable claims. `userClaims` mapping with `bytes32(0)` bypass for unrestricted operations.
- **Identity Monitoring** — `IdentityWatcher` tracks on-chain events and generates security alerts at Info/Warning/Critical severity. Detects unknown guardian recovery, unauthorized delegation, and offline session issuance.
- **Gas Parameter Binding** — `gasLimit` and `maxFeePerGas` fields in SovereignIntent v2.0, signed into the EIP-712 struct for ERC-4337 integration.
- **Session Epoch Mass Invalidation** — `sessionEpoch` field in SovereignIntent v2.1 must match on-chain `sessionEpoch[root]`. Incrementing the epoch instantly invalidates all outstanding sessions and intents without per-key revocation.
- **Recovery-Gated UserOps** — During `RecoveryPending`, `validateUserOp` blocks all operations except recovery management (`cancelRecovery`, `supportRecovery`, `finalizeRecovery`). Returns packed `validationData` (authorizer | validUntil | validAfter) per ERC-4337 spec.
- **Guardian Notifications** — `IdentityWatcher` generates real-time `GuardianNotification` alerts for recovery events and high-value intent execution above configurable thresholds. Drain-based consumption pattern for async notification services.
- **UserOperation Builder** — Rust builder pattern for constructing ERC-4337 v0.7+ `PackedUserOperation` with 3-layer signature chain packed into the `signature` field.

## Security Properties

- **No blind signing** — Every intent binds `keccak256(callData)`. The gateway verifies the hash on-chain. Any single-byte mutation in calldata causes revert.
- **No long-lived execution keys** — Session keys are burned after a single `execute()`. No key reuse, no nonce management at the session level.
- **Recoverable identity** — 2-of-N guardian threshold with 48-hour timelock. The root can cancel recovery at any point during the window.
- **Cross-chain isolation** — HKDF info includes `chain_id`. Same action key + nonce on different networks produces different session keys.
- **Signature malleability protection** — Low-s normalization (`s <= N/2`) on all signature paths.
- **Zeroize everywhere** — All private key material implements `Zeroize`/`ZeroizeOnDrop`. HKDF outputs, signing keys, and info buffers are explicitly zeroed.

## Project Structure

```
src/
├── lib.rs                 Crate root, re-exports, 48 integration tests
├── core/mod.rs            Key derivation, keccak256, ABI encoding
├── intents/mod.rs         SovereignIntent v2.0, DelegationCertificate, EIP-712
├── sessions/mod.rs        SessionKey (HKDF-SHA256), session certificates
├── recovery/mod.rs        RecoveryRequest, PendingRecovery, guardian signing
├── monitor/mod.rs         IdentityWatcher, SecurityAlert, event tracking
└── wasm_api/mod.rs        WASM bindings (feature: "wasm")

contracts/
├── src/IdentityStore.sol      Identity state, delegation, social recovery
├── src/ExecutionGateway.sol   Session validation, scope enforcement, execution
├── src/IAccount.sol           ERC-4337 IAccount interface
├── src/HuntKeyAccount.sol     ERC-4337 account + claims + multicall
└── test/PolicyGuard.t.sol     45 Solidity tests

sdk/ts/src/
└── index.ts               TypeScript SDK (MnemonicManager, IntentSigner, SessionManager)

examples/
└── client-demo.ts         dApp integration demo with full signing flow

specs/
├── protocol_overview.md   4-layer architecture, EIP-712 types, state machine
├── threat_model.md        10 threats mapped to mitigations
├── key_hierarchy.md       Derivation paths, HKDF spec, trust chain
└── invariants.md          4 formal invariants
```

## Quick Start

### Run the Protocol Demo

```bash
cargo run
```

Outputs the full protocol flow: mnemonic generation, key hierarchy, delegation certificates, session keys, and 3-layer signing chain verification.

### Run Rust Tests (48 tests)

```bash
cargo test
```

Covers: key derivation, EIP-712 hash determinism, sign/recover roundtrips, delegation chain verification, recovery threshold/timelock, session key HKDF derivation, cross-chain isolation, identity monitoring alerts, guardian notifications, high-value intent detection, UserOperation builder packing, and full end-to-end protocol flow. Includes property-based tests via proptest.

### Run Solidity Tests (45 tests)

```bash
cd contracts
forge test -vv
```

Covers: direct intent validation, delegated verification, social recovery, execution gateway (happy path, one-time use, selector/target/calldata mismatch, session expiry, value caps, calldata mutation, recovery-blocked execution), ERC-4337 validateUserOp (3-layer chain, recovery management exception, packed validationData, EntryPoint gating, pre-funding), session epoch enforcement, credential/claim checks, and multicall hash verification.

### Build WASM SDK

```bash
cargo build --features wasm --target wasm32-unknown-unknown
```

## Requirements

- [Rust](https://rustup.rs/) 1.85+ (edition 2024)
- [Foundry](https://getfoundry.sh/) (Solidity 0.8.28)
- [wasm-pack](https://rustwasm.github.io/wasm-pack/) (optional, for WASM builds)

## Specifications

| Document | Contents |
|----------|----------|
| [docs/USER_FLOW.md](docs/USER_FLOW.md) | Identity -> Delegation -> Session -> Intent -> Execution flow |
| [ARCHITECTURE.md](ARCHITECTURE.md) | 4-layer defense model, AA integration, state machine, key hierarchy |
| [specs/protocol_overview.md](specs/protocol_overview.md) | EIP-712 type strings, domain separator, session epoch |
| [specs/threat_model.md](specs/threat_model.md) | 10 attack vectors with mitigations |
| [specs/key_hierarchy.md](specs/key_hierarchy.md) | BIP-32/44 paths, HKDF-SHA256 derivation, zeroize policy |
| [specs/invariants.md](specs/invariants.md) | 4 formal protocol invariants |

## License

MIT
