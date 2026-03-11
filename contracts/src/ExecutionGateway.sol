// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {IdentityStore} from "./IdentityStore.sol";

/// @title ExecutionGateway — Scoped execution with ephemeral session keys
/// @notice Inherits IdentityStore for identity state and adds session-key-based execution.
///         Flow: Root authorizes Action Key → Action Key signs SessionCertificate → Session Key signs Intent → execute()
contract ExecutionGateway is IdentityStore {
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
        require(signer != address(0), "session ecrecover failed");
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
        // --- Layer 1: Validate session certificate ---
        require(block.timestamp <= session.expiration, "session expired");
        require(session.chainId == uint64(block.chainid), "session chain mismatch");

        address sessionSigner = _recoverSessionSigner(session);
        require(authorizedKeys[sessionSigner], "session parent not authorized");
        require(sessionSigner == session.parent, "signer is not declared parent");

        // --- One-time use enforcement ---
        require(!usedSessionKeys[session.session], "session key already used");
        usedSessionKeys[session.session] = true;

        // --- Layer 2: Validate intent signed by session key ---
        require(block.timestamp <= intent.expiration, "intent expired");
        require(msg.value <= intent.maxValue, "value exceeds cap");

        address intentSigner = _recoverIntentSigner(intent);
        require(intentSigner == session.session, "intent not signed by session key");

        // --- Layer 3: Scope enforcement ---
        require(intent.functionSig == session.scope, "intent scope mismatch");
        require(intent.targetContract == session.target, "intent target mismatch");
        require(target == intent.targetContract, "call target mismatch");
        require(callData.length >= 4, "calldata too short");
        require(bytes4(callData[:4]) == intent.functionSig, "selector mismatch");

        // --- Value bounds from session certificate ---
        require(intent.maxValue <= session.maxValue, "intent exceeds session cap");

        // --- Forward the call ---
        (bool success, bytes memory returnData) = target.call{value: msg.value}(callData);
        if (!success) {
            // Bubble up the revert reason
            assembly {
                revert(add(returnData, 32), mload(returnData))
            }
        }

        emit Executed(session.session, target, intent.functionSig, uint128(msg.value), success);
    }
}
