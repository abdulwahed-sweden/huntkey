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

// ---------------------------------------------------------------------------
// Sovereign Identity Protocol — Key Hierarchy
// ---------------------------------------------------------------------------

/// Role-based key derivation under the m/999' purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRole {
    /// m/999'/0' — master identity, never touches the network
    RootIdentity,
    /// m/999'/1'/i — ephemeral action keys for constrained operations
    Action,
    /// m/999'/2'/i — proof generation keys
    Proof,
    /// m/999'/3'/i — social / multi-sig recovery keys
    Recovery,
}

impl KeyRole {
    /// Return the BIP-32 path segment for this role.
    pub fn segment(&self) -> &'static str {
        match self {
            KeyRole::RootIdentity => "0'",
            KeyRole::Action => "1'",
            KeyRole::Proof => "2'",
            KeyRole::Recovery => "3'",
        }
    }
}

/// Build the full derivation path for a role + index.
pub fn role_path(role: KeyRole, index: u32) -> String {
    match role {
        KeyRole::RootIdentity => format!("m/999'/{}", role.segment()),
        _ => format!("m/999'/{}/{}", role.segment(), index),
    }
}

/// Manages a sovereign key hierarchy rooted at m/999'.
pub struct KeyHierarchy {
    root: XPriv,
    action_index: u32,
}

impl KeyHierarchy {
    pub fn new(root: XPriv) -> Self {
        Self {
            root,
            action_index: 0,
        }
    }

    /// Derive a key for the given role and index.
    pub fn derive_role(&self, role: KeyRole, index: u32) -> DerivedKey {
        let path = role_path(role, index);
        derive_key(&self.root, &path, true)
    }

    /// Derive the next ephemeral action key, auto-incrementing the index.
    pub fn next_action_key(&mut self) -> DerivedKey {
        let key = self.derive_role(KeyRole::Action, self.action_index);
        self.action_index += 1;
        key
    }

    /// Current action index (next call to `next_action_key` will use this).
    pub fn action_index(&self) -> u32 {
        self.action_index
    }
}

// ---------------------------------------------------------------------------
// Sovereign Identity Protocol — EIP-712 Intent Signing
// ---------------------------------------------------------------------------

/// An intent describing a constrained on-chain action.
#[derive(Debug, Clone)]
pub struct SovereignIntent {
    pub target_contract: [u8; 20],
    pub function_sig: [u8; 4],
    pub max_value: u128,
    pub expiration: u64,
    pub chain_id: u64,
    pub nonce: u64,
}

/// ECDSA signature components.
#[derive(Debug, Clone)]
pub struct IntentSignature {
    pub v: u8,
    pub r: [u8; 32],
    pub s: [u8; 32],
}

fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(data);
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    out
}

/// Pad a byte slice to a 32-byte ABI word (left-padded with zeros).
fn left_pad_32(data: &[u8]) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[32 - data.len()..].copy_from_slice(data);
    word
}

/// Pad a byte slice to a 32-byte ABI word (right-padded with zeros).
/// Used for `bytes4` which Solidity ABI-encodes right-padded.
fn right_pad_32(data: &[u8]) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[..data.len()].copy_from_slice(data);
    word
}

/// Encode a u64 as a 32-byte big-endian ABI word.
fn u64_to_word(val: u64) -> [u8; 32] {
    left_pad_32(&val.to_be_bytes())
}

/// Encode a u128 as a 32-byte big-endian ABI word.
fn u128_to_word(val: u128) -> [u8; 32] {
    left_pad_32(&val.to_be_bytes())
}

/// Encode an address (20 bytes) as a 32-byte left-padded ABI word.
fn address_to_word(addr: &[u8; 20]) -> [u8; 32] {
    left_pad_32(addr)
}

const DOMAIN_TYPE_STR: &str =
    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";
const INTENT_TYPE_STR: &str =
    "SovereignIntent(address targetContract,bytes4 functionSig,uint128 maxValue,uint64 expiration,uint64 chainId,uint64 nonce)";

/// Compute the EIP-712 domain separator.
pub fn domain_separator(chain_id: u64, verifying_contract: &[u8; 20]) -> [u8; 32] {
    let domain_typehash = keccak256(DOMAIN_TYPE_STR.as_bytes());
    let name_hash = keccak256(b"HuntKey");
    let version_hash = keccak256(b"1");

    let mut buf = Vec::with_capacity(5 * 32);
    buf.extend_from_slice(&domain_typehash);
    buf.extend_from_slice(&name_hash);
    buf.extend_from_slice(&version_hash);
    buf.extend_from_slice(&u64_to_word(chain_id));
    buf.extend_from_slice(&address_to_word(verifying_contract));
    keccak256(&buf)
}

/// Compute the EIP-712 struct hash of a SovereignIntent.
pub fn intent_struct_hash(intent: &SovereignIntent) -> [u8; 32] {
    let typehash = keccak256(INTENT_TYPE_STR.as_bytes());

    let mut buf = Vec::with_capacity(7 * 32);
    buf.extend_from_slice(&typehash);
    buf.extend_from_slice(&address_to_word(&intent.target_contract));
    buf.extend_from_slice(&right_pad_32(&intent.function_sig)); // bytes4 right-padded
    buf.extend_from_slice(&u128_to_word(intent.max_value));
    buf.extend_from_slice(&u64_to_word(intent.expiration));
    buf.extend_from_slice(&u64_to_word(intent.chain_id));
    buf.extend_from_slice(&u64_to_word(intent.nonce));
    keccak256(&buf)
}

/// Compute the final EIP-712 signing hash for an intent.
pub fn intent_signing_hash(intent: &SovereignIntent, verifying_contract: &[u8; 20]) -> [u8; 32] {
    let ds = domain_separator(intent.chain_id, verifying_contract);
    let sh = intent_struct_hash(intent);

    let mut buf = Vec::with_capacity(2 + 32 + 32);
    buf.extend_from_slice(&[0x19, 0x01]);
    buf.extend_from_slice(&ds);
    buf.extend_from_slice(&sh);
    keccak256(&buf)
}

/// Sign a SovereignIntent with a private key, returning (v, r, s).
pub fn sign_intent(
    intent: &SovereignIntent,
    verifying_contract: &[u8; 20],
    private_key: &[u8; 32],
) -> IntentSignature {
    use k256::ecdsa::SigningKey;

    let hash = intent_signing_hash(intent, verifying_contract);

    let signing_key = SigningKey::from_bytes(private_key.into()).expect("invalid private key");
    let (sig, recid) = signing_key
        .sign_prehash_recoverable(&hash)
        .expect("signing failed");

    let bytes = sig.to_bytes();
    let mut r = [0u8; 32];
    let mut s = [0u8; 32];
    r.copy_from_slice(&bytes[..32]);
    s.copy_from_slice(&bytes[32..]);

    IntentSignature {
        v: recid.to_byte() + 27,
        r,
        s,
    }
}

/// Recover the signer address from an intent signature.
pub fn recover_signer(
    intent: &SovereignIntent,
    verifying_contract: &[u8; 20],
    sig: &IntentSignature,
) -> [u8; 20] {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

    let hash = intent_signing_hash(intent, verifying_contract);

    let mut sig_bytes = [0u8; 64];
    sig_bytes[..32].copy_from_slice(&sig.r);
    sig_bytes[32..].copy_from_slice(&sig.s);

    let signature = Signature::from_bytes((&sig_bytes).into()).expect("invalid signature");
    let recid = RecoveryId::from_byte(sig.v - 27).expect("invalid recovery id");

    let recovered_key =
        VerifyingKey::recover_from_prehash(&hash, &signature, recid).expect("recovery failed");

    eth_address(&recovered_key)
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

#[cfg(test)]
mod sovereign_tests {
    use super::*;
    use proptest::prelude::*;
    use std::str::FromStr;

    fn test_root() -> XPriv {
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
            function_sig: [0xa9, 0x05, 0x9c, 0xbb], // transfer(address,uint256)
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
    }
}
