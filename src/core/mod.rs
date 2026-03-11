//! Core primitives: key derivation, hashing, and ABI encoding utilities.

use coins_bip32::prelude::*;
use tiny_keccak::{Hasher, Keccak};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub use bip39::Mnemonic;
pub use coins_bip32;

/// Holds a derived key's data. Private key bytes are zeroed on drop.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct DerivedKey {
    /// BIP-32 derivation path used to produce this key.
    #[zeroize(skip)]
    pub path: String,
    /// Raw private key bytes (32 bytes for secp256k1).
    pub private_key: Vec<u8>,
    /// Compressed public key bytes (33 bytes).
    #[zeroize(skip)]
    pub public_key: Vec<u8>,
    /// Ethereum address (last 20 bytes of keccak256 of uncompressed pubkey).
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
    /// m/999'/0' — master identity, never touches the network.
    RootIdentity,
    /// m/999'/1'/i — ephemeral action keys for constrained operations.
    Action,
    /// m/999'/2'/i — proof generation keys.
    Proof,
    /// m/999'/3'/i — social / multi-sig recovery keys.
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
    /// Create a new key hierarchy from a BIP-32 root key.
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
// Internal hashing & ABI encoding helpers
// ---------------------------------------------------------------------------

/// Compute keccak256 of arbitrary data.
pub(crate) fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(data);
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    out
}

/// Pad a byte slice to a 32-byte ABI word (left-padded with zeros).
pub(crate) fn left_pad_32(data: &[u8]) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[32 - data.len()..].copy_from_slice(data);
    word
}

/// Pad a byte slice to a 32-byte ABI word (right-padded with zeros).
/// Used for `bytes4` which Solidity ABI-encodes right-padded.
pub(crate) fn right_pad_32(data: &[u8]) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[..data.len()].copy_from_slice(data);
    word
}

/// Encode a u64 as a 32-byte big-endian ABI word.
pub(crate) fn u64_to_word(val: u64) -> [u8; 32] {
    left_pad_32(&val.to_be_bytes())
}

/// Encode a u128 as a 32-byte big-endian ABI word.
pub(crate) fn u128_to_word(val: u128) -> [u8; 32] {
    left_pad_32(&val.to_be_bytes())
}

/// Encode an address (20 bytes) as a 32-byte left-padded ABI word.
pub(crate) fn address_to_word(addr: &[u8; 20]) -> [u8; 32] {
    left_pad_32(addr)
}

/// EIP-712 domain type string.
pub(crate) const DOMAIN_TYPE_STR: &str =
    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";

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

/// Compute the keccak256 hash of call data for intent binding.
pub fn call_data_hash(call_data: &[u8]) -> [u8; 32] {
    keccak256(call_data)
}
