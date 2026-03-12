// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {ExecutionGateway} from "./ExecutionGateway.sol";
import {IdentityStore} from "./IdentityStore.sol";
import {IAccount, PackedUserOperation} from "./IAccount.sol";

/// @title HuntKeyAccount — ERC-4337 Compatible Sovereign Identity Account
/// @notice Extends ExecutionGateway with ERC-4337 validateUserOp support.
///         The UserOperation.signature field carries the 3-layer chain:
///         abi.encode(SessionParams, IntentParams)
///         Returns compliant validationData packed as (authorizer, validUntil, validAfter).
contract HuntKeyAccount is ExecutionGateway, IAccount {
    // --- Custom errors ---
    error OnlyEntryPoint();
    error RecoveryBlocksUserOp();
    error ClaimNotSatisfied();
    error MulticallFailed(uint256 index);

    // --- ERC-4337 state ---
    address public entryPoint;

    // --- Credential/Claim state ---
    mapping(address => mapping(bytes32 => bool)) public userClaims;

    // --- Recovery management selectors (allowed during RecoveryPending) ---
    bytes4 private constant CANCEL_RECOVERY_SEL = bytes4(keccak256("cancelRecovery(address)"));
    bytes4 private constant SUPPORT_RECOVERY_SEL = bytes4(keccak256("supportRecovery(address,uint8,bytes32,bytes32)"));
    bytes4 private constant FINALIZE_RECOVERY_SEL = bytes4(keccak256("finalizeRecovery(address)"));

    // --- Events ---
    event EntryPointUpdated(address indexed oldEntryPoint, address indexed newEntryPoint);
    event ClaimSet(address indexed account, bytes32 indexed claim, bool value);
    event UserOpValidated(address indexed account, bytes32 indexed userOpHash);
    event MulticallExecuted(address indexed session, uint256 callCount);

    // --- Structs ---
    struct Call {
        address target;
        uint256 value;
        bytes data;
    }

    modifier onlyEntryPoint() {
        if (msg.sender != entryPoint) revert OnlyEntryPoint();
        _;
    }

    /// @notice Set the ERC-4337 EntryPoint address. Only callable by owner.
    function setEntryPoint(address _entryPoint) external onlyOwner {
        address old = entryPoint;
        entryPoint = _entryPoint;
        emit EntryPointUpdated(old, _entryPoint);
    }

    /// @notice Set a claim for an account. Only callable by owner.
    /// @param account The account to set the claim for.
    /// @param claim The bytes32 claim identifier.
    /// @param value True to grant, false to revoke.
    function setClaim(address account, bytes32 claim, bool value) external onlyOwner {
        userClaims[account][claim] = value;
        emit ClaimSet(account, claim, value);
    }

    /// @notice Check whether a claim is satisfied. Returns true if the claim is
    ///         bytes32(0) (no claim required) or if the account holds the claim.
    /// @param account The account to check.
    /// @param claim The required claim.
    function checkClaim(address account, bytes32 claim) public view returns (bool) {
        if (claim == bytes32(0)) return true;
        return userClaims[account][claim];
    }

    /// @notice Pack ERC-4337 validationData from components.
    /// @param sigFailed True if signature validation failed.
    /// @param validUntil Expiration timestamp (0 = no limit).
    /// @param validAfter Earliest valid timestamp (0 = immediately valid).
    /// @return Packed uint256: authorizer(160) | validUntil(48) | validAfter(48).
    function _packValidationData(
        bool sigFailed,
        uint48 validUntil,
        uint48 validAfter
    ) internal pure returns (uint256) {
        return (sigFailed ? 1 : 0)
            | (uint256(validUntil) << 160)
            | (uint256(validAfter) << 208);
    }

    /// @notice Check if a 4-byte selector is a recovery management function.
    function _isRecoveryManagementSelector(bytes4 sel) internal pure returns (bool) {
        return sel == CANCEL_RECOVERY_SEL
            || sel == SUPPORT_RECOVERY_SEL
            || sel == FINALIZE_RECOVERY_SEL;
    }

    /// @notice Validate a UserOperation per ERC-4337.
    ///         The signature field must contain abi.encode(SessionParams, IntentParams).
    ///         Blocks all operations when identity state is RecoveryPending,
    ///         EXCEPT recovery management (cancelRecovery, supportRecovery, finalizeRecovery).
    ///         Returns compliant validationData: (authorizer, validUntil, validAfter).
    /// @param userOp The packed UserOperation.
    /// @param userOpHash The hash of the UserOperation (used for event logging).
    /// @param missingAccountFunds Amount to pre-fund the EntryPoint.
    /// @return validationData Packed (authorizer, validUntil, validAfter).
    function validateUserOp(
        PackedUserOperation calldata userOp,
        bytes32 userOpHash,
        uint256 missingAccountFunds
    ) external onlyEntryPoint returns (uint256 validationData) {
        // Decode the 3-layer chain from signature
        (SessionParams memory session, IntentParams memory intent) =
            abi.decode(userOp.signature, (SessionParams, IntentParams));

        // --- Identity state check ---
        if (identityState[session.parent] == IdentityState.RecoveryPending) {
            // Allow recovery management calls through, block everything else
            if (userOp.callData.length >= 4) {
                bytes4 selector = bytes4(userOp.callData[:4]);
                if (!_isRecoveryManagementSelector(selector)) {
                    revert RecoveryBlocksUserOp();
                }
                // Recovery management: skip full 3-layer validation, pre-fund, return success.
                // The recovery functions enforce their own authorization (guardian sigs, root checks).
                if (missingAccountFunds > 0) {
                    (bool success,) = payable(entryPoint).call{value: missingAccountFunds}("");
                    if (!success) return _packValidationData(true, 0, 0);
                }
                emit UserOpValidated(userOp.sender, userOpHash);
                return _packValidationData(false, uint48(session.expiration), 0);
            }
            revert RecoveryBlocksUserOp();
        }
        if (identityState[session.parent] != IdentityState.Active) {
            return _packValidationData(true, 0, 0);
        }

        // --- Layer 1: Validate session certificate ---
        if (block.timestamp > session.expiration) return _packValidationData(true, 0, 0);
        if (session.chainId != uint64(block.chainid)) return _packValidationData(true, 0, 0);

        address sessionSigner = _recoverSessionSignerMem(session);
        if (!authorizedKeys[sessionSigner]) return _packValidationData(true, 0, 0);
        if (sessionSigner != session.parent) return _packValidationData(true, 0, 0);

        // --- One-time use enforcement ---
        if (usedSessionKeys[session.session]) return _packValidationData(true, 0, 0);
        usedSessionKeys[session.session] = true;

        // --- Layer 2: Validate intent signed by session key ---
        if (block.timestamp > intent.expiration) return _packValidationData(true, 0, 0);

        address intentSigner = _recoverIntentSignerMem(intent);
        if (intentSigner != session.session) return _packValidationData(true, 0, 0);

        // --- Layer 3: Scope enforcement ---
        if (intent.functionSig != session.scope) return _packValidationData(true, 0, 0);
        if (intent.targetContract != session.target) return _packValidationData(true, 0, 0);

        // --- Value bounds from session certificate ---
        if (intent.maxValue > session.maxValue) return _packValidationData(true, 0, 0);

        // --- Session epoch enforcement ---
        if (intent.sessionEpoch != sessionEpoch[session.parent]) return _packValidationData(true, 0, 0);

        // --- Credential/Claim check ---
        if (!checkClaim(session.parent, intent.requiredClaim)) {
            revert ClaimNotSatisfied();
        }

        // --- Pre-fund the EntryPoint ---
        if (missingAccountFunds > 0) {
            (bool success,) = payable(entryPoint).call{value: missingAccountFunds}("");
            if (!success) return _packValidationData(true, 0, 0);
        }

        emit UserOpValidated(userOp.sender, userOpHash);
        // Return packed validationData: success, validUntil = session expiration, validAfter = 0
        return _packValidationData(false, uint48(session.expiration), 0);
    }

    /// @notice Execute a batch of calls (multicall). Validates calldata hash
    ///         against the intent's callDataHash for the entire batch.
    /// @param session The session certificate parameters.
    /// @param intent The intent parameters (callDataHash must match keccak256(abi.encode(calls))).
    /// @param calls Array of calls to execute.
    function executeMulticall(
        SessionParams calldata session,
        IntentParams calldata intent,
        Call[] calldata calls
    ) external payable {
        // --- Identity state check ---
        if (identityState[session.parent] != IdentityState.Active) revert IdentityNotActive();

        // --- Layer 1: Validate session certificate ---
        if (block.timestamp > session.expiration) revert SessionExpired();
        if (session.chainId != uint64(block.chainid)) revert SessionChainMismatch();

        address sessionSigner = _recoverSessionSigner(session);
        if (!authorizedKeys[sessionSigner]) revert SessionParentNotAuthorized();
        if (sessionSigner != session.parent) revert SignerNotDeclaredParent();

        // --- One-time use enforcement ---
        if (usedSessionKeys[session.session]) revert SessionKeyAlreadyUsed();
        usedSessionKeys[session.session] = true;

        // --- Layer 2: Validate intent signed by session key ---
        if (block.timestamp > intent.expiration) revert IntentExpired();
        if (msg.value > intent.maxValue) revert ValueExceedsCap();

        address intentSigner = _recoverIntentSigner(intent);
        if (intentSigner != session.session) revert IntentNotSignedBySessionKey();

        // --- Scope enforcement ---
        if (intent.functionSig != session.scope) revert IntentScopeMismatch();

        // --- Value bounds from session certificate ---
        if (intent.maxValue > session.maxValue) revert IntentExceedsSessionCap();

        // --- Session epoch enforcement ---
        if (intent.sessionEpoch != sessionEpoch[session.parent]) revert SessionEpochMismatch();

        // --- Multicall calldata hash verification ---
        bytes32 multicallHash = keccak256(abi.encode(calls));
        if (multicallHash != intent.callDataHash) revert CalldataHashMismatch();

        // --- Credential/Claim check ---
        if (!checkClaim(session.parent, intent.requiredClaim)) {
            revert ClaimNotSatisfied();
        }

        // --- Execute all calls ---
        for (uint256 i = 0; i < calls.length; i++) {
            if (calls[i].target != intent.targetContract) revert CallTargetMismatch();

            (bool success, bytes memory returnData) = calls[i].target.call{value: calls[i].value}(calls[i].data);
            if (!success) {
                revert MulticallFailed(i);
            }
        }

        emit MulticallExecuted(session.session, calls.length);
        emit IntentExecuted(session.parent, session.session, intent.functionSig);
    }

    // --- Internal memory-compatible signature recovery ---
    // abi.decode produces memory structs, but parent _recover* functions expect calldata.
    // These overloads handle memory inputs for validateUserOp.

    function _recoverSessionSignerMem(SessionParams memory p) internal view returns (address) {
        _validateSigParams(p.v, p.s);

        bytes32 structHash = keccak256(
            abi.encode(
                SESSION_TYPEHASH,
                p.session,
                p.parent,
                p.scope,
                p.target,
                p.maxValue,
                p.expiration,
                p.chainId
            )
        );
        bytes32 digest = keccak256(
            abi.encodePacked("\x19\x01", DOMAIN_SEPARATOR, structHash)
        );

        address signer = ecrecover(digest, p.v, p.r, p.s);
        if (signer == address(0)) revert EcrecoverFailed();
        return signer;
    }

    function _recoverIntentSignerMem(IntentParams memory p) internal view returns (address) {
        _validateSigParams(p.v, p.s);
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
        bytes32 structHash = keccak256(bytes.concat(first, second));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", DOMAIN_SEPARATOR, structHash));
        address signer = ecrecover(digest, p.v, p.r, p.s);
        if (signer == address(0)) revert EcrecoverFailed();
        return signer;
    }

    /// @notice Allow the account to receive ETH (required for ERC-4337 pre-funding).
    receive() external payable {}
}
