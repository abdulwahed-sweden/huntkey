# HuntKey

BIP-39 seed phrase generator + BIP-32/44 HD key derivation in Rust, with an on-chain ECDSA signature verifier in Solidity.

## What It Does

**Rust** — Generates a 12-word mnemonic and derives Bitcoin + Ethereum keys:
- BIP-39: mnemonic generation (128-bit entropy + checksum)
- BIP-32: hierarchical deterministic key derivation
- BIP-44: standard derivation paths (`m/44'/0'/0'/0/i` for BTC, `m/44'/60'/0'/0/i` for ETH)
- Keccak-256 hashing for Ethereum address derivation

**Solidity** — On-chain signature verification (`ecrecover`) using EIP-191 signed messages.

## Structure

```
src/main.rs               Seed phrase + key derivation (Rust)
contracts/src/SignatureVerifier.sol   ECDSA signature verifier (Solidity)
contracts/test/            Foundry tests
```

## Quick Start

### Generate Keys

```bash
cargo run
```

Output: 12-word seed phrase + 3 Bitcoin and 3 Ethereum derived key pairs.

### Run Contract Tests

```bash
cd contracts
forge test -vv
```

## Requirements

- [Rust](https://rustup.rs/) 1.93+
- [Foundry](https://getfoundry.sh/)

## License

MIT
