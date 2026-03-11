// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @title PolicyGuard — EIP-712 intent validation with delegated ephemeral key authorization
contract PolicyGuard {
    // --- EIP-712 constants ---
    bytes32 public constant INTENT_TYPEHASH =
        keccak256(
            "SovereignIntent(address targetContract,bytes4 functionSig,uint128 maxValue,uint64 expiration,uint64 chainId,uint64 nonce)"
        );

    bytes32 public constant DELEGATION_TYPEHASH =
        keccak256(
            "DelegationCertificate(address delegate,bytes4 scope,uint128 maxValue,uint64 expiration,uint64 chainId,uint64 nonce)"
        );

    bytes32 public immutable DOMAIN_SEPARATOR;

    // --- Malleability guard ---
    uint256 private constant SECP256K1_N_DIV_2 =
        0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A0;

    // --- Structs for parameter packing ---
    struct DelegationParams {
        address delegate;
        bytes4 scope;
        uint128 maxValue;
        uint64 expiration;
        uint64 chainId;
        uint64 nonce;
        uint8 v;
        bytes32 r;
        bytes32 s;
    }

    struct IntentParams {
        address targetContract;
        bytes4 functionSig;
        uint128 maxValue;
        uint64 expiration;
        uint64 chainId;
        uint64 nonce;
        uint8 v;
        bytes32 r;
        bytes32 s;
    }

    // --- State ---
    address public owner;
    mapping(address => bool) public authorizedKeys;
    mapping(address => bool) public authorizedProvers;
    mapping(address => uint256) public nonces;
    mapping(address => uint256) public delegationNonces;

    // --- Events ---
    event KeyAuthorized(address indexed key);
    event KeyRevoked(address indexed key);
    event ProverRegistered(address indexed prover);
    event ProverRevoked(address indexed prover);
    event IntentValidated(
        address indexed signer,
        address indexed targetContract,
        uint128 maxValue,
        uint64 nonce
    );
    event DelegatedIntentValidated(
        address indexed prover,
        address indexed delegate,
        address indexed targetContract,
        uint128 maxValue
    );

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    bool private _gateOpen;

    modifier gatedFunction() {
        require(_gateOpen, "delegation required");
        _;
        _gateOpen = false;
    }

    constructor() {
        owner = msg.sender;
        DOMAIN_SEPARATOR = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256("HuntKey"),
                keccak256("1"),
                block.chainid,
                address(this)
            )
        );
    }

    // --- Key management ---

    function authorizeKey(address key) external onlyOwner {
        authorizedKeys[key] = true;
        emit KeyAuthorized(key);
    }

    function revokeKey(address key) external onlyOwner {
        authorizedKeys[key] = false;
        emit KeyRevoked(key);
    }

    // --- Prover (root identity) management ---

    function registerProver(address prover) external onlyOwner {
        authorizedProvers[prover] = true;
        emit ProverRegistered(prover);
    }

    function revokeProver(address prover) external onlyOwner {
        authorizedProvers[prover] = false;
        emit ProverRevoked(prover);
    }

    // --- Signature validation helpers ---

    function _validateSigParams(uint8 v, bytes32 s) internal pure {
        require(uint256(s) <= SECP256K1_N_DIV_2, "malleable signature: s too high");
        require(v == 27 || v == 28, "invalid v value");
    }

    function _recoverIntentSigner(IntentParams calldata p) internal view returns (address) {
        _validateSigParams(p.v, p.s);

        bytes32 structHash = keccak256(
            abi.encode(
                INTENT_TYPEHASH,
                p.targetContract,
                p.functionSig,
                p.maxValue,
                p.expiration,
                p.chainId,
                p.nonce
            )
        );
        bytes32 digest = keccak256(
            abi.encodePacked("\x19\x01", DOMAIN_SEPARATOR, structHash)
        );

        address signer = ecrecover(digest, p.v, p.r, p.s);
        require(signer != address(0), "ecrecover failed");
        return signer;
    }

    function _recoverDelegationProver(DelegationParams calldata d) internal view returns (address) {
        _validateSigParams(d.v, d.s);

        bytes32 structHash = keccak256(
            abi.encode(
                DELEGATION_TYPEHASH,
                d.delegate,
                d.scope,
                d.maxValue,
                d.expiration,
                d.chainId,
                d.nonce
            )
        );
        bytes32 digest = keccak256(
            abi.encodePacked("\x19\x01", DOMAIN_SEPARATOR, structHash)
        );

        address prover = ecrecover(digest, d.v, d.r, d.s);
        require(prover != address(0), "delegation ecrecover failed");
        return prover;
    }

    // --- Original direct-authorization validation ---

    function validateIntent(
        address targetContract,
        bytes4 functionSig,
        uint128 maxValue,
        uint64 expiration,
        uint64 intentChainId,
        uint64 nonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external payable {
        require(block.timestamp <= expiration, "intent expired");
        require(msg.value <= maxValue, "value exceeds cap");
        _validateSigParams(v, s);

        bytes32 structHash = keccak256(
            abi.encode(
                INTENT_TYPEHASH,
                targetContract,
                functionSig,
                maxValue,
                expiration,
                intentChainId,
                nonce
            )
        );
        bytes32 digest = keccak256(
            abi.encodePacked("\x19\x01", DOMAIN_SEPARATOR, structHash)
        );

        address signer = ecrecover(digest, v, r, s);
        require(signer != address(0), "ecrecover failed");
        require(authorizedKeys[signer], "unauthorized key");
        require(nonce == nonces[signer], "invalid nonce");
        nonces[signer]++;

        emit IntentValidated(signer, targetContract, maxValue, nonce);
    }

    // --- Delegated verification ---

    /// @notice Validate an intent carrying a delegation certificate from a registered prover.
    function validateDelegatedIntent(
        DelegationParams calldata delegation,
        IntentParams calldata intent
    ) external payable {
        // 1. Validate delegation
        require(block.timestamp <= delegation.expiration, "delegation expired");
        address prover = _recoverDelegationProver(delegation);
        require(authorizedProvers[prover], "unregistered prover");
        require(delegation.nonce == delegationNonces[prover], "invalid delegation nonce");
        delegationNonces[prover]++;

        // 2. Validate intent
        require(block.timestamp <= intent.expiration, "intent expired");
        require(msg.value <= intent.maxValue, "value exceeds cap");
        address intentSigner = _recoverIntentSigner(intent);

        // 3. Delegation chain: intent signer must be the delegate
        require(intentSigner == delegation.delegate, "signer is not delegate");

        // 4. Scope enforcement
        require(intent.functionSig == delegation.scope, "function outside delegation scope");

        // 5. Value cap
        require(intent.maxValue <= delegation.maxValue, "intent exceeds delegation cap");

        // 6. Intent nonce replay protection
        require(intent.nonce == nonces[delegation.delegate], "invalid nonce");
        nonces[delegation.delegate]++;

        // Open the gate
        _gateOpen = true;

        emit DelegatedIntentValidated(prover, delegation.delegate, intent.targetContract, intent.maxValue);
    }

    /// @notice Example gated function — only callable after a valid delegated intent.
    function gatedPurchase(address, uint128) external gatedFunction {
        // In production this would forward the call / transfer funds.
    }
}
