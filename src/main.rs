use huntkey::{
    derive_key, generate_mnemonic, root_from_mnemonic,
    KeyHierarchy, KeyRole, SovereignIntent,
    sign_intent, recover_signer, role_path,
};

fn main() {
    let mnemonic = generate_mnemonic(12);
    println!("========================================");
    println!("  BIP-39 Seed Phrase (12 Words)");
    println!("========================================");
    println!("  {}\n", mnemonic);

    let root = root_from_mnemonic(&mnemonic);

    // --- Legacy BIP-44 derivation (kept for compatibility) ---
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

    // --- Sovereign Identity Protocol ---
    println!("\n========================================");
    println!("  Sovereign Identity Protocol");
    println!("========================================");

    let root2 = root_from_mnemonic(&mnemonic);
    let mut hierarchy = KeyHierarchy::new(root2);

    // Root identity key (display only — never used on-chain)
    let root_id = hierarchy.derive_role(KeyRole::RootIdentity, 0);
    println!("\n  Root Identity (NEVER sent on-chain):");
    println!("  Path    : {}", role_path(KeyRole::RootIdentity, 0));
    println!("  Address : 0x{}", hex::encode(root_id.eth_address.unwrap()));

    // Recovery key (display only)
    let recovery = hierarchy.derive_role(KeyRole::Recovery, 0);
    println!("\n  Recovery Key (cold storage):");
    println!("  Path    : {}", role_path(KeyRole::Recovery, 0));
    println!("  Address : 0x{}", hex::encode(recovery.eth_address.unwrap()));

    // Ephemeral action key
    let action_key = hierarchy.next_action_key();
    println!("\n  Ephemeral Action Key #0:");
    println!("  Path    : {}", action_key.path);
    println!("  Address : 0x{}", hex::encode(action_key.eth_address.unwrap()));

    // Build and sign a sample intent
    let verifying_contract = [0xAA; 20];
    let intent = SovereignIntent {
        target_contract: [0xBB; 20],
        function_sig: [0xa9, 0x05, 0x9c, 0xbb], // transfer(address,uint256)
        max_value: 1_000_000_000_000_000_000, // 1 ETH in wei
        expiration: 1800000000,
        chain_id: 1,
        nonce: 0,
    };

    println!("\n  --- Signing Intent ---");
    println!("  Target    : 0x{}", hex::encode(intent.target_contract));
    println!("  Function  : 0x{}", hex::encode(intent.function_sig));
    println!("  Max Value : {} wei", intent.max_value);
    println!("  Expiry    : {}", intent.expiration);
    println!("  Chain ID  : {}", intent.chain_id);
    println!("  Nonce     : {}", intent.nonce);

    let privkey: [u8; 32] = action_key.private_key.as_slice().try_into().unwrap();
    let sig = sign_intent(&intent, &verifying_contract, &privkey);

    println!("\n  --- EIP-712 Signature ---");
    println!("  v : {}", sig.v);
    println!("  r : 0x{}", hex::encode(sig.r));
    println!("  s : 0x{}", hex::encode(sig.s));

    // Verify by recovery
    let recovered = recover_signer(&intent, &verifying_contract, &sig);
    let expected = action_key.eth_address.unwrap();
    println!("\n  --- Verification ---");
    println!("  Recovered : 0x{}", hex::encode(recovered));
    println!("  Expected  : 0x{}", hex::encode(expected));
    println!(
        "  Match     : {}",
        if recovered == expected { "YES" } else { "NO" }
    );

    println!();
}
