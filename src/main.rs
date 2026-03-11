use bip39::Mnemonic;
use coins_bip32::prelude::*;
use tiny_keccak::{Hasher, Keccak};

/// Derive an Ethereum address from an uncompressed public key (keccak256).
fn eth_address(pubkey: &k256::ecdsa::VerifyingKey) -> [u8; 20] {
    let uncompressed = pubkey.to_encoded_point(false);
    let mut hasher = Keccak::v256();
    hasher.update(&uncompressed.as_bytes()[1..]); // skip 0x04 prefix
    let mut hash = [0u8; 32];
    hasher.finalize(&mut hash);
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&hash[12..]); // last 20 bytes
    addr
}

fn derive_and_print(root: &XPriv, label: &str, path: &str) {
    let derived = root.derive_path(path).expect("derivation failed");
    let signing_key: &k256::ecdsa::SigningKey = derived.as_ref();
    let verifying_key = signing_key.verifying_key();

    println!("  Path        : {}", path);
    println!("  Private Key : 0x{}", hex::encode(signing_key.to_bytes()));
    println!(
        "  Public Key  : 0x{}",
        hex::encode(verifying_key.to_sec1_bytes())
    );

    if label == "Ethereum" {
        let addr = eth_address(verifying_key);
        println!("  Address     : 0x{}", hex::encode(addr));
    }
}

fn main() {
    // ── 1. Generate 12-word BIP-39 mnemonic ──
    let mnemonic = Mnemonic::generate(12).expect("failed to generate mnemonic");
    println!("========================================");
    println!("  BIP-39 Seed Phrase (12 Words)");
    println!("========================================");
    println!("  {}\n", mnemonic);

    // ── 2. Derive 64-byte seed (PBKDF2, 2048 rounds) ──
    let seed = mnemonic.to_seed("");

    // ── 3. Create BIP-32 master key ──
    let root = XPriv::root_from_seed(&seed, None).expect("failed to create root key");

    // ── 4. Bitcoin keys (BIP-44: m/44'/0'/0'/0/i) ──
    println!("========================================");
    println!("  Bitcoin (BIP-44)");
    println!("========================================");
    for i in 0u32..3 {
        let path = format!("m/44'/0'/0'/0/{}", i);
        println!("\n  --- Address #{} ---", i + 1);
        derive_and_print(&root, "Bitcoin", &path);
    }

    // ── 5. Ethereum keys (BIP-44: m/44'/60'/0'/0/i) ──
    println!("\n========================================");
    println!("  Ethereum (BIP-44)");
    println!("========================================");
    for i in 0u32..3 {
        let path = format!("m/44'/60'/0'/0/{}", i);
        println!("\n  --- Address #{} ---", i + 1);
        derive_and_print(&root, "Ethereum", &path);
    }

    println!();
}
