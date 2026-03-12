// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @title ClaimVerifier — ZK-SNARK claim verification for identity-bound credentials
/// @notice Verifies zero-knowledge proofs tied to claim types. In production,
///         the `verifyProof` function would delegate to a Groth16 or PLONK verifier.
///         This implementation uses a commitment-based model where claim proofs
///         are registered off-chain and verified on-chain via hash commitments.
contract ClaimVerifier {
    // --- Claim type constants ---
    bytes32 public constant AGE_OVER_18 = keccak256("AGE_OVER_18");
    bytes32 public constant KYC_VERIFIED = keccak256("KYC_VERIFIED");
    bytes32 public constant COUNTRY_ALLOWED = keccak256("COUNTRY_ALLOWED");
    bytes32 public constant DAO_MEMBER = keccak256("DAO_MEMBER");

    // --- Custom errors ---
    error InvalidProof();
    error ClaimNotRegistered();
    error ProofAlreadyUsed();
    error InvalidClaimType();
    error OnlyIssuer();

    // --- State ---
    /// @notice Issuer address authorized to register claim commitments.
    address public issuer;

    /// @notice Mapping: claimType => account => proof commitment hash.
    ///         A non-zero value means the account holds a valid claim of that type.
    mapping(bytes32 => mapping(address => bytes32)) public claimCommitments;

    /// @notice Tracks used proof hashes to prevent replay.
    mapping(bytes32 => bool) public usedProofs;

    // --- Events ---
    event ClaimRegistered(address indexed account, bytes32 indexed claimType, bytes32 commitment);
    event ClaimRevoked(address indexed account, bytes32 indexed claimType);
    event ClaimProofVerified(address indexed account, bytes32 indexed claimType, bytes32 proofHash);
    event IssuerUpdated(address indexed oldIssuer, address indexed newIssuer);

    modifier onlyIssuer() {
        if (msg.sender != issuer) revert OnlyIssuer();
        _;
    }

    constructor(address _issuer) {
        issuer = _issuer;
        emit IssuerUpdated(address(0), _issuer);
    }

    /// @notice Update the claim issuer. Only callable by current issuer.
    function setIssuer(address newIssuer) external onlyIssuer {
        address old = issuer;
        issuer = newIssuer;
        emit IssuerUpdated(old, newIssuer);
    }

    /// @notice Register a claim commitment for an account.
    ///         The commitment is keccak256(abi.encodePacked(account, claimType, secret))
    ///         where secret is known only to the claim holder.
    /// @param account The account the claim belongs to.
    /// @param claimType The type of claim (AGE_OVER_18, KYC_VERIFIED, etc).
    /// @param commitment The hash commitment for the claim proof.
    function registerClaim(
        address account,
        bytes32 claimType,
        bytes32 commitment
    ) external onlyIssuer {
        if (commitment == bytes32(0)) revert InvalidProof();
        claimCommitments[claimType][account] = commitment;
        emit ClaimRegistered(account, claimType, commitment);
    }

    /// @notice Revoke a claim for an account.
    function revokeClaim(address account, bytes32 claimType) external onlyIssuer {
        claimCommitments[claimType][account] = bytes32(0);
        emit ClaimRevoked(account, claimType);
    }

    /// @notice Verify a claim proof against a registered commitment.
    ///         The proof is valid if keccak256(abi.encodePacked(account, claimType, proof)) == commitment.
    /// @param account The account claiming the credential.
    /// @param claimType The type of claim to verify.
    /// @param proof The secret proof data (preimage).
    /// @return proofHash The hash of the verified proof (for intent binding).
    function verifyClaimProof(
        address account,
        bytes32 claimType,
        bytes32 proof
    ) external returns (bytes32 proofHash) {
        bytes32 commitment = claimCommitments[claimType][account];
        if (commitment == bytes32(0)) revert ClaimNotRegistered();

        proofHash = keccak256(abi.encodePacked(account, claimType, proof));
        if (proofHash != commitment) revert InvalidProof();
        if (usedProofs[proofHash]) revert ProofAlreadyUsed();

        usedProofs[proofHash] = true;
        emit ClaimProofVerified(account, claimType, proofHash);
    }

    /// @notice Check if an account has a registered claim of the given type.
    /// @param account The account to check.
    /// @param claimType The claim type to check.
    /// @return True if a commitment exists.
    function hasClaim(address account, bytes32 claimType) external view returns (bool) {
        return claimCommitments[claimType][account] != bytes32(0);
    }

    /// @notice Verify a proof hash without consuming it (view-only check).
    ///         Used by ExecutionGateway to verify intent.claimProofHash matches.
    /// @param account The account.
    /// @param claimType The claim type.
    /// @param proofHash The proof hash to verify.
    /// @return True if the proof hash matches the registered commitment.
    function verifyProofHash(
        address account,
        bytes32 claimType,
        bytes32 proofHash
    ) external view returns (bool) {
        bytes32 commitment = claimCommitments[claimType][account];
        if (commitment == bytes32(0)) return false;
        return proofHash == commitment;
    }
}
