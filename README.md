<p align="center">
  <h1 align="center">HuntKey</h1>
  <p align="center"><strong>The Intent-Based Sovereign Smart Account Protocol</strong></p>
  <p align="center">
    No blind signing. No long-lived execution keys. Recoverable identity.
  </p>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Rust-1.85+-orange?logo=rust" alt="Rust 1.85+">
  <img src="https://img.shields.io/badge/Solidity-0.8.28-363636?logo=solidity" alt="Solidity 0.8.28">
  <img src="https://img.shields.io/badge/ERC--4337-Account%20Abstraction-3C3C3D?logo=ethereum" alt="ERC-4337">
  <img src="https://img.shields.io/badge/Tests-126%20passing-brightgreen" alt="Tests Passing">
  <img src="https://img.shields.io/badge/License-MIT-blue" alt="License MIT">
  <img src="https://img.shields.io/badge/Security-Defense%20in%20Depth-critical" alt="Security Focused">
</p>

---

HuntKey is a policy-enforced identity protocol that combines ERC-4337 Account Abstraction with a 4-layer defense-in-depth execution model. Your master key never touches the network — ephemeral session keys handle constrained operations, verified through typed structured data signing and on-chain policy enforcement.

Every transaction is bound to its exact calldata hash, scoped by function selector and value cap, and validated through 15 on-chain checks before execution. A single failed check reverts everything.

---

## The Problem

Traditional wallets ask you to sign raw transaction bytes with the same key that controls your entire on-chain identity. One compromised key — one phishing signature — and everything is gone. There is no scope restriction, no expiration, no policy enforcement, and no recovery path.

Account abstraction improved the execution model but left the signing model unchanged. Users still blindly approve opaque payloads with long-lived keys that have unrestricted access.

**HuntKey eliminates this entire class of risk.** The root identity key stays in cold storage. Ephemeral session keys — deterministically derived, single-use, chain-isolated — handle every operation within cryptographically enforced constraints. If a session key is compromised, the blast radius is one transaction, one selector, one value cap, on one chain.

---

## How It Works

```
BIP-39 Mnemonic
  └─ Root Identity (m/999'/0') ── cold storage, signs delegation certs
       ├─ Action Key (m/999'/1'/i) ── warm, scoped by selector + value cap
       │    └─ Session Key (HKDF-SHA256) ── ephemeral, one-time use
       │         └─ Signs SovereignIntent v2.3 ── calldata hash + gas + credentials
       │              ├─ ExecutionGateway.execute() ── direct policy firewall
       │              └─ HuntKeyAccount.validateUserOp() ── ERC-4337
       └─ Recovery Keys (m/999'/3'/i) ── 2-of-N guardians + 48h timelock
```

1. **Delegate** — The root key signs an EIP-712 delegation certificate, granting an action key permission for a specific function selector and value cap.
2. **Derive** — The action key derives an ephemeral session key via HKDF-SHA256 with chain ID binding. Each session key is unique per (action key, chain, nonce).
3. **Sign** — The session key signs a `SovereignIntent` — a fully constrained description of the on-chain action, including exact calldata hash, gas parameters, and credential requirements.
4. **Execute** — The 3-layer signing chain is validated on-chain through 15 sequential checks. The call is forwarded only if every check passes.
5. **Burn** — The session key is marked as used. It cannot execute again.

---

## Defense-in-Depth: 4-Layer Execution Model

Each layer constrains the next. No single layer compromise can drain funds or hijack identity.

| Layer | Component | Role |
|:-----:|-----------|------|
| **1** | **Root Identity** | Cold storage. Signs delegation certificates. Never on-chain. |
| **2** | **Action Keys** | Warm keys. Scoped by function selector, value cap, expiration. |
| **3** | **Session Keys** | HKDF-SHA256 derived. One-time use. Chain-isolated. |
| **4** | **Execution** | On-chain policy firewall + ERC-4337 Account Abstraction. |

### On-Chain Validation (15 Checks)

Every `execute()` call passes through the full validation sequence:

| # | Check | Failure |
|---|-------|---------|
| 1 | Session cert signature → `parent` | `InvalidSessionSignature` |
| 2 | Parent in `authorizedKeys` | `UnauthorizedKey` |
| 3 | Session cert not expired | `SessionExpired` |
| 4 | Value within session cap | `SessionValueExceeded` |
| 5 | Intent signature → `session` | `InvalidIntentSignature` |
| 6 | Intent not expired | `IntentExpired` |
| 7 | Value within intent cap | `ValueExceedsCap` |
| 8 | Nonce matches | `InvalidNonce` |
| 9 | Session epoch matches | `SessionEpochMismatch` |
| 10 | Target address matches | `TargetMismatch` |
| 11 | Selector matches | `SelectorMismatch` |
| 12 | `keccak256(callData)` matches | `CallDataMismatch` |
| 13 | Identity not in recovery | `RecoveryBlocksExecution` |
| 14 | Required claim satisfied | `ClaimRequired` |
| 15 | External call succeeds | `ExecutionFailed` |

---

## Security Properties

**No blind signing.** Every intent binds `keccak256(callData)`. The gateway verifies the hash on-chain. A single-byte mutation in calldata causes revert.

**No long-lived execution keys.** Session keys are burned after a single `execute()`. No key reuse, no nonce management at the session level.

**Recoverable identity.** 2-of-N guardian threshold with 48-hour timelock. The original root can cancel recovery at any point during the window.

**Cross-chain isolation.** HKDF info includes `chain_id`. The same action key + nonce on different networks produces different session keys.

**Signature malleability protection.** Low-s normalization (`s <= N/2`) enforced on all signature verification paths.

**Zeroize everywhere.** All private key material implements `Zeroize`/`ZeroizeOnDrop`. HKDF outputs, signing keys, and info buffers are explicitly zeroed after use.

**Gas siphoning prevention.** `maxPriorityFeePerGas` is signed into the EIP-712 struct, preventing malicious bundlers from inflating priority fees.

**Mass invalidation.** Incrementing the root's session epoch instantly voids all outstanding sessions and intents in a single transaction.

---

## Features — v2.3

### ERC-4337 Account Abstraction
`HuntKeyAccount.sol` implements `IAccount.validateUserOp()`. The 3-layer signing chain is packed into `UserOperation.signature` as `abi.encode(SessionParams, IntentParams)`. Recovery-gated UserOps block all operations during `RecoveryPending` except recovery management.

### ZK Claim System
`ClaimVerifier.sol` implements commitment-based ZK claim verification with four claim types: `AGE_OVER_18`, `KYC_VERIFIED`, `COUNTRY_ALLOWED`, `DAO_MEMBER`. Claims are registered as hash commitments (`keccak256(abi.encodePacked(account, claimType, secret))`) and verified on-chain with replay protection.

### Full ERC-4337 Paymaster
`HuntKeyPaymaster.sol` implements `IPaymaster` with three modes: self-funded (0), ETH sponsorship (1), and ERC20 token payment (2). Includes deposit management, configurable token gas pricing, and post-op token collection.

### Multicall Execution
`executeMulticall()` supports batched calls with `keccak256(abi.encode(calls))` hash verification across the entire batch, bound to the intent's `callDataHash`.

### Credential Binding
`requiredClaim` field gates operations on verifiable claims. `claimProofHash` binds verified ZK proofs to specific intents. Both signed into the EIP-712 struct.

### Paymaster Binding
`paymasterMode` and `paymaster` address are signed into the EIP-712 struct, preventing mode downgrade and paymaster substitution attacks.

### Session Epoch Mass Invalidation
`sessionEpoch` field must match on-chain `sessionEpoch[root]`. Incrementing the epoch instantly invalidates all outstanding sessions without per-key revocation.

### Identity Monitoring
`IdentityWatcher` tracks on-chain events and generates security alerts at Info/Warning/Critical severity. `DashboardState` aggregates metrics into exportable snapshots with time-range filtering.

### Event Log (Black Box)
Structured, append-only event log records every intent execution, session invalidation, recovery state change, and high-value intent for forensic analysis and dashboard consumption.

---

## Real-World Use Cases

### Digital Identity & Credentials
Issue verifiable credentials as ZK claims. A university issues a `KYC_VERIFIED` claim as a hash commitment — the student proves eligibility on-chain without revealing personal data. The `requiredClaim` field gates smart contract access to verified holders only.

### Enterprise Access Control
Map corporate authorization hierarchies to the 4-layer model. Department heads hold action keys scoped to specific contract selectors and value caps. Session keys provide time-limited, single-use execution for employees. Mass invalidation via session epoch handles offboarding instantly.

### Medical Records
Patient identity anchored to a cold root key. Healthcare providers receive delegated action keys scoped to specific record operations. Session keys ensure each access is one-time and auditable. The credential system gates access on verifiable practitioner claims.

### Islamic Finance (Sharia-Compliant DeFi)
Intent-based signing eliminates blind approval of non-compliant operations. Every transaction's exact calldata is bound and verified. Credential claims can gate operations on Sharia compliance verification. The protocol's transparent, auditable execution model aligns with Islamic finance principles of clarity in contracts.

### DAO Identity & Governance
DAO members hold `DAO_MEMBER` claims verified through the ZK claim system. Governance proposals require credential-gated execution. Multi-call batching enables atomic proposal execution. The monitoring dashboard provides real-time visibility into DAO identity state and recovery events.

---

## Architecture

```
contracts/
├── src/
│   ├── IdentityStore.sol        Identity state, delegation, social recovery
│   ├── ExecutionGateway.sol     Session validation, scope enforcement, execution
│   ├── IAccount.sol             ERC-4337 IAccount interface
│   ├── HuntKeyAccount.sol       ERC-4337 account + claims + multicall
│   ├── ClaimVerifier.sol        ZK claim commitment verification
│   ├── IPaymaster.sol           ERC-4337 IPaymaster interface
│   └── HuntKeyPaymaster.sol     Paymaster: sponsored + ERC20 token payment
├── test/
│   └── PolicyGuard.t.sol        66 Solidity tests
└── lib/
    └── forge-std/               Foundry testing framework

src/
├── lib.rs                       Crate root, re-exports, 60 integration tests
├── core/mod.rs                  Key derivation, keccak256, ABI encoding
├── intents/mod.rs               SovereignIntent v2.3, DelegationCertificate, EIP-712
├── sessions/mod.rs              SessionKey (HKDF-SHA256), session certificates
├── recovery/mod.rs              RecoveryRequest, PendingRecovery, guardian signing
├── monitor/mod.rs               IdentityWatcher, SecurityAlert, DashboardState
└── wasm_api/mod.rs              WASM bindings (feature: "wasm")

sdk/ts/src/
└── index.ts                     TypeScript SDK (MnemonicManager, IntentSigner,
                                 SessionManager, ClaimManager, PaymasterClient,
                                 ProtocolAuditor, ProtocolDashboard)
```

### Key Hierarchy

```
BIP-39 Mnemonic (12/24 words)
  └─ BIP-32 Root (XPriv)
       ├─ m/44'/0'/0'/0/{i}    Bitcoin (legacy compatibility)
       ├─ m/44'/60'/0'/0/{i}   Ethereum (legacy compatibility)
       └─ m/999'               Sovereign Identity Namespace
            ├─ 0'              Root Identity (cold storage)
            ├─ 1'/{i}          Action Keys (warm, auto-incrementing)
            ├─ 2'/{i}          Proof Keys (reserved for ZK)
            └─ 3'/{i}          Recovery Keys (guardian operations)
```

All paths under `m/999'` use hardened derivation, isolating the sovereign identity namespace from standard BIP-44 derivations.

---

## Quick Start

### Run the Protocol Demo

```bash
cargo run
```

Outputs the full protocol flow: mnemonic generation, key hierarchy, delegation certificates, session keys, and 3-layer signing chain verification.

### Run Rust Tests (60 tests)

```bash
cargo test
```

Covers key derivation, EIP-712 hash determinism, sign/recover roundtrips, delegation chain verification, recovery threshold/timelock, HKDF session key derivation, cross-chain isolation, identity monitoring, guardian notifications, UserOperation builder, event log, dashboard snapshots, ZK claim proof binding, paymaster mode binding, and property-based tests via proptest.

### Run Solidity Tests (66 tests)

```bash
cd contracts && forge test -vv
```

Covers direct intent validation, delegated verification, social recovery, execution gateway, ERC-4337 validateUserOp, session epoch enforcement, credential/claim checks, multicall hash verification, ClaimVerifier (registration, verification, proof replay, revocation), and HuntKeyPaymaster (sponsored mode, token pay, postOp collection, deposit management).

### Build WASM SDK

```bash
cargo build --features wasm --target wasm32-unknown-unknown
```

---

## Documentation

| Document | Contents |
|----------|----------|
| **[ARCHITECTURE.md](ARCHITECTURE.md)** | 4-layer defense model, AA integration, state machine, key hierarchy |
| **[docs/USER_FLOW.md](docs/USER_FLOW.md)** | Identity → Delegation → Session → Intent → Execution flow |
| **[docs/CLAIMS.md](docs/CLAIMS.md)** | ZK claim system: commitment model, claim types, intent binding |
| **[docs/PAYMASTER.md](docs/PAYMASTER.md)** | ERC-4337 paymaster: modes, token payment, intent binding |
| **[docs/DASHBOARD.md](docs/DASHBOARD.md)** | Monitoring dashboard: snapshots, filtering, JSON export |
| **[docs/DEPLOYMENT.md](docs/DEPLOYMENT.md)** | Deployment & integration: contract deploy, SDK setup, operations |
| **[specs/protocol_overview.md](specs/protocol_overview.md)** | EIP-712 type strings, domain separator, session epoch |
| **[specs/threat_model.md](specs/threat_model.md)** | 14 attack vectors with mitigations |
| **[specs/key_hierarchy.md](specs/key_hierarchy.md)** | BIP-32/44 paths, HKDF-SHA256 derivation, zeroize policy |
| **[specs/invariants.md](specs/invariants.md)** | 4 formal protocol invariants |

---

## Requirements

- [Rust](https://rustup.rs/) 1.85+ (edition 2024)
- [Foundry](https://getfoundry.sh/) (Solidity 0.8.28)
- [wasm-pack](https://rustwasm.github.io/wasm-pack/) (optional, for WASM builds)

---

## Protocol Philosophy

HuntKey is built on three convictions:

**Keys are not identity.** A private key is a cryptographic primitive — not a person, not an organization, not a reputation. Identity is the root. Keys are scoped, temporary instruments that the identity delegates for constrained purposes. When a key is compromised, the identity survives.

**Every signature should mean something.** Blind signing is a design failure, not a user failure. HuntKey binds every signature to typed, structured data — the exact target, selector, calldata hash, value cap, expiration, chain, and credentials. The signer knows precisely what they are authorizing. The contract verifies precisely what was signed.

**Security is depth, not perimeter.** No single check protects the protocol. Fifteen validation steps, four hierarchical layers, and cryptographic binding at every transition create a system where compromise of any single component cannot escalate to full account control. Defense in depth is not a feature — it is the architecture.

---

<p align="center">
  <sub>MIT License</sub>
</p>

---

## Support

HuntKey is maintained as independent open-source work. If it is useful to you or
your team, sponsorship helps fund maintenance, tests, documentation and releases.

<p align="center">
  <a href="https://github.com/sponsors/abdulwahed-sweden?metadata_source=huntkey&metadata_campaign=readme">
    <img src="https://img.shields.io/badge/Sponsor_continued_development-%E2%9D%A4-db61a2?style=for-the-badge&logo=githubsponsors&logoColor=white" alt="Sponsor continued development">
  </a>
</p>
