//! EIP-712 intent signing and verification.

use crate::core::{
    address_to_word, domain_separator, eth_address, keccak256, right_pad_32, u128_to_word,
    u64_to_word,
};
use zeroize::{Zeroize, Zeroizing};

/// EIP-712 type string for SovereignIntent (v2.0 with gasLimit, maxFeePerGas, requiredClaim).
pub(crate) const INTENT_TYPE_STR: &str =
    "SovereignIntent(address targetContract,bytes4 functionSig,address recipient,address assetAddress,bytes32 callDataHash,uint128 maxValue,uint64 expiration,uint64 chainId,uint64 nonce,uint64 gasLimit,uint128 maxFeePerGas,bytes32 requiredClaim)";

/// An intent describing a constrained on-chain action.
#[derive(Debug, Clone)]
pub struct SovereignIntent {
    /// Target contract address for the call.
    pub target_contract: [u8; 20],
    /// 4-byte function selector.
    pub function_sig: [u8; 4],
    /// Recipient address for the operation.
    pub recipient: [u8; 20],
    /// Asset contract address (zero for native ETH).
    pub asset_address: [u8; 20],
    /// keccak256 hash of the exact calldata to be submitted.
    pub call_data_hash: [u8; 32],
    /// Maximum wei allowed for this intent.
    pub max_value: u128,
    /// Unix timestamp after which the intent is void.
    pub expiration: u64,
    /// Chain ID this intent is bound to.
    pub chain_id: u64,
    /// Per-signer nonce for replay protection.
    pub nonce: u64,
    /// Gas limit for ERC-4337 UserOperation validation.
    pub gas_limit: u64,
    /// Maximum fee per gas unit (wei) for ERC-4337 operations.
    pub max_fee_per_gas: u128,
    /// Required credential claim (bytes32). Zero means no claim required.
    pub required_claim: [u8; 32],
}

/// ECDSA signature components.
#[derive(Debug, Clone)]
pub struct IntentSignature {
    /// Recovery id (27 or 28).
    pub v: u8,
    /// r component of the signature.
    pub r: [u8; 32],
    /// s component of the signature.
    pub s: [u8; 32],
}

/// Compute the EIP-712 struct hash of a SovereignIntent.
pub fn intent_struct_hash(intent: &SovereignIntent) -> [u8; 32] {
    let typehash = keccak256(INTENT_TYPE_STR.as_bytes());

    let mut buf = Vec::with_capacity(13 * 32);
    buf.extend_from_slice(&typehash);
    buf.extend_from_slice(&address_to_word(&intent.target_contract));
    buf.extend_from_slice(&right_pad_32(&intent.function_sig)); // bytes4 right-padded
    buf.extend_from_slice(&address_to_word(&intent.recipient));
    buf.extend_from_slice(&address_to_word(&intent.asset_address));
    buf.extend_from_slice(&intent.call_data_hash); // bytes32 is already 32 bytes
    buf.extend_from_slice(&u128_to_word(intent.max_value));
    buf.extend_from_slice(&u64_to_word(intent.expiration));
    buf.extend_from_slice(&u64_to_word(intent.chain_id));
    buf.extend_from_slice(&u64_to_word(intent.nonce));
    buf.extend_from_slice(&u64_to_word(intent.gas_limit));
    buf.extend_from_slice(&u128_to_word(intent.max_fee_per_gas));
    buf.extend_from_slice(&intent.required_claim); // bytes32 is already 32 bytes
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

// ---------------------------------------------------------------------------
// Delegation Certificates
// ---------------------------------------------------------------------------

/// EIP-712 type string for DelegationCertificate.
pub(crate) const DELEGATION_TYPE_STR: &str =
    "DelegationCertificate(address delegate,bytes4 scope,uint128 maxValue,uint64 expiration,uint64 chainId,uint64 nonce)";

/// A delegation certificate that links an ActionKey back to a RootIdentity.
/// The Root signs this off-chain; the contract verifies the chain on-chain.
#[derive(Debug, Clone)]
pub struct DelegationCertificate {
    /// Ethereum address of the delegated action key.
    pub delegate: [u8; 20],
    /// Function selector the delegate is authorized to invoke.
    pub scope: [u8; 4],
    /// Maximum wei the delegate may spend per intent.
    pub max_value: u128,
    /// Unix timestamp after which the delegation is void.
    pub expiration: u64,
    /// Chain this delegation is bound to (prevents cross-chain replay).
    pub chain_id: u64,
    /// Per-prover nonce consumed on-chain to prevent replay.
    pub nonce: u64,
}

/// Signed delegation — carries the certificate plus its ECDSA components.
#[derive(Debug, Clone)]
pub struct SignedDelegation {
    /// The delegation certificate that was signed.
    pub certificate: DelegationCertificate,
    /// Recovery id (27 or 28).
    pub v: u8,
    /// r component of the signature.
    pub r: [u8; 32],
    /// s component of the signature.
    pub s: [u8; 32],
}

/// Compute the EIP-712 struct hash of a DelegationCertificate.
pub fn delegation_struct_hash(cert: &DelegationCertificate) -> [u8; 32] {
    let typehash = keccak256(DELEGATION_TYPE_STR.as_bytes());

    let mut buf = Vec::with_capacity(7 * 32);
    buf.extend_from_slice(&typehash);
    buf.extend_from_slice(&address_to_word(&cert.delegate));
    buf.extend_from_slice(&right_pad_32(&cert.scope)); // bytes4 right-padded
    buf.extend_from_slice(&u128_to_word(cert.max_value));
    buf.extend_from_slice(&u64_to_word(cert.expiration));
    buf.extend_from_slice(&u64_to_word(cert.chain_id));
    buf.extend_from_slice(&u64_to_word(cert.nonce));
    keccak256(&buf)
}

/// Compute the EIP-712 signing hash for a delegation certificate.
pub fn delegation_signing_hash(
    cert: &DelegationCertificate,
    verifying_contract: &[u8; 20],
) -> [u8; 32] {
    let ds = domain_separator(cert.chain_id, verifying_contract);
    let sh = delegation_struct_hash(cert);

    let mut buf = Vec::with_capacity(2 + 32 + 32);
    buf.extend_from_slice(&[0x19, 0x01]);
    buf.extend_from_slice(&ds);
    buf.extend_from_slice(&sh);
    keccak256(&buf)
}

/// Sign a DelegationCertificate with the RootIdentity private key.
/// The root_private_key bytes are wrapped in Zeroizing to ensure cleanup.
pub fn sign_delegation(
    cert: &DelegationCertificate,
    verifying_contract: &[u8; 20],
    root_private_key: &[u8; 32],
) -> SignedDelegation {
    use k256::ecdsa::SigningKey;

    let hash = delegation_signing_hash(cert, verifying_contract);

    // Wrap the signing key so it zeroizes on drop
    let mut key_bytes = Zeroizing::new(*root_private_key);
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

    SignedDelegation {
        certificate: cert.clone(),
        v: recid.to_byte() + 27,
        r,
        s,
    }
}

/// Recover the prover (root identity) address from a signed delegation.
pub fn recover_delegator(
    cert: &DelegationCertificate,
    verifying_contract: &[u8; 20],
    v: u8,
    r: &[u8; 32],
    s: &[u8; 32],
) -> [u8; 20] {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

    let hash = delegation_signing_hash(cert, verifying_contract);

    let mut sig_bytes = [0u8; 64];
    sig_bytes[..32].copy_from_slice(r);
    sig_bytes[32..].copy_from_slice(s);

    let signature = Signature::from_bytes((&sig_bytes).into()).expect("invalid signature");
    let recid = RecoveryId::from_byte(v - 27).expect("invalid recovery id");

    let recovered =
        VerifyingKey::recover_from_prehash(&hash, &signature, recid).expect("recovery failed");

    eth_address(&recovered)
}
