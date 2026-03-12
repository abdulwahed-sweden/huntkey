# Deployment & Integration Guide

This guide covers end-to-end deployment of the HuntKey protocol contracts, Rust SDK configuration, TypeScript SDK integration, and production operations.

---

## Table of Contents

1. [Prerequisites](#1-prerequisites)
2. [Contract Deployment](#2-contract-deployment)
3. [Post-Deployment Configuration](#3-post-deployment-configuration)
4. [Rust SDK Integration](#4-rust-sdk-integration)
5. [TypeScript SDK Integration](#5-typescript-sdk-integration)
6. [ERC-4337 Integration](#6-erc-4337-integration)
7. [Paymaster Setup](#7-paymaster-setup)
8. [ZK Claim Setup](#8-zk-claim-setup)
9. [Monitoring & Dashboard](#9-monitoring--dashboard)
10. [Upgrade & Migration](#10-upgrade--migration)
11. [Security Checklist](#11-security-checklist)
12. [Network Reference](#12-network-reference)

---

## 1. Prerequisites

### Toolchain

| Tool | Version | Purpose |
|------|---------|---------|
| [Rust](https://rustup.rs/) | 1.85+ (edition 2024) | Rust SDK, key derivation, signing |
| [Foundry](https://getfoundry.sh/) | Latest | Contract compilation, testing, deployment |
| [Node.js](https://nodejs.org/) | 18+ | TypeScript SDK |
| [wasm-pack](https://rustwasm.github.io/wasm-pack/) | 0.12+ | WASM build (optional) |

### Environment Variables

Create a `.env` file (never committed):

```bash
# Deployer private key (NOT a HuntKey root — this is the contract owner)
DEPLOYER_PRIVATE_KEY=0x...

# RPC endpoints
RPC_URL_MAINNET=https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY
RPC_URL_SEPOLIA=https://eth-sepolia.g.alchemy.com/v2/YOUR_KEY
RPC_URL_BASE=https://base-mainnet.g.alchemy.com/v2/YOUR_KEY

# Verification
ETHERSCAN_API_KEY=...

# ERC-4337 EntryPoint (v0.7)
ENTRYPOINT_ADDRESS=0x0000000071727De22E5E9d8BAf0edAc6f37da032
```

> **Security**: The `DEPLOYER_PRIVATE_KEY` is the Foundry deployer account. It becomes the `owner` of all deployed contracts. This key should be a multisig or hardware wallet in production.

---

## 2. Contract Deployment

### Deployment Order

Contracts must be deployed in this order due to inheritance and configuration dependencies:

```
1. HuntKeyAccount      (inherits ExecutionGateway → IdentityStore)
2. ClaimVerifier        (standalone, issuer set at construction)
3. HuntKeyPaymaster     (standalone, needs EntryPoint address)
```

### Step 1: Deploy HuntKeyAccount

`HuntKeyAccount` is the primary contract. It inherits `ExecutionGateway` (which inherits `IdentityStore`), so deploying it deploys the entire protocol stack.

```bash
cd contracts

# Testnet (Sepolia)
forge create src/HuntKeyAccount.sol:HuntKeyAccount \
  --rpc-url $RPC_URL_SEPOLIA \
  --private-key $DEPLOYER_PRIVATE_KEY \
  --verify \
  --etherscan-api-key $ETHERSCAN_API_KEY

# Mainnet
forge create src/HuntKeyAccount.sol:HuntKeyAccount \
  --rpc-url $RPC_URL_MAINNET \
  --private-key $DEPLOYER_PRIVATE_KEY \
  --verify \
  --etherscan-api-key $ETHERSCAN_API_KEY
```

Record the deployed address — this is your `HUNTKEY_ACCOUNT` address.

The constructor automatically computes the `DOMAIN_SEPARATOR` using `block.chainid` and `address(this)`, binding all EIP-712 signatures to this specific deployment.

### Step 2: Deploy ClaimVerifier

```bash
# The issuer address is the account authorized to register/revoke claims.
# In production, this should be a multisig or claim issuance service.
ISSUER_ADDRESS=0x...

forge create src/ClaimVerifier.sol:ClaimVerifier \
  --constructor-args $ISSUER_ADDRESS \
  --rpc-url $RPC_URL_SEPOLIA \
  --private-key $DEPLOYER_PRIVATE_KEY \
  --verify \
  --etherscan-api-key $ETHERSCAN_API_KEY
```

Record the deployed address — this is your `CLAIM_VERIFIER` address.

### Step 3: Deploy HuntKeyPaymaster

```bash
forge create src/HuntKeyPaymaster.sol:HuntKeyPaymaster \
  --constructor-args $ENTRYPOINT_ADDRESS \
  --rpc-url $RPC_URL_SEPOLIA \
  --private-key $DEPLOYER_PRIVATE_KEY \
  --verify \
  --etherscan-api-key $ETHERSCAN_API_KEY
```

Record the deployed address — this is your `PAYMASTER` address.

### Deployment Script (Programmatic)

For reproducible deployments, create `contracts/script/Deploy.s.sol`:

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import "forge-std/Script.sol";
import {HuntKeyAccount} from "../src/HuntKeyAccount.sol";
import {ClaimVerifier} from "../src/ClaimVerifier.sol";
import {HuntKeyPaymaster} from "../src/HuntKeyPaymaster.sol";

contract Deploy is Script {
    function run() external {
        uint256 deployerKey = vm.envUint("DEPLOYER_PRIVATE_KEY");
        address entryPoint = vm.envAddress("ENTRYPOINT_ADDRESS");
        address issuer = vm.envAddress("ISSUER_ADDRESS");

        vm.startBroadcast(deployerKey);

        HuntKeyAccount account = new HuntKeyAccount();
        account.setEntryPoint(entryPoint);

        ClaimVerifier verifier = new ClaimVerifier(issuer);

        HuntKeyPaymaster paymaster = new HuntKeyPaymaster(entryPoint);

        vm.stopBroadcast();

        console.log("HuntKeyAccount:", address(account));
        console.log("ClaimVerifier:", address(verifier));
        console.log("HuntKeyPaymaster:", address(paymaster));
        console.log("Domain Separator:", vm.toString(account.DOMAIN_SEPARATOR()));
    }
}
```

Run with:

```bash
forge script script/Deploy.s.sol:Deploy \
  --rpc-url $RPC_URL_SEPOLIA \
  --broadcast \
  --verify \
  --etherscan-api-key $ETHERSCAN_API_KEY
```

---

## 3. Post-Deployment Configuration

After deploying all contracts, the owner must configure the protocol:

### 3.1 Set the EntryPoint

```bash
cast send $HUNTKEY_ACCOUNT "setEntryPoint(address)" $ENTRYPOINT_ADDRESS \
  --rpc-url $RPC_URL_SEPOLIA \
  --private-key $DEPLOYER_PRIVATE_KEY
```

### 3.2 Register Action Keys

Each root identity's action keys must be authorized on-chain:

```bash
# Authorize an action key derived from the root identity
cast send $HUNTKEY_ACCOUNT "authorizeKey(address)" $ACTION_KEY_ADDRESS \
  --rpc-url $RPC_URL_SEPOLIA \
  --private-key $DEPLOYER_PRIVATE_KEY
```

### 3.3 Set Guardians for Social Recovery

```bash
# Register 3 guardians for a root identity
cast send $HUNTKEY_ACCOUNT \
  "setGuardians(address,address[])" \
  $ROOT_ADDRESS \
  "[$GUARDIAN_1,$GUARDIAN_2,$GUARDIAN_3]" \
  --rpc-url $RPC_URL_SEPOLIA \
  --private-key $DEPLOYER_PRIVATE_KEY
```

### 3.4 Fund the Paymaster

```bash
# Deposit ETH into the paymaster's EntryPoint balance
cast send $PAYMASTER "deposit()" \
  --value 1ether \
  --rpc-url $RPC_URL_SEPOLIA \
  --private-key $DEPLOYER_PRIVATE_KEY

# Configure sponsored accounts
cast send $PAYMASTER "setSponsoredAccount(address,bool)" $ACCOUNT_ADDRESS true \
  --rpc-url $RPC_URL_SEPOLIA \
  --private-key $DEPLOYER_PRIVATE_KEY

# Configure ERC20 token gas payment (token address + price per gas in token units)
cast send $PAYMASTER "setTokenGasPrice(address,uint256)" $TOKEN_ADDRESS $PRICE_PER_GAS \
  --rpc-url $RPC_URL_SEPOLIA \
  --private-key $DEPLOYER_PRIVATE_KEY
```

### 3.5 Register Claims

```bash
# Register a KYC claim for an account via the ClaimVerifier
# commitment = keccak256(abi.encodePacked(account, claimType, secret))
cast send $CLAIM_VERIFIER \
  "registerClaim(address,bytes32,bytes32)" \
  $ACCOUNT_ADDRESS $KYC_CLAIM_TYPE_HASH $COMMITMENT_HASH \
  --rpc-url $RPC_URL_SEPOLIA \
  --private-key $ISSUER_PRIVATE_KEY
```

---

## 4. Rust SDK Integration

### 4.1 Full Signing Flow

```rust
use huntkey::{
    root_from_mnemonic, KeyHierarchy, KeyRole,
    SovereignIntent, sign_intent, recover_signer,
    derive_session_key, SessionCertificate, sign_session_cert,
    call_data_hash,
};

// Step 1: Derive the key hierarchy from the user's mnemonic
let mnemonic = /* securely loaded from keystore */;
let root = root_from_mnemonic(&mnemonic);
let mut hierarchy = KeyHierarchy::new(root);

// Step 2: Get the action key
let action_key = hierarchy.next_action_key();
let action_privkey: [u8; 32] = action_key.private_key
    .as_slice().try_into().unwrap();

// Step 3: Derive an ephemeral session key
let session = derive_session_key(&action_privkey, nonce, chain_id);

// Step 4: Sign the session certificate (action key signs)
let session_cert = SessionCertificate {
    session: session.eth_address,
    parent: action_key.eth_address.unwrap(),
    scope: [0xa9, 0x05, 0x9c, 0xbb], // transfer(address,uint256)
    target: target_contract,
    max_value: 2_000_000_000_000_000_000, // 2 ETH
    expiration: 1800000000,
    chain_id: 1,
};
let signed_session = sign_session_cert(
    &session_cert, &verifying_contract, &action_privkey
);

// Step 5: Build and sign the intent (session key signs)
let calldata = /* ABI-encoded function call */;
let intent = SovereignIntent {
    target_contract,
    function_sig: [0xa9, 0x05, 0x9c, 0xbb],
    recipient: recipient_address,
    asset_address: [0x00; 20],        // native ETH
    call_data_hash: call_data_hash(&calldata),
    max_value: 1_000_000_000_000_000_000,
    expiration: 1800000000,
    chain_id: 1,
    nonce: 0,
    session_epoch: 0,                 // must match on-chain
    gas_limit: 100_000,
    max_fee_per_gas: 50_000_000_000,
    max_priority_fee_per_gas: 2_000_000_000,
    required_claim: [0x00; 32],       // no claim required
    claim_proof_hash: [0x00; 32],     // no proof binding
    paymaster_mode: 0,                // self-funded
    paymaster: [0x00; 20],            // no paymaster
};
let intent_sig = sign_intent(
    &intent, &verifying_contract, &session.private_key
);
```

### 4.2 UserOperation Builder

```rust
use huntkey::UserOperationBuilder;

let user_op = UserOperationBuilder::new(account_address)
    .nonce(entrypoint_nonce)
    .call_data(calldata.to_vec())
    .gas(200_000, 100_000, 50_000, 50_000_000_000, 2_000_000_000)
    .paymaster_and_data(paymaster_data)
    .build(signature_payload);
```

### 4.3 WASM Build for Browser

```bash
cargo build --features wasm --target wasm32-unknown-unknown
# Or with wasm-pack for npm packaging:
wasm-pack build --features wasm --target web
```

---

## 5. TypeScript SDK Integration

### 5.1 Initialize the SDK

```typescript
import { init, IntentSigner, SessionManager, zeroize } from "@huntkey/sdk";
import huntkey_wasm from "@huntkey/wasm";

// Load the WASM module
init(huntkey_wasm);
```

### 5.2 Construct and Sign an Intent

```typescript
const intent = {
  targetContract: "bb".repeat(20),
  functionSig: "a9059cbb",
  recipient: "cc".repeat(20),
  assetAddress: "00".repeat(20),
  callDataHash: IntentSigner.computeCallDataHash(callDataHex),
  maxValue: "1000000000000000000",
  expiration: Math.floor(Date.now() / 1000) + 3600,
  chainId: 1,
  nonce: 0,
  sessionEpoch: 0,
  gasLimit: 100000,
  maxFeePerGas: "50000000000",
  maxPriorityFeePerGas: "2000000000",
  requiredClaim: "00".repeat(32),
  claimProofHash: "00".repeat(32),
  paymasterMode: 0,
  paymaster: "00".repeat(20),
};

const intentJson = IntentSigner.createIntent(intent);
```

### 5.3 Sign a Session Certificate

```typescript
const sig = SessionManager.signSessionCert(
  sessionCert,
  gatewayAddress,
  actionPrivkeyHex  // zeroed in WASM after use
);

// Always zeroize local key copies
zeroize(actionKeyBytes);
```

### 5.4 Query On-Chain State

```typescript
import { ProtocolAuditor } from "@huntkey/sdk";

const auditor = new ProtocolAuditor(provider, huntKeyAccountAddress);

// Get full identity state
const state = await auditor.getIdentityState(rootAddress);
console.log("State:", state.state);           // 0=Active, 1=Recovery, 2=Frozen
console.log("Epoch:", state.sessionEpoch);

// Check if an epoch is stale
const revoked = await auditor.isEpochRevoked(rootAddress, BigInt(0));
```

---

## 6. ERC-4337 Integration

### 6.1 Bundler Configuration

HuntKey is compatible with any ERC-4337 v0.7+ bundler (Stackup, Pimlico, Alchemy, Biconomy).

```typescript
// Pack the 3-layer signature chain into UserOperation.signature
const signature = ethers.AbiCoder.defaultAbiCoder().encode(
  [
    "tuple(address,address,bytes4,address,uint128,uint64,uint64,uint8,bytes32,bytes32)",
    "tuple(address,bytes4,address,address,bytes32,uint128,uint64,uint64,uint64,uint64,uint64,uint128,uint128,bytes32,bytes32,uint8,address,uint8,bytes32,bytes32)"
  ],
  [sessionParams, intentParams]
);

const userOp = {
  sender: huntKeyAccountAddress,
  nonce: await entryPoint.getNonce(huntKeyAccountAddress, 0),
  initCode: "0x",
  callData: targetCallData,
  accountGasLimits: packGasLimits(callGasLimit, verificationGasLimit),
  preVerificationGas: preVerificationGas,
  gasFees: packGasFees(maxFeePerGas, maxPriorityFeePerGas),
  paymasterAndData: "0x",  // or paymaster data for sponsored/token modes
  signature: signature,
};
```

### 6.2 validationData Decoding

```typescript
// HuntKeyAccount returns packed validationData:
// authorizer (160 bits) | validUntil (48 bits) | validAfter (48 bits)
function decodeValidationData(data: bigint) {
  const authorizer = data & ((1n << 160n) - 1n);
  const validUntil = (data >> 160n) & ((1n << 48n) - 1n);
  const validAfter = (data >> 208n) & ((1n << 48n) - 1n);
  return { authorizer, validUntil, validAfter };
}
```

### 6.3 Recovery Exception

During `RecoveryPending`, all UserOps are blocked except recovery management functions:

| Selector | Function | Behavior |
|----------|----------|----------|
| `cancelRecovery(address)` | Allowed | Skips 3-layer validation |
| `supportRecovery(address,uint8,bytes32,bytes32)` | Allowed | Skips 3-layer validation |
| `finalizeRecovery(address)` | Allowed | Skips 3-layer validation |
| All others | Blocked | Reverts with `RecoveryBlocksUserOp` |

---

## 7. Paymaster Setup

### 7.1 Mode 1 — ETH Sponsorship

The simplest mode. The paymaster pays gas from its EntryPoint deposit.

```bash
# 1. Fund the paymaster
cast send $PAYMASTER "deposit()" --value 10ether \
  --rpc-url $RPC_URL --private-key $DEPLOYER_KEY

# 2. Approve accounts for sponsorship
cast send $PAYMASTER "setSponsoredAccount(address,bool)" $USER_ACCOUNT true \
  --rpc-url $RPC_URL --private-key $DEPLOYER_KEY
```

UserOp `paymasterAndData` format:

```
[paymaster address (20 bytes)] [0x01 (1 byte)]
```

### 7.2 Mode 2 — ERC20 Token Payment

Users pay gas in ERC20 tokens. The paymaster collects tokens in `postOp`.

```bash
# 1. Configure token and price (price per gas unit, scaled by 1e18)
cast send $PAYMASTER "setTokenGasPrice(address,uint256)" \
  $USDC_ADDRESS 1000000000000000 \  # 0.001 USDC per gas unit
  --rpc-url $RPC_URL --private-key $DEPLOYER_KEY

# 2. User must approve the paymaster to spend tokens
cast send $USDC_ADDRESS "approve(address,uint256)" $PAYMASTER $MAX_AMOUNT \
  --rpc-url $RPC_URL --private-key $USER_KEY
```

UserOp `paymasterAndData` format:

```
[paymaster address (20 bytes)] [0x02 (1 byte)] [token address (20 bytes)]
```

### 7.3 SDK Paymaster Client

```typescript
import { PaymasterClient, PaymasterMode } from "@huntkey/sdk";

const pm = new PaymasterClient(provider, paymasterAddress);

// Check if sponsored
const sponsored = await pm.isSponsored(userAddress);

// Build paymasterAndData
const pmData = pm.buildPaymasterAndData(PaymasterMode.SPONSORED);
const pmDataToken = pm.buildPaymasterAndData(
  PaymasterMode.TOKEN_PAY, usdcAddress
);

// Include in the intent
const intent = {
  ...otherFields,
  paymasterMode: 1,        // sponsored
  paymaster: paymasterHex, // paymaster address (no 0x)
};
```

---

## 8. ZK Claim Setup

### 8.1 Claim Registration Flow

```
1. Issuer generates secret for the user
2. Issuer computes commitment = keccak256(abi.encodePacked(account, claimType, secret))
3. Issuer calls ClaimVerifier.registerClaim(account, claimType, commitment)
4. Issuer securely delivers secret to user (out-of-band)
```

### 8.2 Claim Verification in Intents

```
1. User calls ClaimVerifier.verifyClaimProof(account, claimType, secret) → proofHash
2. User sets intent.requiredClaim = claimType
3. User sets intent.claimProofHash = proofHash
4. On-chain: ExecutionGateway checks claim satisfaction
```

### 8.3 Claim Type Constants

```solidity
AGE_OVER_18     = keccak256("AGE_OVER_18")
KYC_VERIFIED    = keccak256("KYC_VERIFIED")
COUNTRY_ALLOWED = keccak256("COUNTRY_ALLOWED")
DAO_MEMBER      = keccak256("DAO_MEMBER")
```

### 8.4 SDK Claim Manager

```typescript
import { ClaimManager } from "@huntkey/sdk";

const claims = new ClaimManager(provider, claimVerifierAddress);
const hasClaim = await claims.hasClaim(account, kycTypeHash);
const valid = await claims.verifyProofHash(account, kycTypeHash, proofHash);
```

---

## 9. Monitoring & Dashboard

### 9.1 Rust Watcher Setup

```rust
use huntkey::{
    IdentityWatcher, WatcherConfig,
    DashboardState, export_dashboard_state,
};

// Configure with a high-value threshold (e.g., 10 ETH)
let config = WatcherConfig {
    high_value_threshold: 10_000_000_000_000_000_000,
};
let mut watcher = IdentityWatcher::with_config(config);

// Register known guardians and delegates
watcher.register_guardians(root_identity, guardian_list);
watcher.register_delegate(root_identity, action_key_address);

// Process events from your event subscription
watcher.on_intent_executed(identity, session, selector, block, timestamp);
watcher.on_recovery_state_changed(identity, "RecoveryPending", Some(guardian), block, ts);
watcher.on_session_invalidated(identity, new_epoch, block, timestamp);

// Drain guardian notifications
let notifications = watcher.drain_notifications();
for notif in notifications {
    push_notification_service.send(notif.guardian, &notif.alert);
}

// Dashboard metrics
let json = export_dashboard_state(watcher.event_log(), now);
```

### 9.2 Dashboard API

```rust
let dashboard = DashboardState::new(watcher.event_log());

// Full snapshot
let snap = dashboard.snapshot(now);
// snap.active_identities, snap.executed_intents, etc.

// Time-range filtered
let snap = dashboard.snapshot_in_range(hour_ago, now, now);

// Query entries
let entries = dashboard.entries_in_range(hour_ago, now);
let intent_entries = dashboard.entries_by_type(EventType::IntentExecuted);
```

### 9.3 TypeScript Dashboard

```typescript
import { ProtocolAuditor, ProtocolDashboard } from "@huntkey/sdk";

const auditor = new ProtocolAuditor(provider, huntKeyAccountAddress);
const dashboard = new ProtocolDashboard(auditor);

// Batch identity state queries
const states = await dashboard.batchGetIdentityState(rootAddresses);

// Aggregate by state
const counts = await dashboard.countByState(rootAddresses);
console.log(`Active: ${counts.active}`);
console.log(`Recovery: ${counts.recoveryPending}`);
console.log(`Frozen: ${counts.frozen}`);
```

### 9.4 Event Subscription (Production)

In production, wire the watcher to an Ethereum event subscription:

```rust
// Using alloy (or ethers-rs)
let filter = Filter::new()
    .address(huntkey_account_address)
    .events(vec![
        "IntentExecuted(address,address,bytes4)",
        "SessionInvalidated(address,uint256)",
        "RecoveryStateChanged(address,uint8)",
    ]);

provider.subscribe_logs(&filter, |log| {
    match log.topic0 {
        INTENT_EXECUTED_SIG => watcher.on_intent_executed(...),
        SESSION_INVALIDATED_SIG => watcher.on_session_invalidated(...),
        RECOVERY_STATE_CHANGED_SIG => watcher.on_recovery_state_changed(...),
    }
});
```

---

## 10. Upgrade & Migration

### Contract Immutability

All HuntKey contracts are **non-upgradeable** by design. The `DOMAIN_SEPARATOR` is immutable, computed at construction from `block.chainid` and `address(this)`. This means:

- **No proxy patterns** — reduces attack surface
- **No storage layout risks** — DOMAIN_SEPARATOR cannot be corrupted
- **Migration = new deployment** — deploy new contracts, migrate state

### Migration Procedure

1. Deploy new contracts with the updated code
2. Authorize the same action keys on the new contract
3. Set the same guardians on the new contract
4. Increment `sessionEpoch` on the old contract to void all outstanding sessions
5. Update dApp configuration to point to new contract addresses
6. Users re-derive session keys against the new `DOMAIN_SEPARATOR`

### Session Epoch Migration

To instantly void all outstanding sessions and intents:

```bash
cast send $HUNTKEY_ACCOUNT "cancelAllSessions(address)" $ROOT_ADDRESS \
  --rpc-url $RPC_URL --private-key $AUTHORIZED_KEY
```

This increments `sessionEpoch[root]`. All intents signed with the previous epoch will fail the `SessionEpochMismatch` check.

---

## 11. Security Checklist

### Pre-Deployment

- [ ] All 66 Solidity tests pass (`forge test`)
- [ ] All 60 Rust tests pass (`cargo test`)
- [ ] Contract verified on block explorer
- [ ] `DOMAIN_SEPARATOR` logged and matches expected value
- [ ] Deployer key is a multisig or hardware wallet
- [ ] EntryPoint address is the canonical v0.7 deployment

### Post-Deployment

- [ ] EntryPoint set via `setEntryPoint()`
- [ ] At least one action key authorized via `authorizeKey()`
- [ ] Guardians set for all root identities (minimum 3)
- [ ] Paymaster funded if using sponsored mode
- [ ] Claim issuer address correctly configured
- [ ] IdentityWatcher running with guardian notification drain

### Operational

- [ ] Monitor `RecoveryStateChanged` events — unknown guardian = immediate response
- [ ] High-value threshold configured on IdentityWatcher
- [ ] Session epoch increment procedure documented for incident response
- [ ] Guardian notification pipeline tested end-to-end
- [ ] Paymaster deposit balance monitored (alert if below threshold)
- [ ] Token gas prices updated if using ERC20 payment mode

### Key Material

- [ ] Root identity key (`m/999'/0'`) in cold storage — never touches the network
- [ ] All private key material implements `Zeroize`/`ZeroizeOnDrop`
- [ ] Session keys discarded after single use
- [ ] Action key private keys stored in secure enclave or HSM
- [ ] Mnemonic backup stored in geographically separated secure locations

---

## 12. Network Reference

### Supported Networks

| Network | Chain ID | EntryPoint v0.7 |
|---------|----------|-----------------|
| Ethereum Mainnet | 1 | `0x0000000071727De22E5E9d8BAf0edAc6f37da032` |
| Sepolia | 11155111 | `0x0000000071727De22E5E9d8BAf0edAc6f37da032` |
| Base | 8453 | `0x0000000071727De22E5E9d8BAf0edAc6f37da032` |
| Arbitrum One | 42161 | `0x0000000071727De22E5E9d8BAf0edAc6f37da032` |
| Optimism | 10 | `0x0000000071727De22E5E9d8BAf0edAc6f37da032` |
| Polygon | 137 | `0x0000000071727De22E5E9d8BAf0edAc6f37da032` |

### Cross-Chain Isolation

Each deployment produces a unique `DOMAIN_SEPARATOR` from `(chainId, address(this))`. Session keys derived via HKDF include `chain_id` in the info string. The same action key + nonce on different networks produces different session keys. No cross-chain replay is possible.

### Contract Addresses (Template)

After deployment, record addresses in a configuration file:

```json
{
  "sepolia": {
    "huntKeyAccount": "0x...",
    "claimVerifier": "0x...",
    "paymaster": "0x...",
    "entryPoint": "0x0000000071727De22E5E9d8BAf0edAc6f37da032",
    "domainSeparator": "0x..."
  }
}
```
