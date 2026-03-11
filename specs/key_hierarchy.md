# HuntKey v1.0 — Key Hierarchy

## BIP-32/44 Derivation Paths

### Legacy Paths (BIP-44 Compatibility)

```
Bitcoin:   m/44'/0'/0'/0/{i}
Ethereum:  m/44'/60'/0'/0/{i}
```

Standard HD wallet paths preserved for backward compatibility.

### Sovereign Identity Paths (Purpose 999')

```
m/999'/0'        Root Identity      Cold storage, never on-chain
m/999'/1'/{i}    Action Keys        Warm keys for signing sessions
m/999'/2'/{i}    Proof Keys         ZK proof generation (reserved)
m/999'/3'/{i}    Recovery Keys      Guardian/recovery operations
```

All paths under `m/999'` use hardened derivation (`'`), isolating the sovereign identity namespace from standard BIP-44 derivations.

## Key Roles

### Root Identity (`m/999'/0'`)

- Single key (no index). Master identity anchor.
- Signs `DelegationCertificate` to authorize action keys.
- Never transmitted on-chain. Stored in cold storage.
- Can cancel all sessions, freeze identity, set guardians.

### Action Keys (`m/999'/1'/{i}`)

- Auto-incrementing index via `KeyHierarchy::next_action_key()`.
- Registered on-chain via `authorizeKey(address)`.
- Signs `SessionCertificate` to authorize ephemeral session keys.
- Each key is scoped by the session certificates it issues.

### Proof Keys (`m/999'/2'/{i}`)

- Reserved for future ZK proof generation.
- Not currently used in the protocol.

### Recovery Keys (`m/999'/3'/{i}`)

- Used as guardian keys for social recovery.
- Sign `RecoveryRequest` structs off-chain.
- 3-5 guardians per identity, 2-of-N threshold.

## Ephemeral Session Keys

Derived deterministically from an action key, not from the BIP-32 tree:

```
HKDF-SHA256(
    IKM:  action_private_key (32 bytes)
    Salt: "HuntKey-V1-Session-Key"
    Info: parent_compressed_pubkey (33 bytes) || nonce (8 bytes BE)
) → session_private_key (32 bytes)
```

Properties:
- Deterministic: same action key + nonce always produces the same session key.
- Unique: different action keys or nonces produce different session keys.
- One-time use: burned on-chain after `execute()`.
- Zeroized: `SessionKey` implements `Zeroize` and `ZeroizeOnDrop`.

## Trust Chain

```
BIP-39 Mnemonic
  └─ BIP-32 Root (XPriv)
       └─ m/999'/0' (Root Identity)
            │
            ├─ signs DelegationCertificate ──> m/999'/1'/{i} (Action Key)
            │                                      │
            │                                      └─ signs SessionCertificate ──> HKDF(action, nonce) (Session Key)
            │                                                                          │
            │                                                                          └─ signs SovereignIntent
            │
            └─ m/999'/3'/{i} (Recovery Keys / Guardians)
                 └─ signs RecoveryRequest (2-of-N threshold + 48h timelock)
```

## Zeroize Policy

All private key material implements `Zeroize` and/or `ZeroizeOnDrop`:
- `DerivedKey.private_key: Vec<u8>` — zeroed on drop
- `SessionKey.private_key: [u8; 32]` — zeroed on drop
- All signing functions wrap key bytes in `Zeroizing<[u8; 32]>` and explicitly zeroize after use
- HKDF output key material (`okm`) is zeroized after constructing the signing key
