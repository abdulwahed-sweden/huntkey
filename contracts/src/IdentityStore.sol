// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @title IdentityStore — Identity state, delegation verification, and social recovery
abstract contract IdentityStore {
    // --- EIP-712 constants ---
    bytes32 public constant INTENT_TYPEHASH =
        keccak256(
            "SovereignIntent(address targetContract,bytes4 functionSig,uint128 maxValue,uint64 expiration,uint64 chainId,uint64 nonce)"
        );

    bytes32 public constant DELEGATION_TYPEHASH =
        keccak256(
            "DelegationCertificate(address delegate,bytes4 scope,uint128 maxValue,uint64 expiration,uint64 chainId,uint64 nonce)"
        );

    bytes32 public constant RECOVERY_TYPEHASH =
        keccak256(
            "RecoveryRequest(address oldRoot,address newRoot,uint64 chainId,uint64 nonce)"
        );

    bytes32 public immutable DOMAIN_SEPARATOR;

    // --- Malleability guard ---
    uint256 internal constant SECP256K1_N_DIV_2 =
        0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A0;

    // --- Recovery constants ---
    uint256 public constant RECOVERY_WINDOW = 48 hours;
    uint256 public constant GUARDIAN_THRESHOLD = 2;

    // --- Structs ---
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

    // --- Identity state ---
    address public owner;
    mapping(address => bool) public authorizedKeys;
    mapping(address => bool) public authorizedProvers;
    mapping(address => uint256) public nonces;
    mapping(address => uint256) public delegationNonces;

    // --- Recovery state ---
    mapping(address => address[]) internal _guardians;
    mapping(address => address) public pendingNewRoot;
    mapping(address => uint256) public recoveryApprovals;
    mapping(address => uint256) public recoveryInitiatedAt;
    mapping(address => mapping(address => bool)) public recoveryApproved;
    mapping(address => uint256) public recoveryNonces;

    // --- Events ---
    event KeyAuthorized(address indexed key);
    event KeyRevoked(address indexed key);
    event ProverRegistered(address indexed prover);
    event ProverRevoked(address indexed prover);
    event IntentValidated(
        address indexed signer, address indexed targetContract, uint128 maxValue, uint64 nonce
    );
    event DelegatedIntentValidated(
        address indexed prover, address indexed delegate, address indexed targetContract, uint128 maxValue
    );
    event GuardiansSet(address indexed root, uint256 count);
    event RecoveryInitiated(address indexed oldRoot, address indexed newRoot, address indexed guardian);
    event RecoverySupported(address indexed oldRoot, address indexed guardian, uint256 approvals);
    event RecoveryCancelled(address indexed oldRoot);
    event RecoveryFinalized(address indexed oldRoot, address indexed newRoot);

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
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

    // --- Prover management ---

    function registerProver(address prover) external onlyOwner {
        authorizedProvers[prover] = true;
        emit ProverRegistered(prover);
    }

    function revokeProver(address prover) external onlyOwner {
        authorizedProvers[prover] = false;
        emit ProverRevoked(prover);
    }

    // --- Guardian management ---

    function setGuardians(address root, address[] calldata guardianList) external onlyOwner {
        require(guardianList.length >= 3 && guardianList.length <= 5, "need 3-5 guardians");
        _guardians[root] = guardianList;
        emit GuardiansSet(root, guardianList.length);
    }

    function getGuardians(address root) external view returns (address[] memory) {
        return _guardians[root];
    }

    function isGuardian(address root, address candidate) public view returns (bool) {
        address[] storage gs = _guardians[root];
        for (uint256 i = 0; i < gs.length; i++) {
            if (gs[i] == candidate) return true;
        }
        return false;
    }

    // --- Signature helpers ---

    function _validateSigParams(uint8 v, bytes32 s) internal pure {
        require(uint256(s) <= SECP256K1_N_DIV_2, "malleable signature: s too high");
        require(v == 27 || v == 28, "invalid v value");
    }

    function _recoverIntentSigner(IntentParams calldata p) internal view returns (address) {
        _validateSigParams(p.v, p.s);
        bytes32 structHash = keccak256(
            abi.encode(INTENT_TYPEHASH, p.targetContract, p.functionSig, p.maxValue, p.expiration, p.chainId, p.nonce)
        );
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", DOMAIN_SEPARATOR, structHash));
        address signer = ecrecover(digest, p.v, p.r, p.s);
        require(signer != address(0), "ecrecover failed");
        return signer;
    }

    function _recoverDelegationProver(DelegationParams calldata d) internal view returns (address) {
        _validateSigParams(d.v, d.s);
        bytes32 structHash = keccak256(
            abi.encode(DELEGATION_TYPEHASH, d.delegate, d.scope, d.maxValue, d.expiration, d.chainId, d.nonce)
        );
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", DOMAIN_SEPARATOR, structHash));
        address prover = ecrecover(digest, d.v, d.r, d.s);
        require(prover != address(0), "delegation ecrecover failed");
        return prover;
    }

    function _recoverRecoverySigner(
        address oldRoot, address newRoot, uint64 chainId, uint64 nonce, uint8 v, bytes32 r, bytes32 s
    ) internal view returns (address) {
        _validateSigParams(v, s);
        bytes32 structHash = keccak256(abi.encode(RECOVERY_TYPEHASH, oldRoot, newRoot, chainId, nonce));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", DOMAIN_SEPARATOR, structHash));
        address signer = ecrecover(digest, v, r, s);
        require(signer != address(0), "recovery ecrecover failed");
        return signer;
    }

    // --- Direct-authorization intent validation ---

    function validateIntent(
        address targetContract, bytes4 functionSig, uint128 maxValue,
        uint64 expiration, uint64 intentChainId, uint64 nonce,
        uint8 v, bytes32 r, bytes32 s
    ) external payable {
        require(block.timestamp <= expiration, "intent expired");
        require(msg.value <= maxValue, "value exceeds cap");
        _validateSigParams(v, s);

        bytes32 structHash = keccak256(
            abi.encode(INTENT_TYPEHASH, targetContract, functionSig, maxValue, expiration, intentChainId, nonce)
        );
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", DOMAIN_SEPARATOR, structHash));

        address signer = ecrecover(digest, v, r, s);
        require(signer != address(0), "ecrecover failed");
        require(authorizedKeys[signer], "unauthorized key");
        require(nonce == nonces[signer], "invalid nonce");
        nonces[signer]++;

        emit IntentValidated(signer, targetContract, maxValue, nonce);
    }

    // --- Delegated verification ---

    bool internal _gateOpen;

    modifier gatedFunction() {
        require(_gateOpen, "delegation required");
        _;
        _gateOpen = false;
    }

    function validateDelegatedIntent(
        DelegationParams calldata delegation, IntentParams calldata intent
    ) external payable {
        require(block.timestamp <= delegation.expiration, "delegation expired");
        address prover = _recoverDelegationProver(delegation);
        require(authorizedProvers[prover], "unregistered prover");
        require(delegation.nonce == delegationNonces[prover], "invalid delegation nonce");
        delegationNonces[prover]++;

        require(block.timestamp <= intent.expiration, "intent expired");
        require(msg.value <= intent.maxValue, "value exceeds cap");
        address intentSigner = _recoverIntentSigner(intent);

        require(intentSigner == delegation.delegate, "signer is not delegate");
        require(intent.functionSig == delegation.scope, "function outside delegation scope");
        require(intent.maxValue <= delegation.maxValue, "intent exceeds delegation cap");

        require(intent.nonce == nonces[delegation.delegate], "invalid nonce");
        nonces[delegation.delegate]++;

        _gateOpen = true;

        emit DelegatedIntentValidated(prover, delegation.delegate, intent.targetContract, intent.maxValue);
    }

    function gatedPurchase(address, uint128) external gatedFunction {}

    // --- Social Recovery ---

    function initiateRecovery(address oldRoot, address newRoot, uint8 v, bytes32 r, bytes32 s) external {
        require(authorizedProvers[oldRoot], "old root not registered");
        require(newRoot != address(0), "invalid new root");
        require(pendingNewRoot[oldRoot] == address(0), "recovery already pending");

        address guardian = _recoverRecoverySigner(oldRoot, newRoot, uint64(block.chainid), uint64(recoveryNonces[oldRoot]), v, r, s);
        require(isGuardian(oldRoot, guardian), "not a guardian");

        pendingNewRoot[oldRoot] = newRoot;
        recoveryApprovals[oldRoot] = 1;
        recoveryApproved[oldRoot][guardian] = true;

        emit RecoveryInitiated(oldRoot, newRoot, guardian);
    }

    function supportRecovery(address oldRoot, uint8 v, bytes32 r, bytes32 s) external {
        address newRoot = pendingNewRoot[oldRoot];
        require(newRoot != address(0), "no pending recovery");

        address guardian = _recoverRecoverySigner(oldRoot, newRoot, uint64(block.chainid), uint64(recoveryNonces[oldRoot]), v, r, s);
        require(isGuardian(oldRoot, guardian), "not a guardian");
        require(!recoveryApproved[oldRoot][guardian], "already approved");

        recoveryApproved[oldRoot][guardian] = true;
        recoveryApprovals[oldRoot]++;

        if (recoveryApprovals[oldRoot] >= GUARDIAN_THRESHOLD && recoveryInitiatedAt[oldRoot] == 0) {
            recoveryInitiatedAt[oldRoot] = block.timestamp;
        }

        emit RecoverySupported(oldRoot, guardian, recoveryApprovals[oldRoot]);
    }

    function cancelRecovery(address oldRoot) external {
        require(msg.sender == oldRoot, "only old root can cancel");
        require(pendingNewRoot[oldRoot] != address(0), "no pending recovery");
        _clearRecovery(oldRoot);
        emit RecoveryCancelled(oldRoot);
    }

    function finalizeRecovery(address oldRoot) external {
        address newRoot = pendingNewRoot[oldRoot];
        require(newRoot != address(0), "no pending recovery");
        require(recoveryApprovals[oldRoot] >= GUARDIAN_THRESHOLD, "threshold not met");
        require(recoveryInitiatedAt[oldRoot] > 0, "timelock not started");
        require(block.timestamp >= recoveryInitiatedAt[oldRoot] + RECOVERY_WINDOW, "timelock not expired");

        authorizedProvers[oldRoot] = false;
        authorizedProvers[newRoot] = true;
        nonces[newRoot] = 0;
        delegationNonces[newRoot] = 0;
        _guardians[newRoot] = _guardians[oldRoot];
        delete _guardians[oldRoot];
        recoveryNonces[oldRoot]++;
        _clearRecovery(oldRoot);

        emit RecoveryFinalized(oldRoot, newRoot);
    }

    function _clearRecovery(address oldRoot) internal {
        address[] storage guardianList = _guardians[oldRoot];
        for (uint256 i = 0; i < guardianList.length; i++) {
            recoveryApproved[oldRoot][guardianList[i]] = false;
        }
        pendingNewRoot[oldRoot] = address(0);
        recoveryApprovals[oldRoot] = 0;
        recoveryInitiatedAt[oldRoot] = 0;
    }
}
