# ZK Claim System

## Overview

The ZK Claim System enables identity-bound verifiable credentials that gate on-chain operations. Claims are registered as hash commitments and verified on-chain without revealing the underlying data.

## Claim Types

| Constant | Description |
|----------|-------------|
| `AGE_OVER_18` | Age verification (18+) |
| `KYC_VERIFIED` | Know Your Customer verification |
| `COUNTRY_ALLOWED` | Country/jurisdiction allowlist |
| `DAO_MEMBER` | DAO membership proof |

## Architecture

```
Issuer registers commitment
  │
  ▼
ClaimVerifier.sol
  ├── registerClaim(account, claimType, commitment)
  ├── verifyClaimProof(account, claimType, proof) → proofHash
  ├── hasClaim(account, claimType) → bool
  └── verifyProofHash(account, claimType, proofHash) → bool
```

### Commitment Model

1. **Off-chain**: Issuer computes `commitment = keccak256(abi.encodePacked(account, claimType, secret))`
2. **Registration**: Issuer calls `registerClaim(account, claimType, commitment)`
3. **Verification**: User proves claim by revealing `secret` → `verifyClaimProof()` computes the hash and compares
4. **Replay Prevention**: Each proof hash is marked as used after verification

### Intent Binding

The `claimProofHash` field in SovereignIntent v2.3 binds a verified claim proof to the intent:

```
SovereignIntent {
  ...
  requiredClaim:  bytes32  -- claim type required (zero = none)
  claimProofHash: bytes32  -- hash of the verified proof (zero = no binding)
  ...
}
```

When `claimProofHash` is non-zero, the intent is bound to a specific proof verification. This prevents proof reuse across intents.

## SDK Integration

```typescript
import { ClaimManager, ClaimTypes } from "@huntkey/sdk";

const claims = new ClaimManager(provider, claimVerifierAddress);

// Check if account has a claim
const hasKyc = await claims.hasClaim(account, kycTypeHash);

// Verify a proof hash
const valid = await claims.verifyProofHash(account, kycTypeHash, proofHash);
```

## Security Properties

- **Zero-knowledge**: Only the commitment hash is stored on-chain; the secret is never revealed publicly
- **Replay protection**: Each proof can only be used once via `usedProofs` mapping
- **Issuer control**: Only the authorized issuer can register/revoke claims
- **Intent binding**: `claimProofHash` in the EIP-712 struct prevents reuse of proofs across intents
