//! Ephemeral session keys: HKDF derivation and session certificate signing.

use crate::core::{
    address_to_word, domain_separator, eth_address, keccak256, right_pad_32, u128_to_word,
    u64_to_word,
};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// EIP-712 type string for SessionCertificate.
pub(crate) const SESSION_TYPE_STR: &str =
    "SessionCertificate(address session,address parent,bytes4 scope,address target,uint128 maxValue,uint64 expiration,uint64 chainId)";

/// An ephemeral session key derived from an action key + nonce.
/// All secret material is zeroed on drop.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SessionKey {
    /// Raw private key bytes (32 bytes). Zeroed on drop.
    pub private_key: [u8; 32],
    /// Compressed public key bytes.
    #[zeroize(skip)]
    pub public_key: Vec<u8>,
    /// Ethereum address of the session key.
    #[zeroize(skip)]
    pub eth_address: [u8; 20],
}

/// A session certificate that links an ephemeral session key to its parent action key.
#[derive(Debug, Clone)]
pub struct SessionCertificate {
    /// Ethereum address of the ephemeral session key.
    pub session: [u8; 20],
    /// Ethereum address of the parent action key.
    pub parent: [u8; 20],
    /// Function selector the session is scoped to.
    pub scope: [u8; 4],
    /// Target contract address the session is scoped to.
    pub target: [u8; 20],
    /// Maximum wei this session may spend per intent.
    pub max_value: u128,
    /// Unix timestamp after which the session is void.
    pub expiration: u64,
    /// Chain ID this session is bound to.
    pub chain_id: u64,
}

/// Signed session certificate.
#[derive(Debug, Clone)]
pub struct SignedSession {
    /// The session certificate that was signed.
    pub certificate: SessionCertificate,
    /// Recovery id (27 or 28).
    pub v: u8,
    /// r component of the signature.
    pub r: [u8; 32],
    /// s component of the signature.
    pub s: [u8; 32],
}

/// Derive an ephemeral one-time-use session key using HKDF-SHA256.
///
/// IKM: action_privkey (32 bytes)
/// Salt: domain string "HuntKey-V1-Session-Key"
/// Info: parent_pubkey (33 bytes compressed) || nonce (8 bytes big-endian)
///
/// This ensures global uniqueness: different action keys, different nonces,
/// or different parent public keys all produce distinct session keys.
/// All secret material is zeroized after use.
pub fn derive_session_key(action_privkey: &[u8; 32], nonce: u64) -> SessionKey {
    use hkdf::Hkdf;
    use k256::ecdsa::SigningKey;
    use sha2::Sha256;

    // Derive parent pubkey for info binding
    let parent_key = SigningKey::from_bytes(action_privkey.into()).expect("invalid action key");
    let parent_pubkey = parent_key.verifying_key().to_sec1_bytes();

    // Build info: parent_pubkey || nonce
    let mut info = Vec::with_capacity(parent_pubkey.len() + 8);
    info.extend_from_slice(&parent_pubkey);
    info.extend_from_slice(&nonce.to_be_bytes());

    // HKDF extract + expand
    let salt = b"HuntKey-V1-Session-Key";
    let hk = Hkdf::<Sha256>::new(Some(salt), action_privkey);
    let mut okm = Zeroizing::new([0u8; 32]);
    hk.expand(&info, &mut *okm).expect("HKDF expand failed");

    // Zero the info buffer
    for b in info.iter_mut() {
        *b = 0;
    }

    let signing_key =
        SigningKey::from_bytes((&*okm).into()).expect("invalid session key derivation");
    okm.zeroize();

    let verifying_key = signing_key.verifying_key();
    let addr = eth_address(verifying_key);

    SessionKey {
        private_key: signing_key.to_bytes().into(),
        public_key: verifying_key.to_sec1_bytes().to_vec(),
        eth_address: addr,
    }
}

/// Compute the EIP-712 struct hash of a SessionCertificate.
pub fn session_struct_hash(cert: &SessionCertificate) -> [u8; 32] {
    let typehash = keccak256(SESSION_TYPE_STR.as_bytes());

    let mut buf = Vec::with_capacity(8 * 32);
    buf.extend_from_slice(&typehash);
    buf.extend_from_slice(&address_to_word(&cert.session));
    buf.extend_from_slice(&address_to_word(&cert.parent));
    buf.extend_from_slice(&right_pad_32(&cert.scope)); // bytes4 right-padded
    buf.extend_from_slice(&address_to_word(&cert.target));
    buf.extend_from_slice(&u128_to_word(cert.max_value));
    buf.extend_from_slice(&u64_to_word(cert.expiration));
    buf.extend_from_slice(&u64_to_word(cert.chain_id));
    keccak256(&buf)
}

/// Compute the EIP-712 signing hash for a session certificate.
pub fn session_signing_hash(
    cert: &SessionCertificate,
    verifying_contract: &[u8; 20],
) -> [u8; 32] {
    let ds = domain_separator(cert.chain_id, verifying_contract);
    let sh = session_struct_hash(cert);

    let mut buf = Vec::with_capacity(2 + 32 + 32);
    buf.extend_from_slice(&[0x19, 0x01]);
    buf.extend_from_slice(&ds);
    buf.extend_from_slice(&sh);
    keccak256(&buf)
}

/// Sign a SessionCertificate with the parent action key's private key.
/// The action_privkey is zeroized after use.
pub fn sign_session_cert(
    cert: &SessionCertificate,
    verifying_contract: &[u8; 20],
    action_privkey: &[u8; 32],
) -> SignedSession {
    use k256::ecdsa::SigningKey;

    let hash = session_signing_hash(cert, verifying_contract);

    let mut key_bytes = Zeroizing::new(*action_privkey);
    let signing_key = SigningKey::from_bytes((&*key_bytes).into()).expect("invalid private key");
    key_bytes.zeroize();

    let (sig, recid) = signing_key
        .sign_prehash_recoverable(&hash)
        .expect("signing failed");

    let bytes = sig.to_bytes();
    let mut r = [0u8; 32];
    let mut s = [0u8; 32];
    r.copy_from_slice(&bytes[..32]);
    s.copy_from_slice(&bytes[32..]);

    SignedSession {
        certificate: cert.clone(),
        v: recid.to_byte() + 27,
        r,
        s,
    }
}

/// Recover the signer (parent action key) of a signed session certificate.
pub fn recover_session_signer(
    cert: &SessionCertificate,
    verifying_contract: &[u8; 20],
    v: u8,
    r: &[u8; 32],
    s: &[u8; 32],
) -> [u8; 20] {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

    let hash = session_signing_hash(cert, verifying_contract);

    let mut sig_bytes = [0u8; 64];
    sig_bytes[..32].copy_from_slice(r);
    sig_bytes[32..].copy_from_slice(s);

    let signature = Signature::from_bytes((&sig_bytes).into()).expect("invalid signature");
    let recid = RecoveryId::from_byte(v - 27).expect("invalid recovery id");

    let recovered =
        VerifyingKey::recover_from_prehash(&hash, &signature, recid).expect("recovery failed");

    eth_address(&recovered)
}
