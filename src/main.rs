use huntkey::{
    derive_key, generate_mnemonic, root_from_mnemonic,
    KeyHierarchy, KeyRole, SovereignIntent,
    sign_intent, recover_signer, role_path,
    DelegationCertificate, sign_delegation, recover_delegator,
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
    let fn_sig = [0xa9, 0x05, 0x9c, 0xbb]; // transfer(address,uint256)
    let intent = SovereignIntent {
        target_contract: [0xBB; 20],
        function_sig: fn_sig,
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

    let action_privkey: [u8; 32] = action_key.private_key.as_slice().try_into().unwrap();
    let sig = sign_intent(&intent, &verifying_contract, &action_privkey);

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

    // --- Delegated Access Control ---
    println!("\n========================================");
    println!("  Delegated Access Control");
    println!("========================================");

    // Root endorses action key with a delegation certificate
    let cert = DelegationCertificate {
        delegate: action_key.eth_address.unwrap(),
        scope: fn_sig,
        max_value: 2_000_000_000_000_000_000, // 2 ETH cap
        expiration: 1900000000,
        chain_id: 1,
        nonce: 0,
    };

    println!("\n  --- Delegation Certificate ---");
    println!("  Delegate  : 0x{}", hex::encode(cert.delegate));
    println!("  Scope     : 0x{}", hex::encode(cert.scope));
    println!("  Max Value : {} wei", cert.max_value);
    println!("  Expiry    : {}", cert.expiration);
    println!("  Chain ID  : {}", cert.chain_id);
    println!("  Nonce     : {}", cert.nonce);

    let root_privkey: [u8; 32] = root_id.private_key.as_slice().try_into().unwrap();
    let deleg_sig = sign_delegation(&cert, &verifying_contract, &root_privkey);

    println!("\n  --- Delegation Signature (by Root) ---");
    println!("  v : {}", deleg_sig.v);
    println!("  r : 0x{}", hex::encode(deleg_sig.r));
    println!("  s : 0x{}", hex::encode(deleg_sig.s));

    // Verify delegation
    let recovered_root = recover_delegator(
        &deleg_sig.certificate,
        &verifying_contract,
        deleg_sig.v,
        &deleg_sig.r,
        &deleg_sig.s,
    );
    println!("\n  --- Delegation Verification ---");
    println!("  Recovered Root : 0x{}", hex::encode(recovered_root));
    println!("  Expected Root  : 0x{}", hex::encode(root_id.eth_address.unwrap()));
    println!(
        "  Root Match     : {}",
        if recovered_root == root_id.eth_address.unwrap() { "YES" } else { "NO" }
    );

    // Verify the delegation chain: delegate matches intent signer
    println!("\n  --- Delegation Chain ---");
    println!("  Delegate in cert : 0x{}", hex::encode(cert.delegate));
    println!("  Intent signer    : 0x{}", hex::encode(recovered));
    println!(
        "  Chain Valid      : {}",
        if cert.delegate == recovered { "YES" } else { "NO" }
    );

    println!();
}
