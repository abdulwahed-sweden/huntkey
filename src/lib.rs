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
/// Identity monitoring, event tracking, and security alerting.
pub mod monitor;
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
    IntentSignature, PackedUserOperation, SignedDelegation, SovereignIntent, UserOperationBuilder,
};

pub use recovery::{
    recover_recovery_signer, recovery_signing_hash, recovery_struct_hash, sign_recovery_request,
    PendingRecovery, RecoveryRequest,
};

pub use monitor::{
    AlertCategory, AlertSeverity, EventLog, EventLogEntry, EventType, GuardianNotification,
    IdentityWatcher, SecurityAlert, WatcherConfig,
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
            session_epoch: 0,
            gas_limit: 0,
            max_fee_per_gas: 0,
            max_priority_fee_per_gas: 0,
            required_claim: [0x00; 32],
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
            session_epoch: 0,
            gas_limit: 0,
            max_fee_per_gas: 0,
            max_priority_fee_per_gas: 0,
            required_claim: [0x00; 32],
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
            session_epoch: 0,
            gas_limit: 0,
            max_fee_per_gas: 0,
            max_priority_fee_per_gas: 0,
            required_claim: [0x00; 32],
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
                session_epoch: 0,
                gas_limit: 0,
                max_fee_per_gas: 0,
                max_priority_fee_per_gas: 0,
                required_claim: [0x00; 32],
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
            let session = derive_session_key(&action_privkey, nonce, 1);

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

        let s1 = derive_session_key(&action_privkey, 0, 1);
        let s2 = derive_session_key(&action_privkey, 0, 1);
        assert_eq!(s1.eth_address, s2.eth_address);
        assert_eq!(s1.private_key, s2.private_key);
    }

    #[test]
    fn test_derive_session_key_different_nonces() {
        let root = test_root();
        let hierarchy = KeyHierarchy::new(root);
        let action_key = hierarchy.derive_role(KeyRole::Action, 0);
        let action_privkey: [u8; 32] = action_key.private_key.as_slice().try_into().unwrap();

        let s0 = derive_session_key(&action_privkey, 0, 1);
        let s1 = derive_session_key(&action_privkey, 1, 1);
        let s2 = derive_session_key(&action_privkey, 2, 1);

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

        let session = derive_session_key(&action_privkey, 0, 1);

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
        let session = derive_session_key(&action_privkey, 42, 1);

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
            session_epoch: 0,
            gas_limit: 0,
            max_fee_per_gas: 0,
            max_priority_fee_per_gas: 0,
            required_claim: [0x00; 32],
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

    #[test]
    fn test_hkdf_chain_id_isolation() {
        // Same action key + nonce on different chains must produce different session keys
        let root = test_root();
        let hierarchy = KeyHierarchy::new(root);
        let action_key = hierarchy.derive_role(KeyRole::Action, 0);
        let action_privkey: [u8; 32] = action_key.private_key.as_slice().try_into().unwrap();

        let s_mainnet = derive_session_key(&action_privkey, 0, 1);   // Ethereum mainnet
        let s_polygon = derive_session_key(&action_privkey, 0, 137); // Polygon
        let s_arb     = derive_session_key(&action_privkey, 0, 42161); // Arbitrum

        assert_ne!(s_mainnet.eth_address, s_polygon.eth_address,
            "same key+nonce on different chains must produce different session keys");
        assert_ne!(s_mainnet.eth_address, s_arb.eth_address);
        assert_ne!(s_polygon.eth_address, s_arb.eth_address);

        // Same chain must still be deterministic
        let s_mainnet2 = derive_session_key(&action_privkey, 0, 1);
        assert_eq!(s_mainnet.eth_address, s_mainnet2.eth_address);
    }

    #[test]
    fn test_end_to_end_protocol_flow() {
        // Full protocol simulation: mnemonic → hierarchy → delegation → session → intent
        // Verifies the complete 4-layer signing chain matches the protocol spec.
        let mnemonic = Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        ).unwrap();
        let root = root_from_mnemonic(&mnemonic);
        let mut hierarchy = KeyHierarchy::new(root);

        // Layer 1: Root Identity (cold, never on-chain)
        let root_id = hierarchy.derive_role(KeyRole::RootIdentity, 0);
        let root_privkey: [u8; 32] = root_id.private_key.as_slice().try_into().unwrap();

        // Layer 1b: Recovery guardians
        let g0 = hierarchy.derive_role(KeyRole::Recovery, 0);
        let g1 = hierarchy.derive_role(KeyRole::Recovery, 1);
        let g2 = hierarchy.derive_role(KeyRole::Recovery, 2);
        // All guardians are distinct
        assert_ne!(g0.eth_address, g1.eth_address);
        assert_ne!(g1.eth_address, g2.eth_address);

        // Layer 2: Action key (warm)
        let action_key = hierarchy.next_action_key();
        let action_privkey: [u8; 32] = action_key.private_key.as_slice().try_into().unwrap();

        let verifying_contract = [0xAA; 20];
        let fn_sig = [0xa9, 0x05, 0x9c, 0xbb]; // transfer(address,uint256)

        // Root signs delegation certificate for action key
        let delegation = DelegationCertificate {
            delegate: action_key.eth_address.unwrap(),
            scope: fn_sig,
            max_value: 5_000_000_000_000_000_000, // 5 ETH
            expiration: 2000000000,
            chain_id: 1,
            nonce: 0,
        };
        let signed_deleg = sign_delegation(&delegation, &verifying_contract, &root_privkey);
        let recovered_root = recover_delegator(
            &signed_deleg.certificate, &verifying_contract,
            signed_deleg.v, &signed_deleg.r, &signed_deleg.s,
        );
        assert_eq!(recovered_root, root_id.eth_address.unwrap(),
            "delegation must recover to root identity");

        // Layer 3: Ephemeral session key (HKDF-derived)
        let session = derive_session_key(&action_privkey, 0, 1);
        let target = [0xBB; 20];

        // Action key signs session certificate
        let session_cert = SessionCertificate {
            session: session.eth_address,
            parent: action_key.eth_address.unwrap(),
            scope: fn_sig,
            target,
            max_value: 2_000_000_000_000_000_000, // 2 ETH
            expiration: 1900000000,
            chain_id: 1,
        };
        let signed_sess = sign_session_cert(&session_cert, &verifying_contract, &action_privkey);
        let recovered_parent = recover_session_signer(
            &signed_sess.certificate, &verifying_contract,
            signed_sess.v, &signed_sess.r, &signed_sess.s,
        );
        assert_eq!(recovered_parent, action_key.eth_address.unwrap(),
            "session cert must recover to action key");

        // Layer 4: Session key signs intent
        let call_data = [0xa9, 0x05, 0x9c, 0xbb, 0x00, 0x01, 0x02, 0x03];
        let data_hash = call_data_hash(&call_data);
        let intent = SovereignIntent {
            target_contract: target,
            function_sig: fn_sig,
            recipient: [0xCC; 20],
            asset_address: [0x00; 20],
            call_data_hash: data_hash,
            max_value: 1_000_000_000_000_000_000, // 1 ETH
            expiration: 1800000000,
            chain_id: 1,
            nonce: 0,
            session_epoch: 0,
            gas_limit: 100_000,
            max_fee_per_gas: 50_000_000_000, // 50 gwei
            max_priority_fee_per_gas: 2_000_000_000, // 2 gwei tip
            required_claim: [0x00; 32],
        };
        let intent_sig = sign_intent(&intent, &verifying_contract, &session.private_key);
        let recovered_session = recover_signer(&intent, &verifying_contract, &intent_sig);
        assert_eq!(recovered_session, session.eth_address,
            "intent must recover to session key");

        // Verify the complete chain links
        assert_eq!(signed_deleg.certificate.delegate, action_key.eth_address.unwrap(),
            "delegation.delegate == action key");
        assert_eq!(signed_sess.certificate.parent, action_key.eth_address.unwrap(),
            "session.parent == action key");
        assert_eq!(signed_sess.certificate.session, recovered_session,
            "session cert.session == intent signer");

        // Verify scope enforcement constraints
        assert_eq!(intent.function_sig, session_cert.scope,
            "intent selector must match session scope");
        assert_eq!(intent.target_contract, session_cert.target,
            "intent target must match session target");
        assert!(intent.max_value <= session_cert.max_value,
            "intent value must not exceed session cap");
        assert!(session_cert.max_value <= delegation.max_value,
            "session cap must not exceed delegation cap");

        // Verify calldata hash binding
        let recomputed_hash = call_data_hash(&call_data);
        assert_eq!(intent.call_data_hash, recomputed_hash,
            "calldata hash must match");

        // Verify cross-chain isolation
        let session_polygon = derive_session_key(&action_privkey, 0, 137);
        assert_ne!(session.eth_address, session_polygon.eth_address,
            "same nonce on different chain must produce different session key");
    }
}

#[cfg(test)]
mod monitor_tests {
    use super::*;

    #[test]
    fn test_watcher_recovery_known_guardian() {
        let mut watcher = IdentityWatcher::new();
        let identity = [0xAA; 20];
        let guardian = [0xBB; 20];

        watcher.register_guardians(identity, vec![guardian, [0xCC; 20], [0xDD; 20]]);

        let alert = watcher.on_recovery_state_changed(
            identity, "RecoveryPending", Some(guardian), 100, 1000,
        );

        assert_eq!(alert.severity, AlertSeverity::Warning);
        assert_eq!(alert.category, AlertCategory::RecoveryInitiated);
        assert!(alert.message.contains("known guardian"));
    }

    #[test]
    fn test_watcher_recovery_unknown_guardian() {
        let mut watcher = IdentityWatcher::new();
        let identity = [0xAA; 20];
        let unknown = [0xFF; 20];

        watcher.register_guardians(identity, vec![[0xBB; 20], [0xCC; 20], [0xDD; 20]]);

        let alert = watcher.on_recovery_state_changed(
            identity, "RecoveryPending", Some(unknown), 100, 1000,
        );

        assert_eq!(alert.severity, AlertSeverity::Critical);
        assert!(alert.message.contains("UNKNOWN guardian"));
    }

    #[test]
    fn test_watcher_delegation_known_vs_unknown() {
        let mut watcher = IdentityWatcher::new();
        let identity = [0xAA; 20];
        let known_delegate = [0xBB; 20];
        let unknown_delegate = [0xFF; 20];

        watcher.register_delegate(identity, known_delegate);

        let alert_known = watcher.on_delegation_endorsed(
            identity, known_delegate, [0xa9, 0x05, 0x9c, 0xbb], 100, 1000,
        );
        assert_eq!(alert_known.severity, AlertSeverity::Info);
        assert_eq!(alert_known.category, AlertCategory::DelegationIssued);

        let alert_unknown = watcher.on_delegation_endorsed(
            identity, unknown_delegate, [0xa9, 0x05, 0x9c, 0xbb], 101, 1001,
        );
        assert_eq!(alert_unknown.severity, AlertSeverity::Warning);
        assert_eq!(alert_unknown.category, AlertCategory::UnknownDelegation);
    }

    #[test]
    fn test_watcher_session_invalidated() {
        let mut watcher = IdentityWatcher::new();
        let identity = [0xAA; 20];

        let alert = watcher.on_session_invalidated(identity, 5, 200, 2000);
        assert_eq!(alert.severity, AlertSeverity::Warning);
        assert_eq!(alert.category, AlertCategory::SessionsInvalidated);
        assert!(alert.message.contains("epoch: 5"));
    }

    #[test]
    fn test_watcher_offline_session_detected() {
        let mut watcher = IdentityWatcher::new();
        let identity = [0xAA; 20];
        let session = [0xEE; 20];

        let alert = watcher.on_offline_session_detected(identity, session, 300, 3000);
        assert_eq!(alert.severity, AlertSeverity::Critical);
        assert_eq!(alert.category, AlertCategory::OfflineSessionDetected);
    }

    #[test]
    fn test_watcher_alert_filtering() {
        let mut watcher = IdentityWatcher::new();
        let id1 = [0xAA; 20];
        let id2 = [0xBB; 20];

        watcher.on_intent_executed(id1, [0x11; 20], [0xa9, 0x05, 0x9c, 0xbb], 100, 1000);
        watcher.on_session_invalidated(id2, 1, 101, 1001);
        watcher.on_offline_session_detected(id1, [0x22; 20], 102, 1002);

        assert_eq!(watcher.alerts().len(), 3);
        assert_eq!(watcher.alerts_by_severity(AlertSeverity::Critical).len(), 1);
        assert_eq!(watcher.alerts_by_severity(AlertSeverity::Info).len(), 1);
        assert_eq!(watcher.alerts_for_identity(&id1).len(), 2);
        assert_eq!(watcher.alerts_for_identity(&id2).len(), 1);

        watcher.clear_alerts();
        assert_eq!(watcher.alerts().len(), 0);
    }

    #[test]
    fn test_watcher_identity_frozen() {
        let mut watcher = IdentityWatcher::new();
        let identity = [0xAA; 20];

        let alert = watcher.on_recovery_state_changed(
            identity, "Frozen", None, 100, 1000,
        );
        assert_eq!(alert.severity, AlertSeverity::Warning);
        assert_eq!(alert.category, AlertCategory::IdentityFrozen);
        assert!(alert.message.contains("FROZEN"));
    }

    #[test]
    fn test_watcher_high_value_intent() {
        let config = WatcherConfig {
            high_value_threshold: 1_000_000_000_000_000_000, // 1 ETH
        };
        let mut watcher = IdentityWatcher::with_config(config);
        let identity = [0xAA; 20];
        let session = [0xBB; 20];

        // Below threshold — no alert
        let result = watcher.on_high_value_intent(
            identity, session, 500_000_000_000_000_000, [0xa9, 0x05, 0x9c, 0xbb], 100, 1000,
        );
        assert!(result.is_none());

        // Above threshold — alert generated
        let result = watcher.on_high_value_intent(
            identity, session, 2_000_000_000_000_000_000, [0xa9, 0x05, 0x9c, 0xbb], 101, 1001,
        );
        assert!(result.is_some());
        let alert = result.unwrap();
        assert_eq!(alert.severity, AlertSeverity::Warning);
        assert_eq!(alert.category, AlertCategory::HighValueIntent);
        assert!(alert.message.contains("High-value"));
    }

    #[test]
    fn test_watcher_guardian_notifications() {
        let config = WatcherConfig {
            high_value_threshold: 1_000_000_000_000_000_000,
        };
        let mut watcher = IdentityWatcher::with_config(config);
        let identity = [0xAA; 20];
        let guardian1 = [0x11; 20];
        let guardian2 = [0x22; 20];

        watcher.register_guardians(identity, vec![guardian1, guardian2, [0x33; 20]]);

        // Trigger a high-value intent — should generate guardian notifications
        watcher.on_high_value_intent(
            identity, [0xBB; 20], 5_000_000_000_000_000_000,
            [0xa9, 0x05, 0x9c, 0xbb], 100, 1000,
        );

        let notifications = watcher.drain_notifications();
        assert_eq!(notifications.len(), 3, "all 3 guardians should be notified");
        assert_eq!(notifications[0].guardian, guardian1);
        assert_eq!(notifications[1].guardian, guardian2);
        assert_eq!(notifications[0].alert.category, AlertCategory::HighValueIntent);

        // Drain should clear
        assert_eq!(watcher.drain_notifications().len(), 0);
    }

    #[test]
    fn test_watcher_recovery_triggers_guardian_notifications() {
        let mut watcher = IdentityWatcher::new();
        let identity = [0xAA; 20];
        let guardian = [0xBB; 20];

        watcher.register_guardians(identity, vec![guardian, [0xCC; 20], [0xDD; 20]]);

        // Unknown guardian recovery → Critical → should notify all guardians
        watcher.on_recovery_state_changed(
            identity, "RecoveryPending", Some([0xFF; 20]), 100, 1000,
        );

        let notifications = watcher.drain_notifications();
        assert_eq!(notifications.len(), 3, "all guardians notified on critical recovery");
        assert_eq!(notifications[0].alert.severity, AlertSeverity::Critical);
    }
}

#[cfg(test)]
mod userop_tests {
    use super::*;

    #[test]
    fn test_user_operation_builder_basic() {
        let sender = [0xAA; 20];
        let call_data = vec![0xa9, 0x05, 0x9c, 0xbb, 0x01, 0x02];
        let sig_payload = vec![0xDE, 0xAD];

        let user_op = UserOperationBuilder::new(sender)
            .nonce(42)
            .call_data(call_data.clone())
            .gas(100_000, 200_000, 50_000, 30_000_000_000, 2_000_000_000)
            .build(sig_payload.clone());

        assert_eq!(user_op.sender, sender);
        assert_eq!(user_op.call_data, call_data);
        assert_eq!(user_op.signature, sig_payload);
        // Nonce packed at bytes 24..32
        assert_eq!(&user_op.nonce[24..], &42u64.to_be_bytes());
    }

    #[test]
    fn test_user_operation_builder_gas_packing() {
        let sender = [0xBB; 20];
        let call_gas: u128 = 100_000;
        let verif_gas: u128 = 200_000;
        let pre_gas: u128 = 50_000;
        let max_fee: u128 = 30_000_000_000;
        let priority_fee: u128 = 2_000_000_000;

        let user_op = UserOperationBuilder::new(sender)
            .gas(call_gas, verif_gas, pre_gas, max_fee, priority_fee)
            .build(vec![]);

        // account_gas_limits: callGasLimit (16 bytes) || verificationGasLimit (16 bytes)
        assert_eq!(&user_op.account_gas_limits[..16], &call_gas.to_be_bytes());
        assert_eq!(&user_op.account_gas_limits[16..], &verif_gas.to_be_bytes());

        // gas_fees: maxFeePerGas (16 bytes) || maxPriorityFeePerGas (16 bytes)
        assert_eq!(&user_op.gas_fees[..16], &max_fee.to_be_bytes());
        assert_eq!(&user_op.gas_fees[16..], &priority_fee.to_be_bytes());
    }

    #[test]
    fn test_user_operation_builder_default_fields() {
        let user_op = UserOperationBuilder::new([0xCC; 20]).build(vec![]);

        assert!(user_op.init_code.is_empty());
        assert!(user_op.paymaster_and_data.is_empty());
        assert!(user_op.call_data.is_empty());
        assert_eq!(user_op.nonce, [0u8; 32]);
    }

    #[test]
    fn test_user_operation_builder_with_init_code() {
        let init = vec![0x60, 0x00, 0x60, 0x00];
        let user_op = UserOperationBuilder::new([0xDD; 20])
            .init_code(init.clone())
            .build(vec![]);

        assert_eq!(user_op.init_code, init);
    }
}

#[cfg(test)]
mod event_log_tests {
    use super::*;

    #[test]
    fn test_event_log_records_and_queries() {
        let mut watcher = IdentityWatcher::new();
        let id = [0xAA; 20];
        let session = [0xBB; 20];
        let selector = [0xa9, 0x05, 0x9c, 0xbb];

        // Fire events that auto-record to the event log
        watcher.on_intent_executed(id, session, selector, 100, 1000);
        watcher.on_session_invalidated(id, 1, 101, 1001);
        watcher.on_recovery_state_changed(id, "RecoveryPending", Some([0xCC; 20]), 102, 1002);

        let log = watcher.event_log();
        assert_eq!(log.len(), 3);

        let intent_events = log.entries_by_type(EventType::IntentExecuted);
        assert_eq!(intent_events.len(), 1);
        assert_eq!(intent_events[0].block_number, 100);

        let session_events = log.entries_by_type(EventType::SessionInvalidated);
        assert_eq!(session_events.len(), 1);
        assert_eq!(session_events[0].metadata[0].1, "1");

        let recovery_events = log.entries_by_type(EventType::RecoveryStateChanged);
        assert_eq!(recovery_events.len(), 1);
    }

    #[test]
    fn test_event_log_export_json() {
        let mut log = EventLog::new();
        log.record_intent_executed([0xAA; 20], [0xBB; 20], [0xa9, 0x05, 0x9c, 0xbb], 100, 1000);

        let json = log.export_json();
        assert!(json.contains("\"event_type\": \"IntentExecuted\""));
        assert!(json.contains("\"block_number\": 100"));
        assert!(json.contains("\"session_key\""));
        assert!(json.contains("\"selector\""));
    }

    #[test]
    fn test_event_log_high_value_recorded() {
        let mut watcher = IdentityWatcher::with_config(WatcherConfig {
            high_value_threshold: 1_000_000,
        });
        watcher.register_guardians([0xAA; 20], vec![[0xDD; 20]]);

        let id = [0xAA; 20];
        let session = [0xBB; 20];
        let selector = [0xa9, 0x05, 0x9c, 0xbb];

        // Below threshold — no high-value log entry
        watcher.on_high_value_intent(id, session, 500_000, selector, 100, 1000);
        assert_eq!(watcher.event_log().entries_by_type(EventType::HighValueIntent).len(), 0);

        // Above threshold — recorded
        watcher.on_high_value_intent(id, session, 2_000_000, selector, 101, 1001);
        assert_eq!(watcher.event_log().entries_by_type(EventType::HighValueIntent).len(), 1);
    }

    #[test]
    fn test_event_log_identity_filter() {
        let mut log = EventLog::new();
        let id1 = [0xAA; 20];
        let id2 = [0xBB; 20];

        log.record_intent_executed(id1, [0x01; 20], [0x01; 4], 100, 1000);
        log.record_intent_executed(id2, [0x02; 20], [0x02; 4], 101, 1001);
        log.record_session_invalidated(id1, 1, 102, 1002);

        assert_eq!(log.entries_for_identity(&id1).len(), 2);
        assert_eq!(log.entries_for_identity(&id2).len(), 1);
    }
}
