use huntkey::{derive_key, generate_mnemonic, root_from_mnemonic};

fn main() {
    let mnemonic = generate_mnemonic(12);
    println!("========================================");
    println!("  BIP-39 Seed Phrase (12 Words)");
    println!("========================================");
    println!("  {}\n", mnemonic);

    let root = root_from_mnemonic(&mnemonic);

    println!("========================================");
    println!("  Bitcoin (BIP-44)");
    println!("========================================");
    for i in 0u32..3 {
        let path = format!("m/44'/0'/0'/0/{}", i);
        let dk = derive_key(&root, &path, false);
        println!("\n  --- Address #{} ---", i + 1);
        println!("  Path        : {}", dk.path);
        println!("  Private Key : 0x{}", hex::encode(&dk.private_key));
        println!("  Public Key  : 0x{}", hex::encode(&dk.public_key));
    }

    println!("\n========================================");
    println!("  Ethereum (BIP-44)");
    println!("========================================");
    for i in 0u32..3 {
        let path = format!("m/44'/60'/0'/0/{}", i);
        let dk = derive_key(&root, &path, true);
        println!("\n  --- Address #{} ---", i + 1);
        println!("  Path        : {}", dk.path);
        println!("  Private Key : 0x{}", hex::encode(&dk.private_key));
        println!("  Public Key  : 0x{}", hex::encode(&dk.public_key));
        if let Some(addr) = dk.eth_address {
            println!("  Address     : 0x{}", hex::encode(addr));
        }
    }

    println!();
}
