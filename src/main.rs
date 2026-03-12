use huntkey::{
    derive_key, generate_mnemonic, root_from_mnemonic,
    KeyHierarchy, KeyRole, SovereignIntent,
    sign_intent, recover_signer, role_path,
    DelegationCertificate, sign_delegation, recover_delegator,
    RecoveryRequest, PendingRecovery,
    sign_recovery_request, recover_recovery_signer,
    derive_session_key, SessionCertificate, sign_session_cert, recover_session_signer,
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
    let call_data = [0xa9, 0x05, 0x9c, 0xbb, 0x00, 0x00]; // sample calldata
    let data_hash = huntkey::call_data_hash(&call_data);
    let intent = SovereignIntent {
        target_contract: [0xBB; 20],
        function_sig: fn_sig,
        recipient: [0xCC; 20],
        asset_address: [0x00; 20],
        call_data_hash: data_hash,
        max_value: 1_000_000_000_000_000_000, // 1 ETH in wei
        expiration: 1800000000,
        chain_id: 1,
        nonce: 0,
        session_epoch: 0,
        gas_limit: 0,
        max_fee_per_gas: 0,
        max_priority_fee_per_gas: 0,
        required_claim: [0x00; 32],
        claim_proof_hash: [0x00; 32],
        paymaster_mode: 0,
        paymaster: [0x00; 20],
    };

    println!("\n  --- Signing Intent (v2) ---");
    println!("  Target      : 0x{}", hex::encode(intent.target_contract));
    println!("  Function    : 0x{}", hex::encode(intent.function_sig));
    println!("  Recipient   : 0x{}", hex::encode(intent.recipient));
    println!("  Asset       : 0x{}", hex::encode(intent.asset_address));
    println!("  CallDataHash: 0x{}", hex::encode(intent.call_data_hash));
    println!("  Max Value   : {} wei", intent.max_value);
    println!("  Expiry      : {}", intent.expiration);
    println!("  Chain ID    : {}", intent.chain_id);
    println!("  Nonce       : {}", intent.nonce);

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

    // --- Social Recovery Demo ---
    println!("\n========================================");
    println!("  Social Recovery");
    println!("========================================");

    // Derive 3 guardian keys from the Recovery role
    let guardian_0 = hierarchy.derive_role(KeyRole::Recovery, 0);
    let guardian_1 = hierarchy.derive_role(KeyRole::Recovery, 1);
    let guardian_2 = hierarchy.derive_role(KeyRole::Recovery, 2);

    println!("\n  Guardians:");
    println!("  [0] 0x{}", hex::encode(guardian_0.eth_address.unwrap()));
    println!("  [1] 0x{}", hex::encode(guardian_1.eth_address.unwrap()));
    println!("  [2] 0x{}", hex::encode(guardian_2.eth_address.unwrap()));

    // Simulate: root is lost, guardians initiate recovery to a new identity
    let new_root_addr = [0xFF; 20]; // placeholder new root
    let recovery_req = RecoveryRequest {
        old_root: root_id.eth_address.unwrap(),
        new_root: new_root_addr,
        chain_id: 1,
        nonce: 0,
    };

    println!("\n  --- Recovery Request ---");
    println!("  Old Root  : 0x{}", hex::encode(recovery_req.old_root));
    println!("  New Root  : 0x{}", hex::encode(recovery_req.new_root));
    println!("  Chain ID  : {}", recovery_req.chain_id);
    println!("  Nonce     : {}", recovery_req.nonce);

    // Guardian 0 signs
    let g0_privkey: [u8; 32] = guardian_0.private_key.as_slice().try_into().unwrap();
    let g0_sig = sign_recovery_request(&recovery_req, &verifying_contract, &g0_privkey);
    let g0_recovered = recover_recovery_signer(&recovery_req, &verifying_contract, &g0_sig);

    // Guardian 1 signs
    let g1_privkey: [u8; 32] = guardian_1.private_key.as_slice().try_into().unwrap();
    let g1_sig = sign_recovery_request(&recovery_req, &verifying_contract, &g1_privkey);
    let g1_recovered = recover_recovery_signer(&recovery_req, &verifying_contract, &g1_sig);

    println!("\n  --- Guardian Signatures ---");
    println!("  Guardian 0 : v={} recovered=0x{}", g0_sig.v, hex::encode(g0_recovered));
    println!("  Guardian 1 : v={} recovered=0x{}", g1_sig.v, hex::encode(g1_recovered));

    // Track locally
    let mut pending = PendingRecovery::new(recovery_req.clone());
    pending.add_approval(g0_recovered);
    pending.add_approval(g1_recovered);

    println!("\n  --- Recovery Status ---");
    println!("  Approvals    : {}/2 threshold", pending.approvals.len());
    println!("  Threshold Met: {}", if pending.threshold_met(2) { "YES" } else { "NO" });
    println!("  Is Alert     : {}", if pending.is_alert(&root_id.eth_address.unwrap()) { "YES" } else { "NO" });

    // Simulate timelock
    pending.initiated_at = Some(1000);
    println!("  Timelock @   : t=1000");
    println!("  Expired @47h : {}", if pending.timelock_expired(1000 + 169200) { "YES" } else { "NO" });
    println!("  Expired @48h : {}", if pending.timelock_expired(1000 + 172800) { "YES" } else { "NO" });

    // --- Ephemeral Session Keys ---
    println!("\n========================================");
    println!("  Ephemeral Session Keys");
    println!("========================================");

    let action_privkey2: [u8; 32] = action_key.private_key.as_slice().try_into().unwrap();
    let session = derive_session_key(&action_privkey2, 0, 1);
    println!("\n  Session Key #0 (derived from Action Key + nonce=0 + chain=1):");
    println!("  Address : 0x{}", hex::encode(session.eth_address));

    let session2 = derive_session_key(&action_privkey2, 1, 1);
    println!("\n  Session Key #1 (derived from Action Key + nonce=1):");
    println!("  Address : 0x{}", hex::encode(session2.eth_address));

    // Action key signs a session certificate
    let target_contract = [0xBB; 20];
    let session_cert = SessionCertificate {
        session: session.eth_address,
        parent: action_key.eth_address.unwrap(),
        scope: fn_sig,
        target: target_contract,
        max_value: 2_000_000_000_000_000_000,
        expiration: 1900000000,
        chain_id: 1,
    };

    println!("\n  --- Session Certificate ---");
    println!("  Session : 0x{}", hex::encode(session_cert.session));
    println!("  Parent  : 0x{}", hex::encode(session_cert.parent));
    println!("  Scope   : 0x{}", hex::encode(session_cert.scope));
    println!("  Target  : 0x{}", hex::encode(session_cert.target));
    println!("  Max Val : {} wei", session_cert.max_value);
    println!("  Expiry  : {}", session_cert.expiration);

    let signed_session = sign_session_cert(&session_cert, &verifying_contract, &action_privkey2);
    println!("\n  --- Session Cert Signature (by Action Key) ---");
    println!("  v : {}", signed_session.v);
    println!("  r : 0x{}", hex::encode(signed_session.r));
    println!("  s : 0x{}", hex::encode(signed_session.s));

    let recovered_parent = recover_session_signer(
        &signed_session.certificate,
        &verifying_contract,
        signed_session.v,
        &signed_session.r,
        &signed_session.s,
    );
    println!("\n  --- Session Cert Verification ---");
    println!("  Recovered Parent : 0x{}", hex::encode(recovered_parent));
    println!("  Expected Parent  : 0x{}", hex::encode(action_key.eth_address.unwrap()));
    println!("  Match            : {}", if recovered_parent == action_key.eth_address.unwrap() { "YES" } else { "NO" });

    // Session key signs an intent
    let session_call_data = [0xa9, 0x05, 0x9c, 0xbb, 0x01, 0x02];
    let session_data_hash = huntkey::call_data_hash(&session_call_data);
    let session_intent = SovereignIntent {
        target_contract: target_contract,
        function_sig: fn_sig,
        recipient: [0xEE; 20],
        asset_address: [0x00; 20],
        call_data_hash: session_data_hash,
        max_value: 1_000_000_000_000_000_000,
        expiration: 1800000000,
        chain_id: 1,
        nonce: 0,
        session_epoch: 0,
        gas_limit: 0,
        max_fee_per_gas: 0,
        max_priority_fee_per_gas: 0,
        required_claim: [0x00; 32],
        claim_proof_hash: [0x00; 32],
        paymaster_mode: 0,
        paymaster: [0x00; 20],
    };

    let session_intent_sig = sign_intent(&session_intent, &verifying_contract, &session.private_key);
    let recovered_session = recover_signer(&session_intent, &verifying_contract, &session_intent_sig);

    println!("\n  --- 3-Layer Signing Chain ---");
    println!("  Root   → Action Key  : 0x{}", hex::encode(action_key.eth_address.unwrap()));
    println!("  Action → Session Key : 0x{}", hex::encode(session.eth_address));
    println!("  Session → Intent     : recovered=0x{}", hex::encode(recovered_session));
    println!("  Chain Valid          : {}", if recovered_session == session.eth_address { "YES" } else { "NO" });

    println!();
}
