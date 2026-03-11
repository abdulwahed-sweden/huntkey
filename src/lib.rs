use coins_bip32::prelude::*;
use tiny_keccak::{Hasher, Keccak};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub use bip39::Mnemonic;
pub use coins_bip32;

/// Holds a derived key's data. Private key bytes are zeroed on drop.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct DerivedKey {
    #[zeroize(skip)]
    pub path: String,
    pub private_key: Vec<u8>,
    #[zeroize(skip)]
    pub public_key: Vec<u8>,
    #[zeroize(skip)]
    pub eth_address: Option<[u8; 20]>,
}

/// Derive an Ethereum address from an uncompressed public key (keccak256).
pub fn eth_address(pubkey: &k256::ecdsa::VerifyingKey) -> [u8; 20] {
    let uncompressed = pubkey.to_encoded_point(false);
    let mut hasher = Keccak::v256();
    hasher.update(&uncompressed.as_bytes()[1..]); // skip 0x04 prefix
    let mut hash = [0u8; 32];
    hasher.finalize(&mut hash);
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&hash[12..]); // last 20 bytes
    addr
}

/// Derive a key from an HD root at the given BIP-44 path.
pub fn derive_key(root: &XPriv, path: &str, is_eth: bool) -> DerivedKey {
    let derived = root.derive_path(path).expect("derivation failed");
    let signing_key: &k256::ecdsa::SigningKey = derived.as_ref();
    let verifying_key = signing_key.verifying_key();

    let eth_addr = if is_eth {
        Some(eth_address(verifying_key))
    } else {
        None
    };

    DerivedKey {
        path: path.to_string(),
        private_key: signing_key.to_bytes().to_vec(),
        public_key: verifying_key.to_sec1_bytes().to_vec(),
        eth_address: eth_addr,
    }
}

/// Generate a BIP-39 mnemonic with the given word count (12 or 24).
pub fn generate_mnemonic(word_count: usize) -> Mnemonic {
    Mnemonic::generate(word_count).expect("failed to generate mnemonic")
}

/// Create a BIP-32 root key from a mnemonic. The seed is zeroized after use.
pub fn root_from_mnemonic(mnemonic: &Mnemonic) -> XPriv {
    let seed = Zeroizing::new(mnemonic.to_seed(""));
    XPriv::root_from_seed(&*seed, None).expect("failed to create root key")
}

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

    fn test_root() -> XPriv {
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
