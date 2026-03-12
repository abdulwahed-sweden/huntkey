// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @title IdentityStore — Identity state, delegation verification, and social recovery
abstract contract IdentityStore {
    // --- Identity state enum ---
    enum IdentityState { Active, RecoveryPending, Frozen }

    // --- Custom errors ---
    error NotOwner();
    error InvalidGuardianCount();
    error MalleableSignature();
    error InvalidVValue();
    error EcrecoverFailed();
    error IntentExpired();
    error ValueExceedsCap();
    error UnauthorizedKey();
    error InvalidNonce();
    error DelegationRequired();
    error DelegationExpired();
    error UnregisteredProver();
    error InvalidDelegationNonce();
    error SignerNotDelegate();
    error ScopeMismatch();
    error ExceedsDelegationCap();
    error NotFrozen();
    error NotAuthorized();
    error OldRootNotRegistered();
    error InvalidNewRoot();
    error RecoveryAlreadyPending();
    error NotAGuardian();
    error NoPendingRecovery();
    error AlreadyApproved();
    error OnlyOldRootCanCancel();
    error ThresholdNotMet();
    error TimelockNotStarted();
    error TimelockNotExpired();
    error SessionEpochMismatch();

    // --- EIP-712 constants ---
    bytes32 public constant INTENT_TYPEHASH =
        keccak256(
            "SovereignIntent(address targetContract,bytes4 functionSig,address recipient,address assetAddress,bytes32 callDataHash,uint128 maxValue,uint64 expiration,uint64 chainId,uint64 nonce,uint64 sessionEpoch,uint64 gasLimit,uint128 maxFeePerGas,bytes32 requiredClaim)"
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
        address recipient;
        address assetAddress;
        bytes32 callDataHash;
        uint128 maxValue;
        uint64 expiration;
        uint64 chainId;
        uint64 nonce;
        uint64 sessionEpoch;
        uint64 gasLimit;
        uint128 maxFeePerGas;
        bytes32 requiredClaim;
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

    // --- Identity state & session epoch ---
    mapping(address => IdentityState) public identityState;
    mapping(address => uint256) public sessionEpoch;

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
    event IdentityFrozen(address indexed root);
    event IdentityUnfrozen(address indexed root);
    event AllSessionsCancelled(address indexed root, uint256 newEpoch);

    // --- v1.1 Events ---
    event IntentExecuted(address indexed root, address indexed sessionKey, bytes4 selector);
    event SessionInvalidated(address indexed root, uint256 newEpoch);
    event RecoveryStateChanged(address indexed root, IdentityState newState);
    event DelegationEndorsed(address indexed root, address indexed delegate, bytes4 scope);

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
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
        if (guardianList.length < 3 || guardianList.length > 5) revert InvalidGuardianCount();
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
        if (uint256(s) > SECP256K1_N_DIV_2) revert MalleableSignature();
        if (v != 27 && v != 28) revert InvalidVValue();
    }

    function _recoverIntentSigner(IntentParams calldata p) internal view returns (address) {
        _validateSigParams(p.v, p.s);
        bytes32 structHash = _intentStructHash(p);
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", DOMAIN_SEPARATOR, structHash));
        address signer = ecrecover(digest, p.v, p.r, p.s);
        if (signer == address(0)) revert EcrecoverFailed();
        return signer;
    }

    function _intentStructHash(IntentParams calldata p) internal view returns (bytes32) {
        // Split abi.encode into two halves to avoid stack-too-deep
        bytes memory first = abi.encode(
            INTENT_TYPEHASH, p.targetContract, p.functionSig,
            p.recipient, p.assetAddress, p.callDataHash,
            p.maxValue
        );
        bytes memory second = abi.encode(
            p.expiration, p.chainId, p.nonce,
            p.sessionEpoch, p.gasLimit, p.maxFeePerGas, p.requiredClaim
        );
        return keccak256(bytes.concat(first, second));
    }

    function _recoverDelegationProver(DelegationParams calldata d) internal view returns (address) {
        _validateSigParams(d.v, d.s);
        bytes32 structHash = keccak256(
            abi.encode(DELEGATION_TYPEHASH, d.delegate, d.scope, d.maxValue, d.expiration, d.chainId, d.nonce)
        );
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", DOMAIN_SEPARATOR, structHash));
        address prover = ecrecover(digest, d.v, d.r, d.s);
        if (prover == address(0)) revert EcrecoverFailed();
        return prover;
    }

    function _recoverRecoverySigner(
        address oldRoot, address newRoot, uint64 chainId, uint64 nonce, uint8 v, bytes32 r, bytes32 s
    ) internal view returns (address) {
        _validateSigParams(v, s);
        bytes32 structHash = keccak256(abi.encode(RECOVERY_TYPEHASH, oldRoot, newRoot, chainId, nonce));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", DOMAIN_SEPARATOR, structHash));
        address signer = ecrecover(digest, v, r, s);
        if (signer == address(0)) revert EcrecoverFailed();
        return signer;
    }

    // --- Direct-authorization intent validation ---

    function validateIntent(IntentParams calldata p) external payable {
        if (block.timestamp > p.expiration) revert IntentExpired();
        if (msg.value > p.maxValue) revert ValueExceedsCap();

        address signer = _recoverIntentSigner(p);
        if (!authorizedKeys[signer]) revert UnauthorizedKey();
        if (p.nonce != nonces[signer]) revert InvalidNonce();
        if (p.sessionEpoch != sessionEpoch[signer]) revert SessionEpochMismatch();
        nonces[signer]++;

        emit IntentValidated(signer, p.targetContract, p.maxValue, p.nonce);
    }

    // --- Delegated verification ---

    bool internal _gateOpen;

    modifier gatedFunction() {
        if (!_gateOpen) revert DelegationRequired();
        _;
        _gateOpen = false;
    }

    function validateDelegatedIntent(
        DelegationParams calldata delegation, IntentParams calldata intent
    ) external payable {
        if (block.timestamp > delegation.expiration) revert DelegationExpired();
        address prover = _recoverDelegationProver(delegation);
        if (!authorizedProvers[prover]) revert UnregisteredProver();
        if (delegation.nonce != delegationNonces[prover]) revert InvalidDelegationNonce();
        delegationNonces[prover]++;

        if (block.timestamp > intent.expiration) revert IntentExpired();
        if (msg.value > intent.maxValue) revert ValueExceedsCap();
        address intentSigner = _recoverIntentSigner(intent);

        if (intentSigner != delegation.delegate) revert SignerNotDelegate();
        if (intent.functionSig != delegation.scope) revert ScopeMismatch();
        if (intent.maxValue > delegation.maxValue) revert ExceedsDelegationCap();

        if (intent.nonce != nonces[delegation.delegate]) revert InvalidNonce();
        nonces[delegation.delegate]++;

        _gateOpen = true;

        emit DelegatedIntentValidated(prover, delegation.delegate, intent.targetContract, intent.maxValue);
        emit DelegationEndorsed(prover, delegation.delegate, delegation.scope);
    }

    function gatedPurchase(address, uint128) external gatedFunction {}

    // --- Identity State Management ---

    function freezeIdentity(address root) external onlyOwner {
        identityState[root] = IdentityState.Frozen;
        emit IdentityFrozen(root);
        emit RecoveryStateChanged(root, IdentityState.Frozen);
    }

    function unfreezeIdentity(address root) external onlyOwner {
        if (identityState[root] != IdentityState.Frozen) revert NotFrozen();
        identityState[root] = IdentityState.Active;
        emit IdentityUnfrozen(root);
        emit RecoveryStateChanged(root, IdentityState.Active);
    }

    function cancelAllSessions(address root) external {
        if (msg.sender != root && msg.sender != owner) revert NotAuthorized();
        sessionEpoch[root]++;
        emit AllSessionsCancelled(root, sessionEpoch[root]);
        emit SessionInvalidated(root, sessionEpoch[root]);
    }

    // --- Social Recovery ---

    function initiateRecovery(address oldRoot, address newRoot, uint8 v, bytes32 r, bytes32 s) external {
        if (!authorizedProvers[oldRoot]) revert OldRootNotRegistered();
        if (newRoot == address(0)) revert InvalidNewRoot();
        if (pendingNewRoot[oldRoot] != address(0)) revert RecoveryAlreadyPending();

        address guardian = _recoverRecoverySigner(oldRoot, newRoot, uint64(block.chainid), uint64(recoveryNonces[oldRoot]), v, r, s);
        if (!isGuardian(oldRoot, guardian)) revert NotAGuardian();

        pendingNewRoot[oldRoot] = newRoot;
        recoveryApprovals[oldRoot] = 1;
        recoveryApproved[oldRoot][guardian] = true;
        identityState[oldRoot] = IdentityState.RecoveryPending;

        emit RecoveryInitiated(oldRoot, newRoot, guardian);
        emit RecoveryStateChanged(oldRoot, IdentityState.RecoveryPending);
    }

    function supportRecovery(address oldRoot, uint8 v, bytes32 r, bytes32 s) external {
        address newRoot = pendingNewRoot[oldRoot];
        if (newRoot == address(0)) revert NoPendingRecovery();

        address guardian = _recoverRecoverySigner(oldRoot, newRoot, uint64(block.chainid), uint64(recoveryNonces[oldRoot]), v, r, s);
        if (!isGuardian(oldRoot, guardian)) revert NotAGuardian();
        if (recoveryApproved[oldRoot][guardian]) revert AlreadyApproved();

        recoveryApproved[oldRoot][guardian] = true;
        recoveryApprovals[oldRoot]++;

        if (recoveryApprovals[oldRoot] >= GUARDIAN_THRESHOLD && recoveryInitiatedAt[oldRoot] == 0) {
            recoveryInitiatedAt[oldRoot] = block.timestamp;
        }

        emit RecoverySupported(oldRoot, guardian, recoveryApprovals[oldRoot]);
    }

    function cancelRecovery(address oldRoot) external {
        if (msg.sender != oldRoot) revert OnlyOldRootCanCancel();
        if (pendingNewRoot[oldRoot] == address(0)) revert NoPendingRecovery();
        _clearRecovery(oldRoot);
        identityState[oldRoot] = IdentityState.Active;
        emit RecoveryCancelled(oldRoot);
        emit RecoveryStateChanged(oldRoot, IdentityState.Active);
    }

    function finalizeRecovery(address oldRoot) external {
        address newRoot = pendingNewRoot[oldRoot];
        if (newRoot == address(0)) revert NoPendingRecovery();
        if (recoveryApprovals[oldRoot] < GUARDIAN_THRESHOLD) revert ThresholdNotMet();
        if (recoveryInitiatedAt[oldRoot] == 0) revert TimelockNotStarted();
        if (block.timestamp < recoveryInitiatedAt[oldRoot] + RECOVERY_WINDOW) revert TimelockNotExpired();

        authorizedProvers[oldRoot] = false;
        authorizedProvers[newRoot] = true;
        nonces[newRoot] = 0;
        delegationNonces[newRoot] = 0;
        _guardians[newRoot] = _guardians[oldRoot];
        delete _guardians[oldRoot];
        recoveryNonces[oldRoot]++;
        _clearRecovery(oldRoot);
        identityState[oldRoot] = IdentityState.Active;
        identityState[newRoot] = IdentityState.Active;

        emit RecoveryFinalized(oldRoot, newRoot);
        emit RecoveryStateChanged(oldRoot, IdentityState.Active);
        emit RecoveryStateChanged(newRoot, IdentityState.Active);
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
