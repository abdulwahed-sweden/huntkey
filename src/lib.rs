//! HuntKey — Sovereign Identity Protocol
//!
//! A policy-enforced identity protocol where the master key never touches the
//! network. Ephemeral action keys handle constrained on-chain operations,
//! verified through EIP-712 typed structured data signing.
#![deny(missing_docs)]

/// Core primitives: key derivation, hashing, and ABI encoding utilities.
pub mod core;
/// EIP-712 intent signing, delegation certificates, and verification.
pub mod intents;
/// Social recovery: guardian threshold signing and pending recovery tracking.
pub mod recovery;
/// Ephemeral session keys: HKDF derivation and session certificate signing.
pub mod sessions;
/// WASM bindings for TypeScript/JavaScript interop (feature-gated).
pub mod wasm_api;

// Re-export all public types and functions at the crate root for convenience.

pub use crate::core::{
    call_data_hash, derive_key, domain_separator, eth_address, generate_mnemonic, role_path,
    root_from_mnemonic, DerivedKey, KeyHierarchy, KeyRole, Mnemonic,
};
pub use coins_bip32;

pub use intents::{
    delegation_signing_hash, delegation_struct_hash, intent_signing_hash, intent_struct_hash,
    recover_delegator, recover_signer, sign_delegation, sign_intent, DelegationCertificate,
    IntentSignature, SignedDelegation, SovereignIntent,
};

pub use recovery::{
    recover_recovery_signer, recovery_signing_hash, recovery_struct_hash, sign_recovery_request,
    PendingRecovery, RecoveryRequest,
};

pub use sessions::{
    derive_session_key, recover_session_signer, session_signing_hash, session_struct_hash,
    sign_session_cert, SessionCertificate, SessionKey, SignedSession,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn test_mnemonic() -> Mnemonic {
        Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap()
    }

    fn test_root() -> coins_bip32::prelude::XPriv {
        root_from_mnemonic(&test_mnemonic())
    }

    #[test]
    fn mnemonic_produces_valid_seed() {
        let seed = test_mnemonic().to_seed("");
        assert_eq!(seed.len(), 64);
    }

    #[test]
    fn master_key_from_seed() {
        let root = test_root();
        let signing_key: &k256::ecdsa::SigningKey = root.as_ref();
        let bytes = signing_key.to_bytes();
        assert_eq!(bytes.len(), 32);
        assert!(bytes.iter().any(|&b| b != 0));
    }

    #[test]
    fn btc_derivation_path() {
        let root = test_root();
        let dk = derive_key(&root, "m/44'/0'/0'/0/0", false);
        assert_eq!(
            hex::encode(&dk.private_key),
            "e284129cc0922579a535bbf4d1a3b25773090d28c909bc0fed73b5e0222cc372"
        );
    }

    #[test]
    fn eth_derivation_path() {
        let root = test_root();
        let dk = derive_key(&root, "m/44'/60'/0'/0/0", true);
        assert_eq!(
            hex::encode(&dk.private_key),
            "1ab42cc412b618bdea3a599e3c9bae199ebf030895b039e9db1e30dafb12b727"
        );
    }

    #[test]
    fn eth_address_derivation() {
        let root = test_root();
        let dk = derive_key(&root, "m/44'/60'/0'/0/0", true);
        assert_eq!(
            hex::encode(dk.eth_address.unwrap()),
            "9858effd232b4033e47d90003d41ec34ecaeda94"
        );
    }

    #[test]
    fn distinct_keys_per_index() {
        let root = test_root();
        let k0 = derive_key(&root, "m/44'/60'/0'/0/0", true);
        let k1 = derive_key(&root, "m/44'/60'/0'/0/1", true);
        assert_ne!(k0.private_key, k1.private_key);
    }

    #[test]
    fn random_mnemonic_is_12_words() {
        let m = generate_mnemonic(12);
        assert_eq!(m.words().count(), 12);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn roundtrip_mnemonic_to_key_is_deterministic(seed_idx in 0u64..1000u64) {
            let mut entropy = [0u8; 16];
            entropy[..8].copy_from_slice(&seed_idx.to_le_bytes());
            let mnemonic = Mnemonic::from_entropy(&entropy).unwrap();

            let root1 = root_from_mnemonic(&mnemonic);
            let root2 = root_from_mnemonic(&mnemonic);

            let k1 = derive_key(&root1, "m/44'/60'/0'/0/0", true);
            let k2 = derive_key(&root2, "m/44'/60'/0'/0/0", true);

            prop_assert_eq!(&k1.private_key, &k2.private_key);
            prop_assert_eq!(k1.eth_address, k2.eth_address);
        }

        #[test]
        fn different_entropy_yields_different_keys(a in 0u64..1000u64, b in 0u64..1000u64) {
            prop_assume!(a != b);
            let mut ent_a = [0u8; 16];
            let mut ent_b = [0u8; 16];
            ent_a[..8].copy_from_slice(&a.to_le_bytes());
            ent_b[..8].copy_from_slice(&b.to_le_bytes());

            let m_a = Mnemonic::from_entropy(&ent_a).unwrap();
            let m_b = Mnemonic::from_entropy(&ent_b).unwrap();

            let k_a = derive_key(&root_from_mnemonic(&m_a), "m/44'/60'/0'/0/0", true);
            let k_b = derive_key(&root_from_mnemonic(&m_b), "m/44'/60'/0'/0/0", true);

            prop_assert_ne!(&k_a.private_key, &k_b.private_key);
        }
    }
}

#[cfg(test)]
mod sovereign_tests {
    use super::*;
    use proptest::prelude::*;
    use std::str::FromStr;

    fn test_root() -> coins_bip32::prelude::XPriv {
        let m = Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        root_from_mnemonic(&m)
    }

    #[test]
    fn test_key_role_paths() {
        assert_eq!(role_path(KeyRole::RootIdentity, 0), "m/999'/0'");
        assert_eq!(role_path(KeyRole::Action, 0), "m/999'/1'/0");
        assert_eq!(role_path(KeyRole::Action, 5), "m/999'/1'/5");
        assert_eq!(role_path(KeyRole::Proof, 0), "m/999'/2'/0");
        assert_eq!(role_path(KeyRole::Recovery, 0), "m/999'/3'/0");
    }

    #[test]
    fn test_each_role_derives_different_key() {
        let root = test_root();
        let hierarchy = KeyHierarchy::new(root);

        let root_key = hierarchy.derive_role(KeyRole::RootIdentity, 0);
        let action_key = hierarchy.derive_role(KeyRole::Action, 0);
        let proof_key = hierarchy.derive_role(KeyRole::Proof, 0);
        let recovery_key = hierarchy.derive_role(KeyRole::Recovery, 0);

        let keys = [
            &root_key.private_key,
            &action_key.private_key,
            &proof_key.private_key,
            &recovery_key.private_key,
        ];
        for i in 0..keys.len() {
            for j in (i + 1)..keys.len() {
                assert_ne!(keys[i], keys[j], "roles {} and {} must produce different keys", i, j);
            }
        }
    }

    #[test]
    fn test_action_key_auto_increment() {
        let root = test_root();
        let mut hierarchy = KeyHierarchy::new(root);

        let k0 = hierarchy.next_action_key();
        let k1 = hierarchy.next_action_key();
        let k2 = hierarchy.next_action_key();

        assert_eq!(hierarchy.action_index(), 3);
        assert_ne!(k0.private_key, k1.private_key);
        assert_ne!(k1.private_key, k2.private_key);
        assert_eq!(k0.path, "m/999'/1'/0");
        assert_eq!(k1.path, "m/999'/1'/1");
        assert_eq!(k2.path, "m/999'/1'/2");
    }

    #[test]
    fn test_eip712_hash_deterministic() {
        let contract = [0xAA; 20];
        let intent = SovereignIntent {
            target_contract: [0xBB; 20],
            function_sig: [0xa9, 0x05, 0x9c, 0xbb],
            recipient: [0x00; 20],
            asset_address: [0x00; 20],
            call_data_hash: [0x00; 32],
            max_value: 1_000_000,
            expiration: 1700000000,
            chain_id: 1,
            nonce: 0,
        };

        let h1 = intent_signing_hash(&intent, &contract);
        let h2 = intent_signing_hash(&intent, &contract);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_sign_and_recover_roundtrip() {
        let root = test_root();
        let mut hierarchy = KeyHierarchy::new(root);
        let action_key = hierarchy.next_action_key();

        let verifying_contract = [0xCC; 20];
        let intent = SovereignIntent {
            target_contract: [0xDD; 20],
            function_sig: [0xa9, 0x05, 0x9c, 0xbb],
            recipient: [0x00; 20],
            asset_address: [0x00; 20],
            call_data_hash: [0x00; 32],
            max_value: 500_000,
            expiration: 1800000000,
            chain_id: 1,
            nonce: 42,
        };

        let privkey: [u8; 32] = action_key.private_key.as_slice().try_into().unwrap();
        let sig = sign_intent(&intent, &verifying_contract, &privkey);

        assert!(sig.v == 27 || sig.v == 28);

        let recovered = recover_signer(&intent, &verifying_contract, &sig);
        assert_eq!(recovered, action_key.eth_address.unwrap());
    }

    #[test]
    fn test_delegation_certificate_sign_and_recover() {
        let root = test_root();
        let hierarchy = KeyHierarchy::new(root);

        let root_id = hierarchy.derive_role(KeyRole::RootIdentity, 0);
        let action_key = hierarchy.derive_role(KeyRole::Action, 0);

        let verifying_contract = [0xCC; 20];
        let cert = DelegationCertificate {
            delegate: action_key.eth_address.unwrap(),
            scope: [0xa9, 0x05, 0x9c, 0xbb],
            max_value: 1_000_000,
            expiration: 1900000000,
            chain_id: 1,
            nonce: 0,
        };

        let root_privkey: [u8; 32] = root_id.private_key.as_slice().try_into().unwrap();
        let signed = sign_delegation(&cert, &verifying_contract, &root_privkey);

        let recovered = recover_delegator(
            &signed.certificate,
            &verifying_contract,
            signed.v,
            &signed.r,
            &signed.s,
        );
        assert_eq!(recovered, root_id.eth_address.unwrap());
    }

    #[test]
    fn test_delegation_hash_deterministic() {
        let contract = [0xAA; 20];
        let cert = DelegationCertificate {
            delegate: [0xBB; 20],
            scope: [0xa9, 0x05, 0x9c, 0xbb],
            max_value: 500_000,
            expiration: 1800000000,
            chain_id: 1,
            nonce: 0,
        };

        let h1 = delegation_signing_hash(&cert, &contract);
        let h2 = delegation_signing_hash(&cert, &contract);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_delegation_chain_id_binding() {
        let contract = [0xAA; 20];
        let cert1 = DelegationCertificate {
            delegate: [0xBB; 20],
            scope: [0xa9, 0x05, 0x9c, 0xbb],
            max_value: 500_000,
            expiration: 1800000000,
            chain_id: 1,
            nonce: 0,
        };
        let cert2 = DelegationCertificate {
            chain_id: 137, // Polygon
            ..cert1.clone()
        };

        let h1 = delegation_signing_hash(&cert1, &contract);
        let h2 = delegation_signing_hash(&cert2, &contract);
        assert_ne!(h1, h2, "different chain_ids must produce different hashes");
    }

    #[test]
    fn test_full_delegation_flow() {
        // Root creates hierarchy, derives action key, signs delegation, action key signs intent
        let root = test_root();
        let mut hierarchy = KeyHierarchy::new(root);

        let root_id = hierarchy.derive_role(KeyRole::RootIdentity, 0);
        let action_key = hierarchy.next_action_key();

        let verifying_contract = [0xCC; 20];
        let fn_sig = [0xa9, 0x05, 0x9c, 0xbb];

        // Step 1: Root endorses action key
        let cert = DelegationCertificate {
            delegate: action_key.eth_address.unwrap(),
            scope: fn_sig,
            max_value: 2_000_000_000_000_000_000, // 2 ETH
            expiration: 1900000000,
            chain_id: 1,
            nonce: 0,
        };

        let root_privkey: [u8; 32] = root_id.private_key.as_slice().try_into().unwrap();
        let signed_deleg = sign_delegation(&cert, &verifying_contract, &root_privkey);

        // Verify delegation recovery
        let recovered_root = recover_delegator(
            &signed_deleg.certificate,
            &verifying_contract,
            signed_deleg.v,
            &signed_deleg.r,
            &signed_deleg.s,
        );
        assert_eq!(recovered_root, root_id.eth_address.unwrap());

        // Step 2: Action key signs intent within delegation scope
        let intent = SovereignIntent {
            target_contract: [0xDD; 20],
            function_sig: fn_sig,
            recipient: [0x00; 20],
            asset_address: [0x00; 20],
            call_data_hash: [0x00; 32],
            max_value: 1_000_000_000_000_000_000, // 1 ETH (within 2 ETH cap)
            expiration: 1800000000,
            chain_id: 1,
            nonce: 0,
        };

        let action_privkey: [u8; 32] = action_key.private_key.as_slice().try_into().unwrap();
        let intent_sig = sign_intent(&intent, &verifying_contract, &action_privkey);

        let recovered_action = recover_signer(&intent, &verifying_contract, &intent_sig);
        assert_eq!(recovered_action, action_key.eth_address.unwrap());

        // Verify delegation delegate matches intent signer
        assert_eq!(signed_deleg.certificate.delegate, recovered_action);
    }

    #[test]
    fn test_recovery_request_sign_and_recover() {
        let root = test_root();
        let hierarchy = KeyHierarchy::new(root);
        // Use recovery key #0 as a "guardian"
        let guardian = hierarchy.derive_role(KeyRole::Recovery, 0);

        let verifying_contract = [0xCC; 20];
        let req = RecoveryRequest {
            old_root: [0xAA; 20],
            new_root: [0xBB; 20],
            chain_id: 1,
            nonce: 0,
        };

        let guardian_privkey: [u8; 32] = guardian.private_key.as_slice().try_into().unwrap();
        let sig = sign_recovery_request(&req, &verifying_contract, &guardian_privkey);

        let recovered = recover_recovery_signer(&req, &verifying_contract, &sig);
        assert_eq!(recovered, guardian.eth_address.unwrap());
    }

    #[test]
    fn test_recovery_hash_chain_id_binding() {
        let contract = [0xAA; 20];
        let req1 = RecoveryRequest {
            old_root: [0xBB; 20],
            new_root: [0xCC; 20],
            chain_id: 1,
            nonce: 0,
        };
        let req2 = RecoveryRequest {
            chain_id: 137,
            ..req1.clone()
        };

        let h1 = recovery_signing_hash(&req1, &contract);
        let h2 = recovery_signing_hash(&req2, &contract);
        assert_ne!(h1, h2, "different chain_ids must produce different recovery hashes");
    }

    #[test]
    fn test_recovery_hash_nonce_binding() {
        let contract = [0xAA; 20];
        let req1 = RecoveryRequest {
            old_root: [0xBB; 20],
            new_root: [0xCC; 20],
            chain_id: 1,
            nonce: 0,
        };
        let req2 = RecoveryRequest {
            nonce: 1,
            ..req1.clone()
        };

        let h1 = recovery_signing_hash(&req1, &contract);
        let h2 = recovery_signing_hash(&req2, &contract);
        assert_ne!(h1, h2, "different nonces must produce different recovery hashes");
    }

    #[test]
    fn test_pending_recovery_tracking() {
        let our_root = [0xAA; 20];
        let new_root = [0xBB; 20];
        let req = RecoveryRequest {
            old_root: our_root,
            new_root,
            chain_id: 1,
            nonce: 0,
        };

        let mut pending = PendingRecovery::new(req);
        assert!(pending.is_alert(&our_root));
        assert!(!pending.threshold_met(2));
        assert!(!pending.timelock_expired(1000));

        // Add first guardian approval
        let g1 = [0x11; 20];
        assert!(pending.add_approval(g1));
        assert!(!pending.threshold_met(2));

        // Duplicate approval rejected
        assert!(!pending.add_approval(g1));

        // Second guardian reaches threshold
        let g2 = [0x22; 20];
        assert!(pending.add_approval(g2));
        assert!(pending.threshold_met(2));

        // Simulate timelock start
        pending.initiated_at = Some(1000);
        assert!(!pending.timelock_expired(1000 + 172799)); // 1 second before
        assert!(pending.timelock_expired(1000 + 172800));  // exactly 48h
    }

    #[test]
    fn test_full_recovery_flow() {
        // Simulate: root is lost, 2 of 3 guardians initiate recovery, sign the request
        let root = test_root();
        let hierarchy = KeyHierarchy::new(root);

        let old_root_id = hierarchy.derive_role(KeyRole::RootIdentity, 0);
        let guardian_0 = hierarchy.derive_role(KeyRole::Recovery, 0);
        let guardian_1 = hierarchy.derive_role(KeyRole::Recovery, 1);

        // New identity from a different mnemonic
        let new_mnemonic = Mnemonic::from_str(
            "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong",
        ).unwrap();
        let new_hierarchy = KeyHierarchy::new(root_from_mnemonic(&new_mnemonic));
        let new_root_id = new_hierarchy.derive_role(KeyRole::RootIdentity, 0);

        let verifying_contract = [0xCC; 20];
        let req = RecoveryRequest {
            old_root: old_root_id.eth_address.unwrap(),
            new_root: new_root_id.eth_address.unwrap(),
            chain_id: 1,
            nonce: 0,
        };

        // Guardian 0 signs
        let g0_privkey: [u8; 32] = guardian_0.private_key.as_slice().try_into().unwrap();
        let g0_sig = sign_recovery_request(&req, &verifying_contract, &g0_privkey);
        let g0_recovered = recover_recovery_signer(&req, &verifying_contract, &g0_sig);
        assert_eq!(g0_recovered, guardian_0.eth_address.unwrap());

        // Guardian 1 signs
        let g1_privkey: [u8; 32] = guardian_1.private_key.as_slice().try_into().unwrap();
        let g1_sig = sign_recovery_request(&req, &verifying_contract, &g1_privkey);
        let g1_recovered = recover_recovery_signer(&req, &verifying_contract, &g1_sig);
        assert_eq!(g1_recovered, guardian_1.eth_address.unwrap());

        // Different guardians signed
        assert_ne!(g0_recovered, g1_recovered);

        // Track locally
        let mut pending = PendingRecovery::new(req);
        pending.add_approval(g0_recovered);
        pending.add_approval(g1_recovered);
        assert!(pending.threshold_met(2));
    }

    proptest! {
        #[test]
        fn prop_random_intents_produce_recoverable_signatures(
            max_val in 0u128..u128::MAX,
            exp in 0u64..u64::MAX,
            nonce in 0u64..10000u64,
        ) {
            let root = test_root();
            let hierarchy = KeyHierarchy::new(root);
            let action_key = hierarchy.derive_role(KeyRole::Action, 0);

            let verifying_contract = [0x11; 20];
            let intent = SovereignIntent {
                target_contract: [0x22; 20],
                function_sig: [0xa9, 0x05, 0x9c, 0xbb],
                recipient: [0x00; 20],
                asset_address: [0x00; 20],
                call_data_hash: [0x00; 32],
                max_value: max_val,
                expiration: exp,
                chain_id: 1,
                nonce,
            };

            let privkey: [u8; 32] = action_key.private_key.as_slice().try_into().unwrap();
            let sig = sign_intent(&intent, &verifying_contract, &privkey);
            let recovered = recover_signer(&intent, &verifying_contract, &sig);
            prop_assert_eq!(recovered, action_key.eth_address.unwrap());
        }

        #[test]
        fn prop_delegation_always_recoverable(
            max_val in 0u128..u128::MAX,
            exp in 0u64..u64::MAX,
            nonce in 0u64..10000u64,
        ) {
            let root = test_root();
            let hierarchy = KeyHierarchy::new(root);
            let root_id = hierarchy.derive_role(KeyRole::RootIdentity, 0);
            let action_key = hierarchy.derive_role(KeyRole::Action, 0);

            let verifying_contract = [0x11; 20];
            let cert = DelegationCertificate {
                delegate: action_key.eth_address.unwrap(),
                scope: [0xa9, 0x05, 0x9c, 0xbb],
                max_value: max_val,
                expiration: exp,
                chain_id: 1,
                nonce,
            };

            let root_privkey: [u8; 32] = root_id.private_key.as_slice().try_into().unwrap();
            let signed = sign_delegation(&cert, &verifying_contract, &root_privkey);
            let recovered = recover_delegator(
                &signed.certificate, &verifying_contract,
                signed.v, &signed.r, &signed.s,
            );
            prop_assert_eq!(recovered, root_id.eth_address.unwrap());
        }

        #[test]
        fn prop_recovery_always_recoverable(
            nonce in 0u64..10000u64,
        ) {
            let root = test_root();
            let hierarchy = KeyHierarchy::new(root);
            let guardian = hierarchy.derive_role(KeyRole::Recovery, 0);

            let verifying_contract = [0x11; 20];
            let req = RecoveryRequest {
                old_root: [0xAA; 20],
                new_root: [0xBB; 20],
                chain_id: 1,
                nonce,
            };

            let guardian_privkey: [u8; 32] = guardian.private_key.as_slice().try_into().unwrap();
            let sig = sign_recovery_request(&req, &verifying_contract, &guardian_privkey);
            let recovered = recover_recovery_signer(&req, &verifying_contract, &sig);
            prop_assert_eq!(recovered, guardian.eth_address.unwrap());
        }

        #[test]
        fn prop_session_cert_always_recoverable(
            nonce in 0u64..10000u64,
        ) {
            let root = test_root();
            let hierarchy = KeyHierarchy::new(root);
            let action_key = hierarchy.derive_role(KeyRole::Action, 0);

            let action_privkey: [u8; 32] = action_key.private_key.as_slice().try_into().unwrap();
            let session = derive_session_key(&action_privkey, nonce);

            let verifying_contract = [0x11; 20];
            let cert = SessionCertificate {
                session: session.eth_address,
                parent: action_key.eth_address.unwrap(),
                scope: [0xa9, 0x05, 0x9c, 0xbb],
                target: [0xDD; 20],
                max_value: 1_000_000,
                expiration: 1900000000,
                chain_id: 1,
            };

            let signed = sign_session_cert(&cert, &verifying_contract, &action_privkey);
            let recovered = recover_session_signer(
                &signed.certificate, &verifying_contract,
                signed.v, &signed.r, &signed.s,
            );
            prop_assert_eq!(recovered, action_key.eth_address.unwrap());
        }
    }

    // --- Session key tests ---

    #[test]
    fn test_derive_session_key_deterministic() {
        let root = test_root();
        let hierarchy = KeyHierarchy::new(root);
        let action_key = hierarchy.derive_role(KeyRole::Action, 0);
        let action_privkey: [u8; 32] = action_key.private_key.as_slice().try_into().unwrap();

        let s1 = derive_session_key(&action_privkey, 0);
        let s2 = derive_session_key(&action_privkey, 0);
        assert_eq!(s1.eth_address, s2.eth_address);
        assert_eq!(s1.private_key, s2.private_key);
    }

    #[test]
    fn test_derive_session_key_different_nonces() {
        let root = test_root();
        let hierarchy = KeyHierarchy::new(root);
        let action_key = hierarchy.derive_role(KeyRole::Action, 0);
        let action_privkey: [u8; 32] = action_key.private_key.as_slice().try_into().unwrap();

        let s0 = derive_session_key(&action_privkey, 0);
        let s1 = derive_session_key(&action_privkey, 1);
        let s2 = derive_session_key(&action_privkey, 2);

        assert_ne!(s0.eth_address, s1.eth_address);
        assert_ne!(s1.eth_address, s2.eth_address);
        assert_ne!(s0.eth_address, s2.eth_address);
    }

    #[test]
    fn test_session_cert_sign_and_recover() {
        let root = test_root();
        let hierarchy = KeyHierarchy::new(root);
        let action_key = hierarchy.derive_role(KeyRole::Action, 0);
        let action_privkey: [u8; 32] = action_key.private_key.as_slice().try_into().unwrap();

        let session = derive_session_key(&action_privkey, 0);

        let verifying_contract = [0xCC; 20];
        let cert = SessionCertificate {
            session: session.eth_address,
            parent: action_key.eth_address.unwrap(),
            scope: [0xa9, 0x05, 0x9c, 0xbb],
            target: [0xDD; 20],
            max_value: 1_000_000,
            expiration: 1900000000,
            chain_id: 1,
        };

        let signed = sign_session_cert(&cert, &verifying_contract, &action_privkey);
        let recovered = recover_session_signer(
            &signed.certificate,
            &verifying_contract,
            signed.v,
            &signed.r,
            &signed.s,
        );
        assert_eq!(recovered, action_key.eth_address.unwrap());
    }

    #[test]
    fn test_session_key_signs_intent() {
        // Full 3-layer chain: Root → Action → Session → Intent
        let root = test_root();
        let hierarchy = KeyHierarchy::new(root);
        let action_key = hierarchy.derive_role(KeyRole::Action, 0);
        let action_privkey: [u8; 32] = action_key.private_key.as_slice().try_into().unwrap();

        // Derive session key
        let session = derive_session_key(&action_privkey, 42);

        let verifying_contract = [0xCC; 20];
        let fn_sig = [0xa9, 0x05, 0x9c, 0xbb];
        let target = [0xDD; 20];

        // Action key signs session certificate
        let cert = SessionCertificate {
            session: session.eth_address,
            parent: action_key.eth_address.unwrap(),
            scope: fn_sig,
            target,
            max_value: 2_000_000,
            expiration: 1900000000,
            chain_id: 1,
        };
        let signed_session = sign_session_cert(&cert, &verifying_contract, &action_privkey);

        // Session key signs intent
        let intent = SovereignIntent {
            target_contract: target,
            function_sig: fn_sig,
            recipient: [0x00; 20],
            asset_address: [0x00; 20],
            call_data_hash: [0x00; 32],
            max_value: 1_000_000,
            expiration: 1800000000,
            chain_id: 1,
            nonce: 0,
        };
        let intent_sig = sign_intent(&intent, &verifying_contract, &session.private_key);

        // Verify chain
        let recovered_parent = recover_session_signer(
            &signed_session.certificate,
            &verifying_contract,
            signed_session.v,
            &signed_session.r,
            &signed_session.s,
        );
        assert_eq!(recovered_parent, action_key.eth_address.unwrap());

        let recovered_session = recover_signer(&intent, &verifying_contract, &intent_sig);
        assert_eq!(recovered_session, session.eth_address);

        // Session address in cert matches intent signer
        assert_eq!(signed_session.certificate.session, recovered_session);
    }

    #[test]
    fn test_session_hash_deterministic() {
        let contract = [0xAA; 20];
        let cert = SessionCertificate {
            session: [0xBB; 20],
            parent: [0xCC; 20],
            scope: [0xa9, 0x05, 0x9c, 0xbb],
            target: [0xDD; 20],
            max_value: 500_000,
            expiration: 1800000000,
            chain_id: 1,
        };

        let h1 = session_signing_hash(&cert, &contract);
        let h2 = session_signing_hash(&cert, &contract);
        assert_eq!(h1, h2);
    }
}
