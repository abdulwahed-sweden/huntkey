# HuntKey: The Sovereign Execution Layer

**No blind signing. No long-lived execution keys. Recoverable identity.**

HuntKey is a policy-enforced identity protocol where the master key never touches the network. Ephemeral session keys handle constrained on-chain operations, verified through a 4-layer defense-in-depth architecture with EIP-712 typed structured data signing.

## How It Works

```
BIP-39 Mnemonic
  └─ Root Identity (m/999'/0') ── cold storage, signs delegation certs
       ├─ Action Key (m/999'/1'/i) ── warm, scoped by selector + value cap
       │    └─ Session Key (HKDF) ── ephemeral, one-time use, burned after execute()
       │         └─ Signs SovereignIntent ── exact calldata hash binding
       │              └─ ExecutionGateway.execute() ── on-chain policy firewall
       └─ Recovery Keys (m/999'/3'/i) ── 2-of-N guardians + 48h timelock
```

Every transaction passes through 15 validation checks before the gateway forwards the call. A single failed check reverts the entire operation.

## Architecture

| Layer | Component | Purpose |
|-------|-----------|---------|
| 1 | Root Identity | Cold key. Signs delegation certificates. Never on-chain. |
| 2 | Action Keys | Warm keys. Scoped by function selector, value cap, expiration. |
| 3 | Session Keys | HKDF-SHA256 derived. One-time use. Chain-isolated. |
| 4 | ExecutionGateway | On-chain policy firewall. Validates full 3-layer chain. |

See [ARCHITECTURE.md](ARCHITECTURE.md) for the complete technical specification.

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
├── lib.rs                 Crate root, re-exports, 34 integration tests
├── core/mod.rs            Key derivation, keccak256, ABI encoding
├── intents/mod.rs         SovereignIntent, DelegationCertificate, EIP-712
├── sessions/mod.rs        SessionKey (HKDF-SHA256), session certificates
├── recovery/mod.rs        RecoveryRequest, PendingRecovery, guardian signing
└── wasm_api/mod.rs        WASM bindings (feature: "wasm")

contracts/
├── src/IdentityStore.sol       Identity state, delegation, social recovery
├── src/ExecutionGateway.sol    Session validation, scope enforcement, execution
└── test/PolicyGuard.t.sol      31 Solidity tests

sdk/ts/src/
└── index.ts               TypeScript SDK (MnemonicManager, IntentSigner, SessionManager)

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

### Run Rust Tests (34 tests)

```bash
cargo test
```

Covers: key derivation, EIP-712 hash determinism, sign/recover roundtrips, delegation chain verification, recovery threshold/timelock, session key HKDF derivation, cross-chain isolation, and full end-to-end protocol flow. Includes property-based tests via proptest.

### Run Solidity Tests (31 tests)

```bash
cd contracts
forge test -vv
```

Covers: direct intent validation, delegated verification, social recovery (initiation, support, cancellation, finalization, timelock), execution gateway (happy path, one-time use, selector/target/calldata mismatch, session expiry, unauthorized parent, value caps, calldata mutation, recovery-blocked execution), domain version verification, and event emission.

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
| [ARCHITECTURE.md](ARCHITECTURE.md) | 4-layer defense model, state machine, key hierarchy rationale |
| [specs/protocol_overview.md](specs/protocol_overview.md) | EIP-712 type strings, domain separator, session epoch |
| [specs/threat_model.md](specs/threat_model.md) | 10 attack vectors with mitigations |
| [specs/key_hierarchy.md](specs/key_hierarchy.md) | BIP-32/44 paths, HKDF-SHA256 derivation, zeroize policy |
| [specs/invariants.md](specs/invariants.md) | 4 formal protocol invariants |

## License

MIT
