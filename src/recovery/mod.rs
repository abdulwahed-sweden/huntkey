//! Social recovery: guardian threshold signing and pending recovery tracking.

use crate::core::{address_to_word, domain_separator, eth_address, keccak256, u64_to_word};
use crate::intents::IntentSignature;
use zeroize::{Zeroize, Zeroizing};

/// EIP-712 type string for RecoveryRequest.
pub(crate) const RECOVERY_TYPE_STR: &str =
    "RecoveryRequest(address oldRoot,address newRoot,uint64 chainId,uint64 nonce)";

/// A recovery request to migrate identity from one root to another.
/// Guardians sign this off-chain; the contract enforces threshold + timelock.
#[derive(Debug, Clone)]
pub struct RecoveryRequest {
    /// Current root identity address being recovered from.
    pub old_root: [u8; 20],
    /// New root identity address to migrate to.
    pub new_root: [u8; 20],
    /// Chain ID this recovery is bound to.
    pub chain_id: u64,
    /// Per-root nonce to prevent replay of old recovery requests.
    pub nonce: u64,
}

/// Tracks the local state of a pending recovery for alerting the user.
#[derive(Debug, Clone)]
pub struct PendingRecovery {
    /// The underlying recovery request.
    pub request: RecoveryRequest,
    /// Guardian addresses that have approved so far.
    pub approvals: Vec<[u8; 20]>,
    /// Timestamp when the threshold was met and timelock started (None if still gathering).
    pub initiated_at: Option<u64>,
}

impl PendingRecovery {
    /// Create a new pending recovery tracker from a recovery request.
    pub fn new(request: RecoveryRequest) -> Self {
        Self {
            request,
            approvals: Vec::new(),
            initiated_at: None,
        }
    }

    /// Record a guardian approval. Returns true if this is a new approval.
    pub fn add_approval(&mut self, guardian: [u8; 20]) -> bool {
        if self.approvals.contains(&guardian) {
            return false;
        }
        self.approvals.push(guardian);
        true
    }

    /// Check if the threshold has been met.
    pub fn threshold_met(&self, threshold: usize) -> bool {
        self.approvals.len() >= threshold
    }

    /// Check if this recovery is an alert (someone initiated recovery on our root).
    pub fn is_alert(&self, our_root: &[u8; 20]) -> bool {
        self.request.old_root == *our_root
    }

    /// Check if the timelock has expired (48 hours = 172800 seconds).
    pub fn timelock_expired(&self, current_time: u64) -> bool {
        match self.initiated_at {
            Some(t) => current_time >= t + 172800,
            None => false,
        }
    }
}

/// Compute the EIP-712 struct hash of a RecoveryRequest.
pub fn recovery_struct_hash(req: &RecoveryRequest) -> [u8; 32] {
    let typehash = keccak256(RECOVERY_TYPE_STR.as_bytes());

    let mut buf = Vec::with_capacity(5 * 32);
    buf.extend_from_slice(&typehash);
    buf.extend_from_slice(&address_to_word(&req.old_root));
    buf.extend_from_slice(&address_to_word(&req.new_root));
    buf.extend_from_slice(&u64_to_word(req.chain_id));
    buf.extend_from_slice(&u64_to_word(req.nonce));
    keccak256(&buf)
}

/// Compute the EIP-712 signing hash for a recovery request.
pub fn recovery_signing_hash(
    req: &RecoveryRequest,
    verifying_contract: &[u8; 20],
) -> [u8; 32] {
    let ds = domain_separator(req.chain_id, verifying_contract);
    let sh = recovery_struct_hash(req);

    let mut buf = Vec::with_capacity(2 + 32 + 32);
    buf.extend_from_slice(&[0x19, 0x01]);
    buf.extend_from_slice(&ds);
    buf.extend_from_slice(&sh);
    keccak256(&buf)
}

/// Sign a RecoveryRequest with a guardian's private key.
/// The guardian_private_key bytes are zeroized after use.
pub fn sign_recovery_request(
    req: &RecoveryRequest,
    verifying_contract: &[u8; 20],
    guardian_private_key: &[u8; 32],
) -> IntentSignature {
    use k256::ecdsa::SigningKey;

    let hash = recovery_signing_hash(req, verifying_contract);

    let mut key_bytes = Zeroizing::new(*guardian_private_key);
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

    IntentSignature {
        v: recid.to_byte() + 27,
        r,
        s,
    }
}

/// Recover the guardian address from a signed recovery request.
pub fn recover_recovery_signer(
    req: &RecoveryRequest,
    verifying_contract: &[u8; 20],
    sig: &IntentSignature,
) -> [u8; 20] {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

    let hash = recovery_signing_hash(req, verifying_contract);

    let mut sig_bytes = [0u8; 64];
    sig_bytes[..32].copy_from_slice(&sig.r);
    sig_bytes[32..].copy_from_slice(&sig.s);

    let signature = Signature::from_bytes((&sig_bytes).into()).expect("invalid signature");
    let recid = RecoveryId::from_byte(sig.v - 27).expect("invalid recovery id");

    let recovered =
        VerifyingKey::recover_from_prehash(&hash, &signature, recid).expect("recovery failed");

    eth_address(&recovered)
}
