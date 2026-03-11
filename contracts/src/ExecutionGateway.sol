// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {IdentityStore} from "./IdentityStore.sol";

/// @title ExecutionGateway — Scoped execution with ephemeral session keys
/// @notice Inherits IdentityStore for identity state and adds session-key-based execution.
///         Flow: Root authorizes Action Key → Action Key signs SessionCertificate → Session Key signs Intent → execute()
contract ExecutionGateway is IdentityStore {
    // --- Custom errors ---
    error IdentityNotActive();
    error SessionExpired();
    error SessionChainMismatch();
    error SessionParentNotAuthorized();
    error SignerNotDeclaredParent();
    error SessionKeyAlreadyUsed();
    error IntentNotSignedBySessionKey();
    error IntentScopeMismatch();
    error IntentTargetMismatch();
    error CallTargetMismatch();
    error CalldataTooShort();
    error SelectorMismatch();
    error CalldataHashMismatch();
    error IntentExceedsSessionCap();

    // --- Session EIP-712 constant ---
    bytes32 public constant SESSION_TYPEHASH =
        keccak256(
            "SessionCertificate(address session,address parent,bytes4 scope,address target,uint128 maxValue,uint64 expiration,uint64 chainId)"
        );

    // --- Structs ---
    struct SessionParams {
        address session;
        address parent;
        bytes4 scope;
        address target;
        uint128 maxValue;
        uint64 expiration;
        uint64 chainId;
        uint8 v;
        bytes32 r;
        bytes32 s;
    }

    // --- Session state ---
    mapping(address => bool) public usedSessionKeys;

    // --- Events ---
    event Executed(
        address indexed session,
        address indexed target,
        bytes4 selector,
        uint128 value,
        bool success
    );

    // --- Session signature recovery ---
    function _recoverSessionSigner(SessionParams calldata p) internal view returns (address) {
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

    /// @notice Execute a call after validating the 3-layer signing chain:
    ///         1. SessionCertificate signed by an authorized key (Action Key)
    ///         2. Intent signed by the session address
    ///         3. Scope enforcement: callData selector == intent.functionSig, target == intent.targetContract
    ///         4. Session key burned after single use
    function execute(
        SessionParams calldata session,
        IntentParams calldata intent,
        address target,
        bytes calldata callData
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

        // --- Layer 3: Scope enforcement ---
        if (intent.functionSig != session.scope) revert IntentScopeMismatch();
        if (intent.targetContract != session.target) revert IntentTargetMismatch();
        if (target != intent.targetContract) revert CallTargetMismatch();
        if (callData.length < 4) revert CalldataTooShort();
        if (bytes4(callData[:4]) != intent.functionSig) revert SelectorMismatch();

        // --- CallData hash verification ---
        if (keccak256(callData) != intent.callDataHash) revert CalldataHashMismatch();

        // --- Value bounds from session certificate ---
        if (intent.maxValue > session.maxValue) revert IntentExceedsSessionCap();

        // --- Forward the call ---
        (bool success, bytes memory returnData) = target.call{value: msg.value}(callData);
        if (!success) {
            // Bubble up the revert reason
            assembly {
                revert(add(returnData, 32), mload(returnData))
            }
        }

        emit Executed(session.session, target, intent.functionSig, uint128(msg.value), success);
        emit IntentExecuted(session.parent, session.session, intent.functionSig);
    }
}
