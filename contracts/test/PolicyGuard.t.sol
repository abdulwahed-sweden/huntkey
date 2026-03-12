// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {ExecutionGateway} from "../src/ExecutionGateway.sol";
import {HuntKeyAccount} from "../src/HuntKeyAccount.sol";
import {IdentityStore} from "../src/IdentityStore.sol";
import {IAccount, PackedUserOperation} from "../src/IAccount.sol";
import {Test} from "forge-std/Test.sol";

/// @dev Dummy target contract for execute() tests
contract DummyTarget {
    uint256 public lastValue;
    address public lastSender;
    address public lastRecipient;

    function doSomething(address recipient, uint256 val) external payable {
        lastValue = val;
        lastSender = msg.sender;
        lastRecipient = recipient;
    }

    function otherFunction(uint256 val) external {
        lastValue = val;
    }

    receive() external payable {}
}

contract ExecutionGatewayTest is Test {
    ExecutionGateway guard;

    uint256 constant ACTION_KEY = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;
    address actionSigner;

    uint256 constant SECP256K1_N =
        0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141;

    function setUp() public {
        guard = new ExecutionGateway();
        actionSigner = vm.addr(ACTION_KEY);
        guard.authorizeKey(actionSigner);
    }

    /// @dev Build the EIP-712 intent digest (defaults sessionEpoch/gasLimit/maxFeePerGas/requiredClaim/claim/paymaster to zero).
    function _digest(
        address targetContract,
        bytes4 functionSig,
        address recipient,
        address assetAddress,
        bytes32 dataHash,
        uint128 maxValue,
        uint64 expiration,
        uint64 intentChainId,
        uint64 nonce
    ) internal view returns (bytes32) {
        bytes memory first = abi.encode(
            guard.INTENT_TYPEHASH(), targetContract, functionSig,
            recipient, assetAddress, dataHash, maxValue
        );
        bytes memory second = abi.encode(
            expiration, intentChainId, nonce,
            uint64(0), uint64(0), uint128(0),
            uint128(0), bytes32(0)
        );
        bytes memory third = abi.encode(
            bytes32(0), uint8(0), address(0)
        );
        bytes32 structHash = keccak256(bytes.concat(first, second, third));
        return keccak256(
            abi.encodePacked("\x19\x01", guard.DOMAIN_SEPARATOR(), structHash)
        );
    }

    /// @dev Build the EIP-712 delegation digest.
    function _delegationDigest(
        address delegate,
        bytes4 scope,
        uint128 maxValue,
        uint64 expiration,
        uint64 chainId,
        uint64 nonce
    ) internal view returns (bytes32) {
        bytes32 structHash = keccak256(
            abi.encode(
                guard.DELEGATION_TYPEHASH(),
                delegate,
                scope,
                maxValue,
                expiration,
                chainId,
                nonce
            )
        );
        return keccak256(
            abi.encodePacked("\x19\x01", guard.DOMAIN_SEPARATOR(), structHash)
        );
    }

    /// @dev Build the EIP-712 recovery digest.
    function _recoveryDigest(
        address oldRoot,
        address newRoot,
        uint64 chainId,
        uint64 nonce
    ) internal view returns (bytes32) {
        bytes32 structHash = keccak256(
            abi.encode(
                guard.RECOVERY_TYPEHASH(),
                oldRoot,
                newRoot,
                chainId,
                nonce
            )
        );
        return keccak256(
            abi.encodePacked("\x19\x01", guard.DOMAIN_SEPARATOR(), structHash)
        );
    }

    /// @dev Build the EIP-712 session digest.
    function _sessionDigest(
        address session,
        address parent,
        bytes4 scope,
        address target,
        uint128 maxValue,
        uint64 expiration,
        uint64 chainId
    ) internal view returns (bytes32) {
        bytes32 structHash = keccak256(
            abi.encode(
                guard.SESSION_TYPEHASH(),
                session,
                parent,
                scope,
                target,
                maxValue,
                expiration,
                chainId
            )
        );
        return keccak256(
            abi.encodePacked("\x19\x01", guard.DOMAIN_SEPARATOR(), structHash)
        );
    }

    /// @dev Build IntentParams struct with default sessionEpoch/gas/claim fields.
    function _makeIntent(
        address target, bytes4 fnSig, address recipient, address asset,
        bytes32 dataHash, uint128 maxVal, uint64 exp, uint64 chainId, uint64 nonce,
        uint8 v, bytes32 r, bytes32 s
    ) internal pure returns (IdentityStore.IntentParams memory) {
        return IdentityStore.IntentParams({
            targetContract: target, functionSig: fnSig, recipient: recipient,
            assetAddress: asset, callDataHash: dataHash, maxValue: maxVal,
            expiration: exp, chainId: chainId, nonce: nonce,
            sessionEpoch: 0, gasLimit: 0, maxFeePerGas: 0, maxPriorityFeePerGas: 0,
            requiredClaim: bytes32(0), claimProofHash: bytes32(0), paymasterMode: 0, paymaster: address(0),
            v: v, r: r, s: s
        });
    }

    // =======================================================================
    // Direct authorization tests (updated for v2.0 intent)
    // =======================================================================

    function testValidateIntent() public {
        address target = address(0xBEEF);
        bytes4 fnSig = bytes4(0xa9059cbb);
        address recipient = address(0x1234);
        address asset = address(0);
        bytes32 dataHash = keccak256("test");
        uint128 maxVal = 1 ether;
        uint64 exp = uint64(block.timestamp + 1 hours);
        uint64 chainId = uint64(block.chainid);
        uint64 nonce = 0;

        bytes32 digest = _digest(target, fnSig, recipient, asset, dataHash, maxVal, exp, chainId, nonce);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ACTION_KEY, digest);

        guard.validateIntent{value: 0.5 ether}(
            _makeIntent(target, fnSig, recipient, asset, dataHash, maxVal, exp, chainId, nonce, v, r, s)
        );

        assertEq(guard.nonces(actionSigner), 1);
    }

    function testRevertExpiredIntent() public {
        address target = address(0xBEEF);
        bytes4 fnSig = bytes4(0xa9059cbb);
        address recipient = address(0x1234);
        address asset = address(0);
        bytes32 dataHash = keccak256("test");
        uint128 maxVal = 1 ether;
        uint64 exp = uint64(block.timestamp - 1);
        uint64 chainId = uint64(block.chainid);
        uint64 nonce = 0;

        bytes32 digest = _digest(target, fnSig, recipient, asset, dataHash, maxVal, exp, chainId, nonce);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ACTION_KEY, digest);

        vm.expectRevert(IdentityStore.IntentExpired.selector);
        guard.validateIntent(
            _makeIntent(target, fnSig, recipient, asset, dataHash, maxVal, exp, chainId, nonce, v, r, s)
        );
    }

    function testRevertValueExceedsCap() public {
        address target = address(0xBEEF);
        bytes4 fnSig = bytes4(0xa9059cbb);
        address recipient = address(0x1234);
        address asset = address(0);
        bytes32 dataHash = keccak256("test");
        uint128 maxVal = 0.5 ether;
        uint64 exp = uint64(block.timestamp + 1 hours);
        uint64 chainId = uint64(block.chainid);
        uint64 nonce = 0;

        bytes32 digest = _digest(target, fnSig, recipient, asset, dataHash, maxVal, exp, chainId, nonce);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ACTION_KEY, digest);

        vm.expectRevert(IdentityStore.ValueExceedsCap.selector);
        guard.validateIntent{value: 1 ether}(
            _makeIntent(target, fnSig, recipient, asset, dataHash, maxVal, exp, chainId, nonce, v, r, s)
        );
    }

    function testRevertUnauthorizedKey() public {
        uint256 rogue = 0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef;

        address target = address(0xBEEF);
        bytes4 fnSig = bytes4(0xa9059cbb);
        address recipient = address(0x1234);
        address asset = address(0);
        bytes32 dataHash = keccak256("test");
        uint128 maxVal = 1 ether;
        uint64 exp = uint64(block.timestamp + 1 hours);
        uint64 chainId = uint64(block.chainid);
        uint64 nonce = 0;

        bytes32 digest = _digest(target, fnSig, recipient, asset, dataHash, maxVal, exp, chainId, nonce);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(rogue, digest);

        vm.expectRevert(IdentityStore.UnauthorizedKey.selector);
        guard.validateIntent(
            _makeIntent(target, fnSig, recipient, asset, dataHash, maxVal, exp, chainId, nonce, v, r, s)
        );
    }

    function testRevertNonceReplay() public {
        address target = address(0xBEEF);
        bytes4 fnSig = bytes4(0xa9059cbb);
        address recipient = address(0x1234);
        address asset = address(0);
        bytes32 dataHash = keccak256("test");
        uint128 maxVal = 1 ether;
        uint64 exp = uint64(block.timestamp + 1 hours);
        uint64 chainId = uint64(block.chainid);
        uint64 nonce = 0;

        bytes32 digest = _digest(target, fnSig, recipient, asset, dataHash, maxVal, exp, chainId, nonce);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ACTION_KEY, digest);

        guard.validateIntent(
            _makeIntent(target, fnSig, recipient, asset, dataHash, maxVal, exp, chainId, nonce, v, r, s)
        );

        vm.expectRevert(IdentityStore.InvalidNonce.selector);
        guard.validateIntent(
            _makeIntent(target, fnSig, recipient, asset, dataHash, maxVal, exp, chainId, nonce, v, r, s)
        );
    }

    function testRevertMalleableSignature() public {
        address target = address(0xBEEF);
        bytes4 fnSig = bytes4(0xa9059cbb);
        address recipient = address(0x1234);
        address asset = address(0);
        bytes32 dataHash = keccak256("test");
        uint128 maxVal = 1 ether;
        uint64 exp = uint64(block.timestamp + 1 hours);
        uint64 chainId = uint64(block.chainid);
        uint64 nonce = 0;

        bytes32 digest = _digest(target, fnSig, recipient, asset, dataHash, maxVal, exp, chainId, nonce);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ACTION_KEY, digest);

        bytes32 flippedS = bytes32(SECP256K1_N - uint256(s));
        uint8 flippedV = v == 27 ? 28 : 27;

        vm.expectRevert(IdentityStore.MalleableSignature.selector);
        guard.validateIntent(
            _makeIntent(target, fnSig, recipient, asset, dataHash, maxVal, exp, chainId, nonce, flippedV, r, flippedS)
        );
    }

    // =======================================================================
    // Delegated verification tests
    // =======================================================================

    uint256 constant ROOT_KEY = 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d;
    uint256 constant SHOPPING_KEY = 0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a;

    function _setupDelegation() internal returns (address rootAddr, address shopAddr) {
        rootAddr = vm.addr(ROOT_KEY);
        shopAddr = vm.addr(SHOPPING_KEY);
        guard.registerProver(rootAddr);
    }

    function _buildDelegatedCall(
        address shopAddr,
        bytes4 scope,
        uint128 delegationCap,
        uint64 delegationExp,
        uint64 delegationNonce,
        address target,
        bytes4 intentFnSig,
        uint128 intentVal,
        uint64 intentExp,
        uint64 intentNonce
    ) internal view returns (
        IdentityStore.DelegationParams memory del,
        IdentityStore.IntentParams memory intent
    ) {
        uint64 chainId = uint64(block.chainid);

        bytes32 delDigest = _delegationDigest(shopAddr, scope, delegationCap, delegationExp, chainId, delegationNonce);
        (uint8 dV, bytes32 dR, bytes32 dS) = vm.sign(ROOT_KEY, delDigest);

        del = IdentityStore.DelegationParams({
            delegate: shopAddr,
            scope: scope,
            maxValue: delegationCap,
            expiration: delegationExp,
            chainId: chainId,
            nonce: delegationNonce,
            v: dV,
            r: dR,
            s: dS
        });

        bytes32 dataHash = keccak256("delegated-call");
        bytes32 intentDigest = _digest(target, intentFnSig, address(0), address(0), dataHash, intentVal, intentExp, chainId, intentNonce);
        (uint8 iV, bytes32 iR, bytes32 iS) = vm.sign(SHOPPING_KEY, intentDigest);

        intent = IdentityStore.IntentParams({
            targetContract: target,
            functionSig: intentFnSig,
            recipient: address(0),
            assetAddress: address(0),
            callDataHash: dataHash,
            maxValue: intentVal,
            expiration: intentExp,
            chainId: chainId,
            nonce: intentNonce,
            sessionEpoch: 0,
            gasLimit: 0,
            maxFeePerGas: 0,
            maxPriorityFeePerGas: 0,
            requiredClaim: bytes32(0),
            claimProofHash: bytes32(0),
            paymasterMode: 0,
            paymaster: address(0),
            v: iV,
            r: iR,
            s: iS
        });
    }

    function testDelegatedIntentHappyPath() public {
        (address rootAddr, address shopAddr) = _setupDelegation();

        bytes4 scope = bytes4(0xa9059cbb);
        uint64 exp = uint64(block.timestamp + 1 hours);

        (IdentityStore.DelegationParams memory del, IdentityStore.IntentParams memory intent) =
            _buildDelegatedCall(shopAddr, scope, 2 ether, exp, 0, address(0xCAFE), scope, 0.5 ether, exp, 0);

        guard.validateDelegatedIntent{value: 0.3 ether}(del, intent);

        assertEq(guard.delegationNonces(rootAddr), 1);
        assertEq(guard.nonces(shopAddr), 1);
    }

    function testDelegatedIntentRevertExpiredDelegation() public {
        (, address shopAddr) = _setupDelegation();

        bytes4 scope = bytes4(0xa9059cbb);
        uint64 expiredTs = uint64(block.timestamp - 1);
        uint64 futureTs = uint64(block.timestamp + 1 hours);

        (IdentityStore.DelegationParams memory del, IdentityStore.IntentParams memory intent) =
            _buildDelegatedCall(shopAddr, scope, 1 ether, expiredTs, 0, address(0xCAFE), scope, 0.5 ether, futureTs, 0);

        vm.expectRevert(IdentityStore.DelegationExpired.selector);
        guard.validateDelegatedIntent(del, intent);
    }

    function testDelegatedIntentRevertUnregisteredProver() public {
        uint256 rogueRoot = 0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6;
        address shopAddr = vm.addr(SHOPPING_KEY);

        bytes4 scope = bytes4(0xa9059cbb);
        uint64 chainId = uint64(block.chainid);
        uint64 exp = uint64(block.timestamp + 1 hours);

        bytes32 delDigest = _delegationDigest(shopAddr, scope, 1 ether, exp, chainId, 0);
        (uint8 dV, bytes32 dR, bytes32 dS) = vm.sign(rogueRoot, delDigest);

        IdentityStore.DelegationParams memory del = IdentityStore.DelegationParams({
            delegate: shopAddr, scope: scope, maxValue: 1 ether, expiration: exp,
            chainId: chainId, nonce: 0, v: dV, r: dR, s: dS
        });

        bytes32 dataHash = keccak256("test");
        bytes32 intentDigest = _digest(address(0xCAFE), scope, address(0), address(0), dataHash, 0.5 ether, exp, chainId, 0);
        (uint8 iV, bytes32 iR, bytes32 iS) = vm.sign(SHOPPING_KEY, intentDigest);

        IdentityStore.IntentParams memory intent = IdentityStore.IntentParams({
            targetContract: address(0xCAFE), functionSig: scope,
            recipient: address(0), assetAddress: address(0), callDataHash: dataHash,
            maxValue: 0.5 ether, expiration: exp, chainId: chainId, nonce: 0,
            sessionEpoch: 0, gasLimit: 0, maxFeePerGas: 0, maxPriorityFeePerGas: 0, requiredClaim: bytes32(0),
            claimProofHash: bytes32(0), paymasterMode: 0, paymaster: address(0),
            v: iV, r: iR, s: iS
        });

        vm.expectRevert(IdentityStore.UnregisteredProver.selector);
        guard.validateDelegatedIntent(del, intent);
    }

    function testDelegatedIntentRevertScopeMismatch() public {
        (, address shopAddr) = _setupDelegation();

        bytes4 delegationScope = bytes4(0xa9059cbb);
        bytes4 intentScope = bytes4(0x095ea7b3);
        uint64 exp = uint64(block.timestamp + 1 hours);

        (IdentityStore.DelegationParams memory del, IdentityStore.IntentParams memory intent) =
            _buildDelegatedCall(shopAddr, delegationScope, 1 ether, exp, 0, address(0xCAFE), intentScope, 0.5 ether, exp, 0);

        vm.expectRevert(IdentityStore.ScopeMismatch.selector);
        guard.validateDelegatedIntent(del, intent);
    }

    function testDelegatedIntentRevertExceedsDelegationCap() public {
        (, address shopAddr) = _setupDelegation();

        bytes4 scope = bytes4(0xa9059cbb);
        uint64 exp = uint64(block.timestamp + 1 hours);

        (IdentityStore.DelegationParams memory del, IdentityStore.IntentParams memory intent) =
            _buildDelegatedCall(shopAddr, scope, 0.5 ether, exp, 0, address(0xCAFE), scope, 1 ether, exp, 0);

        vm.expectRevert(IdentityStore.ExceedsDelegationCap.selector);
        guard.validateDelegatedIntent(del, intent);
    }

    function testDelegatedIntentRevertDelegationNonceReplay() public {
        (, address shopAddr) = _setupDelegation();

        bytes4 scope = bytes4(0xa9059cbb);
        uint64 exp = uint64(block.timestamp + 1 hours);

        (IdentityStore.DelegationParams memory del1, IdentityStore.IntentParams memory intent1) =
            _buildDelegatedCall(shopAddr, scope, 1 ether, exp, 0, address(0xCAFE), scope, 0.5 ether, exp, 0);

        guard.validateDelegatedIntent(del1, intent1);

        (IdentityStore.DelegationParams memory del2, IdentityStore.IntentParams memory intent2) =
            _buildDelegatedCall(shopAddr, scope, 1 ether, exp, 0, address(0xCAFE), scope, 0.5 ether, exp, 1);

        vm.expectRevert(IdentityStore.InvalidDelegationNonce.selector);
        guard.validateDelegatedIntent(del2, intent2);
    }

    function testGatedFunctionRevertWithoutDelegation() public {
        vm.expectRevert(IdentityStore.DelegationRequired.selector);
        guard.gatedPurchase(address(0xCAFE), 1 ether);
    }

    // =======================================================================
    // Social Recovery tests
    // =======================================================================

    uint256 constant GUARDIAN_KEY_0 = 0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a;
    uint256 constant GUARDIAN_KEY_1 = 0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba;
    uint256 constant GUARDIAN_KEY_2 = 0x92db14e403b83dfe3df233f83dfa3a0d7096f21ca9b0d6d6b8d88b2b4ec1564e;

    uint256 constant OLD_ROOT_KEY = 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d;
    uint256 constant NEW_ROOT_KEY = 0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97;

    function _setupRecovery() internal returns (
        address oldRoot, address newRoot,
        address g0, address g1, address g2
    ) {
        oldRoot = vm.addr(OLD_ROOT_KEY);
        newRoot = vm.addr(NEW_ROOT_KEY);
        g0 = vm.addr(GUARDIAN_KEY_0);
        g1 = vm.addr(GUARDIAN_KEY_1);
        g2 = vm.addr(GUARDIAN_KEY_2);

        guard.registerProver(oldRoot);

        address[] memory guardians = new address[](3);
        guardians[0] = g0;
        guardians[1] = g1;
        guardians[2] = g2;
        guard.setGuardians(oldRoot, guardians);
    }

    function testRecoveryDeadMansSwitch() public {
        (address oldRoot, address newRoot,,,) = _setupRecovery();
        uint64 chainId = uint64(block.chainid);

        bytes32 digest = _recoveryDigest(oldRoot, newRoot, chainId, 0);
        (uint8 v0, bytes32 r0, bytes32 s0) = vm.sign(GUARDIAN_KEY_0, digest);
        guard.initiateRecovery(oldRoot, newRoot, v0, r0, s0);

        assertEq(guard.recoveryApprovals(oldRoot), 1);
        assertEq(guard.pendingNewRoot(oldRoot), newRoot);
        assertEq(uint256(guard.identityState(oldRoot)), uint256(IdentityStore.IdentityState.RecoveryPending));

        (uint8 v1, bytes32 r1, bytes32 s1) = vm.sign(GUARDIAN_KEY_1, digest);
        guard.supportRecovery(oldRoot, v1, r1, s1);

        assertEq(guard.recoveryApprovals(oldRoot), 2);
        assertTrue(guard.recoveryInitiatedAt(oldRoot) > 0, "timelock should have started");

        vm.expectRevert(IdentityStore.TimelockNotExpired.selector);
        guard.finalizeRecovery(oldRoot);

        vm.warp(block.timestamp + 48 hours);
        guard.finalizeRecovery(oldRoot);

        assertFalse(guard.authorizedProvers(oldRoot), "old root should be deregistered");
        assertTrue(guard.authorizedProvers(newRoot), "new root should be registered");
        assertEq(guard.nonces(newRoot), 0);
        assertEq(guard.delegationNonces(newRoot), 0);

        address[] memory newGuardians = guard.getGuardians(newRoot);
        assertEq(newGuardians.length, 3);
        assertEq(guard.pendingNewRoot(oldRoot), address(0));
        assertEq(guard.recoveryApprovals(oldRoot), 0);
        assertEq(guard.recoveryNonces(oldRoot), 1);

        assertEq(uint256(guard.identityState(oldRoot)), uint256(IdentityStore.IdentityState.Active));
        assertEq(uint256(guard.identityState(newRoot)), uint256(IdentityStore.IdentityState.Active));
    }

    function testRecoveryBetrayalCancellation() public {
        (address oldRoot, address newRoot,,,) = _setupRecovery();
        uint64 chainId = uint64(block.chainid);

        bytes32 digest = _recoveryDigest(oldRoot, newRoot, chainId, 0);
        (uint8 v0, bytes32 r0, bytes32 s0) = vm.sign(GUARDIAN_KEY_0, digest);
        guard.initiateRecovery(oldRoot, newRoot, v0, r0, s0);

        (uint8 v1, bytes32 r1, bytes32 s1) = vm.sign(GUARDIAN_KEY_1, digest);
        guard.supportRecovery(oldRoot, v1, r1, s1);

        assertTrue(guard.recoveryInitiatedAt(oldRoot) > 0, "timelock started");
        assertEq(uint256(guard.identityState(oldRoot)), uint256(IdentityStore.IdentityState.RecoveryPending));

        vm.prank(oldRoot);
        guard.cancelRecovery(oldRoot);

        assertEq(guard.pendingNewRoot(oldRoot), address(0));
        assertEq(guard.recoveryApprovals(oldRoot), 0);
        assertEq(guard.recoveryInitiatedAt(oldRoot), 0);
        assertTrue(guard.authorizedProvers(oldRoot));
        assertEq(uint256(guard.identityState(oldRoot)), uint256(IdentityStore.IdentityState.Active));

        vm.expectRevert(IdentityStore.NoPendingRecovery.selector);
        guard.finalizeRecovery(oldRoot);
    }

    function testRecoveryRevertNonGuardian() public {
        (address oldRoot, address newRoot,,,) = _setupRecovery();
        uint64 chainId = uint64(block.chainid);

        uint256 rogueKey = 0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef;
        bytes32 digest = _recoveryDigest(oldRoot, newRoot, chainId, 0);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(rogueKey, digest);

        vm.expectRevert(IdentityStore.NotAGuardian.selector);
        guard.initiateRecovery(oldRoot, newRoot, v, r, s);
    }

    function testRecoveryRevertThresholdNotMet() public {
        (address oldRoot, address newRoot,,,) = _setupRecovery();
        uint64 chainId = uint64(block.chainid);

        bytes32 digest = _recoveryDigest(oldRoot, newRoot, chainId, 0);
        (uint8 v0, bytes32 r0, bytes32 s0) = vm.sign(GUARDIAN_KEY_0, digest);
        guard.initiateRecovery(oldRoot, newRoot, v0, r0, s0);

        vm.warp(block.timestamp + 48 hours);

        vm.expectRevert(IdentityStore.ThresholdNotMet.selector);
        guard.finalizeRecovery(oldRoot);
    }

    function testRecoveryRevertCancelNotRoot() public {
        (address oldRoot, address newRoot,,,) = _setupRecovery();
        uint64 chainId = uint64(block.chainid);

        bytes32 digest = _recoveryDigest(oldRoot, newRoot, chainId, 0);
        (uint8 v0, bytes32 r0, bytes32 s0) = vm.sign(GUARDIAN_KEY_0, digest);
        guard.initiateRecovery(oldRoot, newRoot, v0, r0, s0);

        vm.prank(address(0xDEAD));
        vm.expectRevert(IdentityStore.OnlyOldRootCanCancel.selector);
        guard.cancelRecovery(oldRoot);
    }

    function testRecoveryRevertDuplicateApproval() public {
        (address oldRoot, address newRoot,,,) = _setupRecovery();
        uint64 chainId = uint64(block.chainid);

        bytes32 digest = _recoveryDigest(oldRoot, newRoot, chainId, 0);
        (uint8 v0, bytes32 r0, bytes32 s0) = vm.sign(GUARDIAN_KEY_0, digest);
        guard.initiateRecovery(oldRoot, newRoot, v0, r0, s0);

        vm.expectRevert(IdentityStore.AlreadyApproved.selector);
        guard.supportRecovery(oldRoot, v0, r0, s0);
    }

    function testRecoveryNonceAntiReplay() public {
        (address oldRoot, address newRoot,,,) = _setupRecovery();
        uint64 chainId = uint64(block.chainid);

        bytes32 digest = _recoveryDigest(oldRoot, newRoot, chainId, 0);
        (uint8 v0, bytes32 r0, bytes32 s0) = vm.sign(GUARDIAN_KEY_0, digest);
        guard.initiateRecovery(oldRoot, newRoot, v0, r0, s0);
        (uint8 v1, bytes32 r1, bytes32 s1) = vm.sign(GUARDIAN_KEY_1, digest);
        guard.supportRecovery(oldRoot, v1, r1, s1);
        vm.warp(block.timestamp + 48 hours);
        guard.finalizeRecovery(oldRoot);

        assertEq(guard.recoveryNonces(oldRoot), 1);
    }

    // =======================================================================
    // Session Key & Execution Gateway tests
    // =======================================================================

    uint256 constant SESSION_KEY = 0x2a871d0798f97d79848a013d4936a73bf4cc922c825d33c1cf7073dff6d409c6;

    function _buildExecuteCall(
        uint256 sessionPrivKey,
        address sessionAddr,
        bytes4 scope,
        address target,
        uint128 sessionCap,
        uint64 sessionExp,
        uint128 intentVal,
        uint64 intentExp,
        uint64 intentNonce,
        bytes memory callData
    ) internal view returns (
        ExecutionGateway.SessionParams memory sess,
        IdentityStore.IntentParams memory intent
    ) {
        uint64 chainId = uint64(block.chainid);

        bytes32 sessDigest = _sessionDigest(sessionAddr, actionSigner, scope, target, sessionCap, sessionExp, chainId);
        (uint8 sV, bytes32 sR, bytes32 sS) = vm.sign(ACTION_KEY, sessDigest);

        sess = ExecutionGateway.SessionParams({
            session: sessionAddr,
            parent: actionSigner,
            scope: scope,
            target: target,
            maxValue: sessionCap,
            expiration: sessionExp,
            chainId: chainId,
            v: sV,
            r: sR,
            s: sS
        });

        bytes32 dataHash = keccak256(callData);
        bytes32 intentDigest = _digest(target, scope, address(0), address(0), dataHash, intentVal, intentExp, chainId, intentNonce);
        (uint8 iV, bytes32 iR, bytes32 iS) = vm.sign(sessionPrivKey, intentDigest);

        intent = IdentityStore.IntentParams({
            targetContract: target,
            functionSig: scope,
            recipient: address(0),
            assetAddress: address(0),
            callDataHash: dataHash,
            maxValue: intentVal,
            expiration: intentExp,
            chainId: chainId,
            nonce: intentNonce,
            sessionEpoch: 0,
            gasLimit: 0,
            maxFeePerGas: 0,
            maxPriorityFeePerGas: 0,
            requiredClaim: bytes32(0),
            claimProofHash: bytes32(0),
            paymasterMode: 0,
            paymaster: address(0),
            v: iV,
            r: iR,
            s: iS
        });
    }

    // -----------------------------------------------------------------------
    // 21. Execute happy path
    // -----------------------------------------------------------------------
    function testExecuteHappyPath() public {
        DummyTarget dummy = new DummyTarget();
        address sessionAddr = vm.addr(SESSION_KEY);
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(42));

        (ExecutionGateway.SessionParams memory sess, IdentityStore.IntentParams memory intent) =
            _buildExecuteCall(SESSION_KEY, sessionAddr, scope, address(dummy), 1 ether, exp, 0.5 ether, exp, 0, cd);

        guard.execute{value: 0.1 ether}(sess, intent, address(dummy), cd);

        assertEq(dummy.lastValue(), 42);
        assertTrue(guard.usedSessionKeys(sessionAddr), "session key should be burned");
    }

    // -----------------------------------------------------------------------
    // 22. One-Time Use — session key fails on second use
    // -----------------------------------------------------------------------
    function testExecuteRevertOneTimeUse() public {
        DummyTarget dummy = new DummyTarget();
        address sessionAddr = vm.addr(SESSION_KEY);
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(1));

        (ExecutionGateway.SessionParams memory sess1, IdentityStore.IntentParams memory intent1) =
            _buildExecuteCall(SESSION_KEY, sessionAddr, scope, address(dummy), 1 ether, exp, 0.5 ether, exp, 0, cd);

        guard.execute(sess1, intent1, address(dummy), cd);

        bytes memory cd2 = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(2));
        (ExecutionGateway.SessionParams memory sess2, IdentityStore.IntentParams memory intent2) =
            _buildExecuteCall(SESSION_KEY, sessionAddr, scope, address(dummy), 1 ether, exp, 0.5 ether, exp, 1, cd2);

        vm.expectRevert(ExecutionGateway.SessionKeyAlreadyUsed.selector);
        guard.execute(sess2, intent2, address(dummy), cd2);
    }

    // -----------------------------------------------------------------------
    // 23. Selector mismatch
    // -----------------------------------------------------------------------
    function testExecuteRevertSelectorMismatch() public {
        DummyTarget dummy = new DummyTarget();
        address sessionAddr = vm.addr(SESSION_KEY);
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(1));

        (ExecutionGateway.SessionParams memory sess, IdentityStore.IntentParams memory intent) =
            _buildExecuteCall(SESSION_KEY, sessionAddr, scope, address(dummy), 1 ether, exp, 0.5 ether, exp, 0, cd);

        vm.expectRevert(ExecutionGateway.SelectorMismatch.selector);
        guard.execute(sess, intent, address(dummy),
            abi.encodeWithSelector(DummyTarget.otherFunction.selector, uint256(1)));
    }

    // -----------------------------------------------------------------------
    // 24. Target mismatch
    // -----------------------------------------------------------------------
    function testExecuteRevertTargetMismatch() public {
        DummyTarget dummy = new DummyTarget();
        DummyTarget other = new DummyTarget();
        address sessionAddr = vm.addr(SESSION_KEY);
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(1));

        (ExecutionGateway.SessionParams memory sess, IdentityStore.IntentParams memory intent) =
            _buildExecuteCall(SESSION_KEY, sessionAddr, scope, address(dummy), 1 ether, exp, 0.5 ether, exp, 0, cd);

        vm.expectRevert(ExecutionGateway.CallTargetMismatch.selector);
        guard.execute(sess, intent, address(other), cd);
    }

    // -----------------------------------------------------------------------
    // 25. Session expired
    // -----------------------------------------------------------------------
    function testExecuteRevertSessionExpired() public {
        DummyTarget dummy = new DummyTarget();
        address sessionAddr = vm.addr(SESSION_KEY);
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 expiredTs = uint64(block.timestamp - 1);
        uint64 futureTs = uint64(block.timestamp + 1 hours);
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(1));

        (ExecutionGateway.SessionParams memory sess, IdentityStore.IntentParams memory intent) =
            _buildExecuteCall(SESSION_KEY, sessionAddr, scope, address(dummy), 1 ether, expiredTs, 0.5 ether, futureTs, 0, cd);

        vm.expectRevert(ExecutionGateway.SessionExpired.selector);
        guard.execute(sess, intent, address(dummy), cd);
    }

    // -----------------------------------------------------------------------
    // 26. Unauthorized parent
    // -----------------------------------------------------------------------
    function testExecuteRevertUnauthorizedParent() public {
        DummyTarget dummy = new DummyTarget();
        uint256 rogueParent = 0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef;
        address rogueAddr = vm.addr(rogueParent);
        address sessionAddr = vm.addr(SESSION_KEY);
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        uint64 chainId = uint64(block.chainid);
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(1));

        bytes32 sessDigest = _sessionDigest(sessionAddr, rogueAddr, scope, address(dummy), 1 ether, exp, chainId);
        (uint8 sV, bytes32 sR, bytes32 sS) = vm.sign(rogueParent, sessDigest);

        ExecutionGateway.SessionParams memory sess = ExecutionGateway.SessionParams({
            session: sessionAddr, parent: rogueAddr, scope: scope, target: address(dummy),
            maxValue: 1 ether, expiration: exp, chainId: chainId, v: sV, r: sR, s: sS
        });

        bytes32 dataHash = keccak256(cd);
        bytes32 intentDigest = _digest(address(dummy), scope, address(0), address(0), dataHash, 0.5 ether, exp, chainId, 0);
        (uint8 iV, bytes32 iR, bytes32 iS) = vm.sign(SESSION_KEY, intentDigest);

        IdentityStore.IntentParams memory intent = IdentityStore.IntentParams({
            targetContract: address(dummy), functionSig: scope,
            recipient: address(0), assetAddress: address(0), callDataHash: dataHash,
            maxValue: 0.5 ether, expiration: exp, chainId: chainId, nonce: 0,
            sessionEpoch: 0, gasLimit: 0, maxFeePerGas: 0, maxPriorityFeePerGas: 0, requiredClaim: bytes32(0),
            claimProofHash: bytes32(0), paymasterMode: 0, paymaster: address(0),
            v: iV, r: iR, s: iS
        });

        vm.expectRevert(ExecutionGateway.SessionParentNotAuthorized.selector);
        guard.execute(sess, intent, address(dummy), cd);
    }

    // -----------------------------------------------------------------------
    // 27. Intent exceeds session cap
    // -----------------------------------------------------------------------
    function testExecuteRevertExceedsSessionCap() public {
        DummyTarget dummy = new DummyTarget();
        address sessionAddr = vm.addr(SESSION_KEY);
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(1));

        (ExecutionGateway.SessionParams memory sess, IdentityStore.IntentParams memory intent) =
            _buildExecuteCall(SESSION_KEY, sessionAddr, scope, address(dummy), 0.1 ether, exp, 1 ether, exp, 0, cd);

        vm.expectRevert(ExecutionGateway.IntentExceedsSessionCap.selector);
        guard.execute(sess, intent, address(dummy), cd);
    }

    // -----------------------------------------------------------------------
    // 28. Malicious calldata mutation — changing a byte causes revert
    // -----------------------------------------------------------------------
    function testMaliciousCalldataMutation() public {
        DummyTarget dummy = new DummyTarget();
        address sessionAddr = vm.addr(SESSION_KEY);
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);

        bytes memory originalCd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(42));

        (ExecutionGateway.SessionParams memory sess, IdentityStore.IntentParams memory intent) =
            _buildExecuteCall(SESSION_KEY, sessionAddr, scope, address(dummy), 1 ether, exp, 0.5 ether, exp, 0, originalCd);

        bytes memory mutatedCd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x5678), uint256(42));

        vm.expectRevert(ExecutionGateway.CalldataHashMismatch.selector);
        guard.execute(sess, intent, address(dummy), mutatedCd);
    }

    // -----------------------------------------------------------------------
    // 29. Execution blocked during recovery
    // -----------------------------------------------------------------------
    function testExecutionBlockedDuringRecovery() public {
        (address oldRoot, address newRoot,,,) = _setupRecovery();

        guard.authorizeKey(actionSigner);

        uint64 chainId = uint64(block.chainid);
        bytes32 recoveryDigestVal = _recoveryDigest(oldRoot, newRoot, chainId, 0);
        (uint8 v0, bytes32 r0, bytes32 s0) = vm.sign(GUARDIAN_KEY_0, recoveryDigestVal);
        guard.initiateRecovery(oldRoot, newRoot, v0, r0, s0);

        assertEq(uint256(guard.identityState(oldRoot)), uint256(IdentityStore.IdentityState.RecoveryPending));

        DummyTarget dummy = new DummyTarget();
        address sessionAddr = vm.addr(SESSION_KEY);
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(42));

        guard.authorizeKey(oldRoot);

        bytes32 sessDigest = _sessionDigest(sessionAddr, oldRoot, scope, address(dummy), 1 ether, exp, chainId);
        (uint8 sV, bytes32 sR, bytes32 sS) = vm.sign(OLD_ROOT_KEY, sessDigest);

        ExecutionGateway.SessionParams memory sess = ExecutionGateway.SessionParams({
            session: sessionAddr, parent: oldRoot, scope: scope, target: address(dummy),
            maxValue: 1 ether, expiration: exp, chainId: chainId, v: sV, r: sR, s: sS
        });

        bytes32 dataHash = keccak256(cd);
        bytes32 intentDigest = _digest(address(dummy), scope, address(0), address(0), dataHash, 0.5 ether, exp, chainId, 0);
        (uint8 iV, bytes32 iR, bytes32 iS) = vm.sign(SESSION_KEY, intentDigest);

        IdentityStore.IntentParams memory intent = IdentityStore.IntentParams({
            targetContract: address(dummy), functionSig: scope,
            recipient: address(0), assetAddress: address(0), callDataHash: dataHash,
            maxValue: 0.5 ether, expiration: exp, chainId: chainId, nonce: 0,
            sessionEpoch: 0, gasLimit: 0, maxFeePerGas: 0, maxPriorityFeePerGas: 0, requiredClaim: bytes32(0),
            claimProofHash: bytes32(0), paymasterMode: 0, paymaster: address(0),
            v: iV, r: iR, s: iS
        });

        vm.expectRevert(ExecutionGateway.IdentityNotActive.selector);
        guard.execute(sess, intent, address(dummy), cd);
    }

    // =======================================================================
    // v1.1 Tests
    // =======================================================================

    // -----------------------------------------------------------------------
    // 30. Domain version verification
    // -----------------------------------------------------------------------
    function testVersionMismatchFails() public view {
        bytes32 expected = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256("HuntKey"),
                keccak256("1"),
                block.chainid,
                address(guard)
            )
        );
        assertEq(guard.DOMAIN_SEPARATOR(), expected, "domain separator must use version 1");

        bytes32 wrongVersion = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256("HuntKey"),
                keccak256("2"),
                block.chainid,
                address(guard)
            )
        );
        assertTrue(guard.DOMAIN_SEPARATOR() != wrongVersion, "version 2 must differ");
    }

    // -----------------------------------------------------------------------
    // 31. Event emission on execute
    // -----------------------------------------------------------------------
    function testEventEmissionOnExecute() public {
        DummyTarget dummy = new DummyTarget();
        address sessionAddr = vm.addr(SESSION_KEY);
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(99));

        (ExecutionGateway.SessionParams memory sess, IdentityStore.IntentParams memory intent) =
            _buildExecuteCall(SESSION_KEY, sessionAddr, scope, address(dummy), 1 ether, exp, 0.5 ether, exp, 0, cd);

        vm.expectEmit(true, true, false, true);
        emit ExecutionGateway.Executed(sessionAddr, address(dummy), scope, 0, true);

        vm.expectEmit(true, true, false, true);
        emit IdentityStore.IntentExecuted(actionSigner, sessionAddr, scope);

        guard.execute(sess, intent, address(dummy), cd);

        assertEq(dummy.lastValue(), 99);
    }
}

// =======================================================================
// HuntKeyAccount (ERC-4337) Tests
// =======================================================================

contract HuntKeyAccountTest is Test {
    HuntKeyAccount account;
    DummyTarget dummy;

    uint256 constant ACTION_KEY = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;
    address actionSigner;

    uint256 constant SESSION_KEY = 0x2a871d0798f97d79848a013d4936a73bf4cc922c825d33c1cf7073dff6d409c6;

    address constant ENTRY_POINT = address(0xE1E1E1);

    function setUp() public {
        account = new HuntKeyAccount();
        dummy = new DummyTarget();
        actionSigner = vm.addr(ACTION_KEY);

        account.authorizeKey(actionSigner);
        account.setEntryPoint(ENTRY_POINT);

        // Fund the account for pre-funding
        vm.deal(address(account), 10 ether);
    }

    /// @dev Pack ERC-4337 validationData: authorizer(160) | validUntil(48) | validAfter(48).
    function _packValidation(bool sigFailed, uint48 validUntil, uint48 validAfter) internal pure returns (uint256) {
        return (sigFailed ? 1 : 0) | (uint256(validUntil) << 160) | (uint256(validAfter) << 208);
    }

    function _intentDigest(
        address targetContract,
        bytes4 functionSig,
        address recipient,
        address assetAddress,
        bytes32 dataHash,
        uint128 maxValue,
        uint64 expiration,
        uint64 intentChainId,
        uint64 nonce,
        uint64 intentSessionEpoch,
        uint64 gasLimit,
        uint128 maxFeePerGas,
        uint128 maxPriorityFeePerGas,
        bytes32 requiredClaim
    ) internal view returns (bytes32) {
        bytes memory first = abi.encode(
            account.INTENT_TYPEHASH(), targetContract, functionSig,
            recipient, assetAddress, dataHash, maxValue
        );
        bytes memory second = abi.encode(
            expiration, intentChainId, nonce,
            intentSessionEpoch, gasLimit, maxFeePerGas,
            maxPriorityFeePerGas, requiredClaim
        );
        bytes memory third = abi.encode(
            bytes32(0), uint8(0), address(0)
        );
        bytes32 structHash = keccak256(bytes.concat(first, second, third));
        return keccak256(
            abi.encodePacked("\x19\x01", account.DOMAIN_SEPARATOR(), structHash)
        );
    }

    function _sessionDigest(
        address session,
        address parent,
        bytes4 scope,
        address target,
        uint128 maxValue,
        uint64 expiration,
        uint64 chainId
    ) internal view returns (bytes32) {
        bytes32 structHash = keccak256(
            abi.encode(
                account.SESSION_TYPEHASH(),
                session,
                parent,
                scope,
                target,
                maxValue,
                expiration,
                chainId
            )
        );
        return keccak256(
            abi.encodePacked("\x19\x01", account.DOMAIN_SEPARATOR(), structHash)
        );
    }

    function _buildUserOpSignature(
        bytes4 scope,
        address target,
        uint128 sessionCap,
        uint64 sessionExp,
        uint128 intentVal,
        uint64 intentExp,
        bytes memory callData,
        uint64 gasLimit,
        uint128 maxFeePerGas,
        bytes32 requiredClaim
    ) internal view returns (bytes memory) {
        address sessionAddr = vm.addr(SESSION_KEY);
        uint64 chainId = uint64(block.chainid);

        // Session cert signed by action key
        bytes32 sessDigest = _sessionDigest(sessionAddr, actionSigner, scope, target, sessionCap, sessionExp, chainId);
        (uint8 sV, bytes32 sR, bytes32 sS) = vm.sign(ACTION_KEY, sessDigest);

        ExecutionGateway.SessionParams memory sess = ExecutionGateway.SessionParams({
            session: sessionAddr,
            parent: actionSigner,
            scope: scope,
            target: target,
            maxValue: sessionCap,
            expiration: sessionExp,
            chainId: chainId,
            v: sV,
            r: sR,
            s: sS
        });

        // Intent signed by session key
        bytes32 dataHash = keccak256(callData);
        bytes32 iDigest = _intentDigest(target, scope, address(0), address(0), dataHash, intentVal, intentExp, chainId, 0, 0, gasLimit, maxFeePerGas, 0, requiredClaim);
        (uint8 iV, bytes32 iR, bytes32 iS) = vm.sign(SESSION_KEY, iDigest);

        IdentityStore.IntentParams memory intent = IdentityStore.IntentParams({
            targetContract: target,
            functionSig: scope,
            recipient: address(0),
            assetAddress: address(0),
            callDataHash: dataHash,
            maxValue: intentVal,
            expiration: intentExp,
            chainId: chainId,
            nonce: 0,
            sessionEpoch: 0,
            gasLimit: gasLimit,
            maxFeePerGas: maxFeePerGas,
            maxPriorityFeePerGas: 0,
            requiredClaim: requiredClaim,
            claimProofHash: bytes32(0),
            paymasterMode: 0,
            paymaster: address(0),
            v: iV,
            r: iR,
            s: iS
        });

        return abi.encode(sess, intent);
    }

    function _emptyUserOp(bytes memory signature) internal pure returns (PackedUserOperation memory) {
        return PackedUserOperation({
            sender: address(0),
            nonce: 0,
            initCode: "",
            callData: "",
            accountGasLimits: bytes32(0),
            preVerificationGas: 0,
            gasFees: bytes32(0),
            paymasterAndData: "",
            signature: signature
        });
    }

    // -----------------------------------------------------------------------
    // 32. validateUserOp happy path
    // -----------------------------------------------------------------------
    function testValidateUserOpHappyPath() public {
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(42));

        bytes memory sig = _buildUserOpSignature(scope, address(dummy), 1 ether, exp, 0.5 ether, exp, cd, 0, 0, bytes32(0));
        PackedUserOperation memory userOp = _emptyUserOp(sig);
        userOp.sender = address(account);

        vm.prank(ENTRY_POINT);
        uint256 result = account.validateUserOp(userOp, keccak256("test-hash"), 0);
        assertEq(result, _packValidation(false, uint48(exp), 0), "validation should succeed with packed validUntil");
    }

    // -----------------------------------------------------------------------
    // 33. validateUserOp blocked during RecoveryPending
    // -----------------------------------------------------------------------
    function testValidateUserOpBlockedDuringRecovery() public {
        // Set up recovery for actionSigner
        uint256 guardianKey = 0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a;
        address guardian = vm.addr(guardianKey);
        address newRoot = address(0xBBBB);

        account.registerProver(actionSigner);
        address[] memory guardians = new address[](3);
        guardians[0] = guardian;
        guardians[1] = vm.addr(0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba);
        guardians[2] = vm.addr(0x92db14e403b83dfe3df233f83dfa3a0d7096f21ca9b0d6d6b8d88b2b4ec1564e);
        account.setGuardians(actionSigner, guardians);

        // Initiate recovery
        bytes32 recovDigest = keccak256(
            abi.encodePacked(
                "\x19\x01",
                account.DOMAIN_SEPARATOR(),
                keccak256(abi.encode(account.RECOVERY_TYPEHASH(), actionSigner, newRoot, uint64(block.chainid), uint64(0)))
            )
        );
        (uint8 rv, bytes32 rr, bytes32 rs) = vm.sign(guardianKey, recovDigest);
        account.initiateRecovery(actionSigner, newRoot, rv, rr, rs);

        // Now try validateUserOp — should revert
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(42));

        bytes memory sig = _buildUserOpSignature(scope, address(dummy), 1 ether, exp, 0.5 ether, exp, cd, 0, 0, bytes32(0));
        PackedUserOperation memory userOp = _emptyUserOp(sig);

        vm.prank(ENTRY_POINT);
        vm.expectRevert(HuntKeyAccount.RecoveryBlocksUserOp.selector);
        account.validateUserOp(userOp, keccak256("test"), 0);
    }

    // -----------------------------------------------------------------------
    // 34. validateUserOp rejects non-EntryPoint caller
    // -----------------------------------------------------------------------
    function testValidateUserOpRejectsNonEntryPoint() public {
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(42));

        bytes memory sig = _buildUserOpSignature(scope, address(dummy), 1 ether, exp, 0.5 ether, exp, cd, 0, 0, bytes32(0));
        PackedUserOperation memory userOp = _emptyUserOp(sig);

        vm.expectRevert(HuntKeyAccount.OnlyEntryPoint.selector);
        account.validateUserOp(userOp, keccak256("test"), 0);
    }

    // -----------------------------------------------------------------------
    // 35. Claim check — requiredClaim set, account holds claim
    // -----------------------------------------------------------------------
    function testClaimCheckSatisfied() public {
        bytes32 claim = keccak256("KYC_VERIFIED");
        account.setClaim(actionSigner, claim, true);

        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(42));

        bytes memory sig = _buildUserOpSignature(scope, address(dummy), 1 ether, exp, 0.5 ether, exp, cd, 0, 0, claim);
        PackedUserOperation memory userOp = _emptyUserOp(sig);
        userOp.sender = address(account);

        vm.prank(ENTRY_POINT);
        uint256 result = account.validateUserOp(userOp, keccak256("test"), 0);
        assertEq(result, _packValidation(false, uint48(exp), 0), "validation with claim should succeed");
    }

    // -----------------------------------------------------------------------
    // 36. Claim check — requiredClaim set, account does NOT hold claim
    // -----------------------------------------------------------------------
    function testClaimCheckFails() public {
        bytes32 claim = keccak256("KYC_VERIFIED");
        // Do NOT set the claim

        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(42));

        bytes memory sig = _buildUserOpSignature(scope, address(dummy), 1 ether, exp, 0.5 ether, exp, cd, 0, 0, claim);
        PackedUserOperation memory userOp = _emptyUserOp(sig);

        vm.prank(ENTRY_POINT);
        vm.expectRevert(HuntKeyAccount.ClaimNotSatisfied.selector);
        account.validateUserOp(userOp, keccak256("test"), 0);
    }

    // -----------------------------------------------------------------------
    // 37. validateUserOp with gasLimit and maxFeePerGas in intent
    // -----------------------------------------------------------------------
    function testValidateUserOpWithGasParams() public {
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(42));

        // Non-zero gas params
        bytes memory sig = _buildUserOpSignature(scope, address(dummy), 1 ether, exp, 0.5 ether, exp, cd, 100000, 50 gwei, bytes32(0));
        PackedUserOperation memory userOp = _emptyUserOp(sig);
        userOp.sender = address(account);

        vm.prank(ENTRY_POINT);
        uint256 result = account.validateUserOp(userOp, keccak256("test"), 0);
        assertEq(result, _packValidation(false, uint48(exp), 0), "validation with gas params should succeed");
    }

    // -----------------------------------------------------------------------
    // 38. Multicall execution
    // -----------------------------------------------------------------------
    function testMulticallExecution() public {
        DummyTarget dummy2 = new DummyTarget();
        // Both calls go to the same target (scope enforcement)
        address sessionAddr = vm.addr(SESSION_KEY);
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        uint64 chainId = uint64(block.chainid);

        // Build calls array
        HuntKeyAccount.Call[] memory calls = new HuntKeyAccount.Call[](2);
        calls[0] = HuntKeyAccount.Call({
            target: address(dummy),
            value: 0,
            data: abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(42))
        });
        calls[1] = HuntKeyAccount.Call({
            target: address(dummy),
            value: 0,
            data: abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x5678), uint256(99))
        });

        // Compute multicall hash
        bytes32 multicallHash = keccak256(abi.encode(calls));

        // Build session cert
        bytes32 sessDigest = _sessionDigest(sessionAddr, actionSigner, scope, address(dummy), 1 ether, exp, chainId);
        (uint8 sV, bytes32 sR, bytes32 sS) = vm.sign(ACTION_KEY, sessDigest);

        ExecutionGateway.SessionParams memory sess = ExecutionGateway.SessionParams({
            session: sessionAddr,
            parent: actionSigner,
            scope: scope,
            target: address(dummy),
            maxValue: 1 ether,
            expiration: exp,
            chainId: chainId,
            v: sV,
            r: sR,
            s: sS
        });

        // Build intent with multicall hash
        bytes32 iDigest = _intentDigest(address(dummy), scope, address(0), address(0), multicallHash, 1 ether, exp, chainId, 0, 0, 0, 0, 0, bytes32(0));
        (uint8 iV, bytes32 iR, bytes32 iS) = vm.sign(SESSION_KEY, iDigest);

        IdentityStore.IntentParams memory intent = IdentityStore.IntentParams({
            targetContract: address(dummy),
            functionSig: scope,
            recipient: address(0),
            assetAddress: address(0),
            callDataHash: multicallHash,
            maxValue: 1 ether,
            expiration: exp,
            chainId: chainId,
            nonce: 0,
            sessionEpoch: 0,
            gasLimit: 0,
            maxFeePerGas: 0,
            maxPriorityFeePerGas: 0,
            requiredClaim: bytes32(0),
            claimProofHash: bytes32(0),
            paymasterMode: 0,
            paymaster: address(0),
            v: iV,
            r: iR,
            s: iS
        });

        account.executeMulticall(sess, intent, calls);

        // Last call sets the values
        assertEq(dummy.lastValue(), 99);
        assertEq(dummy.lastRecipient(), address(0x5678));
        assertTrue(account.usedSessionKeys(sessionAddr), "session key should be burned");
    }

    // -----------------------------------------------------------------------
    // 39. Multicall revert on calldata hash mismatch
    // -----------------------------------------------------------------------
    function testMulticallRevertCalldataHashMismatch() public {
        address sessionAddr = vm.addr(SESSION_KEY);
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        uint64 chainId = uint64(block.chainid);

        HuntKeyAccount.Call[] memory calls = new HuntKeyAccount.Call[](1);
        calls[0] = HuntKeyAccount.Call({
            target: address(dummy),
            value: 0,
            data: abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(42))
        });

        // Sign with wrong hash (hash of different calls)
        bytes32 wrongHash = keccak256("wrong");

        bytes32 sessDigest = _sessionDigest(sessionAddr, actionSigner, scope, address(dummy), 1 ether, exp, chainId);
        (uint8 sV, bytes32 sR, bytes32 sS) = vm.sign(ACTION_KEY, sessDigest);

        ExecutionGateway.SessionParams memory sess = ExecutionGateway.SessionParams({
            session: sessionAddr, parent: actionSigner, scope: scope, target: address(dummy),
            maxValue: 1 ether, expiration: exp, chainId: chainId, v: sV, r: sR, s: sS
        });

        bytes32 iDigest = _intentDigest(address(dummy), scope, address(0), address(0), wrongHash, 1 ether, exp, chainId, 0, 0, 0, 0, 0, bytes32(0));
        (uint8 iV, bytes32 iR, bytes32 iS) = vm.sign(SESSION_KEY, iDigest);

        IdentityStore.IntentParams memory intent = IdentityStore.IntentParams({
            targetContract: address(dummy), functionSig: scope,
            recipient: address(0), assetAddress: address(0), callDataHash: wrongHash,
            maxValue: 1 ether, expiration: exp, chainId: chainId, nonce: 0,
            sessionEpoch: 0, gasLimit: 0, maxFeePerGas: 0, maxPriorityFeePerGas: 0, requiredClaim: bytes32(0),
            claimProofHash: bytes32(0), paymasterMode: 0, paymaster: address(0),
            v: iV, r: iR, s: iS
        });

        vm.expectRevert(ExecutionGateway.CalldataHashMismatch.selector);
        account.executeMulticall(sess, intent, calls);
    }

    // -----------------------------------------------------------------------
    // 40. validateUserOp pre-funds EntryPoint
    // -----------------------------------------------------------------------
    function testValidateUserOpPreFundsEntryPoint() public {
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(42));

        bytes memory sig = _buildUserOpSignature(scope, address(dummy), 1 ether, exp, 0.5 ether, exp, cd, 0, 0, bytes32(0));
        PackedUserOperation memory userOp = _emptyUserOp(sig);
        userOp.sender = address(account);

        uint256 epBalanceBefore = ENTRY_POINT.balance;

        vm.prank(ENTRY_POINT);
        uint256 result = account.validateUserOp(userOp, keccak256("test"), 0.1 ether);
        assertEq(result, _packValidation(false, uint48(exp), 0));

        assertEq(ENTRY_POINT.balance, epBalanceBefore + 0.1 ether, "EntryPoint should be pre-funded");
    }

    // -----------------------------------------------------------------------
    // 41. Session epoch enforcement — mismatch reverts in validateUserOp
    // -----------------------------------------------------------------------
    function testValidateUserOpSessionEpochMismatch() public {
        // Increment session epoch for actionSigner
        account.cancelAllSessions(actionSigner);
        // sessionEpoch[actionSigner] is now 1, but intent has sessionEpoch=0

        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(42));

        bytes memory sig = _buildUserOpSignature(scope, address(dummy), 1 ether, exp, 0.5 ether, exp, cd, 0, 0, bytes32(0));
        PackedUserOperation memory userOp = _emptyUserOp(sig);
        userOp.sender = address(account);

        vm.prank(ENTRY_POINT);
        uint256 result = account.validateUserOp(userOp, keccak256("test"), 0);
        // Should fail with packed SIG_VALIDATION_FAILED
        assertEq(result, _packValidation(true, 0, 0), "epoch mismatch should return sig failure");
    }

    // -----------------------------------------------------------------------
    // 42. Recovery management allowed during RecoveryPending
    // -----------------------------------------------------------------------
    function testRecoveryManagementAllowedDuringRecovery() public {
        // Set up recovery state
        uint256 guardianKey = 0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a;
        address guardian = vm.addr(guardianKey);
        address newRoot = address(0xBBBB);

        account.registerProver(actionSigner);
        address[] memory guardians = new address[](3);
        guardians[0] = guardian;
        guardians[1] = vm.addr(0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba);
        guardians[2] = vm.addr(0x92db14e403b83dfe3df233f83dfa3a0d7096f21ca9b0d6d6b8d88b2b4ec1564e);
        account.setGuardians(actionSigner, guardians);

        // Initiate recovery
        bytes32 recovDigest = keccak256(
            abi.encodePacked(
                "\x19\x01",
                account.DOMAIN_SEPARATOR(),
                keccak256(abi.encode(account.RECOVERY_TYPEHASH(), actionSigner, newRoot, uint64(block.chainid), uint64(0)))
            )
        );
        (uint8 rv, bytes32 rr, bytes32 rs) = vm.sign(guardianKey, recovDigest);
        account.initiateRecovery(actionSigner, newRoot, rv, rr, rs);

        // Build a UserOp with cancelRecovery calldata — should be allowed
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(42));

        bytes memory sig = _buildUserOpSignature(scope, address(dummy), 1 ether, exp, 0.5 ether, exp, cd, 0, 0, bytes32(0));
        PackedUserOperation memory userOp = _emptyUserOp(sig);
        userOp.sender = address(account);
        // Set callData to cancelRecovery(address) selector
        userOp.callData = abi.encodeWithSelector(bytes4(keccak256("cancelRecovery(address)")), actionSigner);

        vm.prank(ENTRY_POINT);
        uint256 result = account.validateUserOp(userOp, keccak256("test"), 0);
        // Recovery management should succeed (packed with validUntil)
        assertEq(result, _packValidation(false, uint48(exp), 0), "recovery management should be allowed");
    }

    // -----------------------------------------------------------------------
    // 43. Non-recovery callData blocked during RecoveryPending
    // -----------------------------------------------------------------------
    function testNonRecoveryBlockedDuringRecovery() public {
        uint256 guardianKey = 0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a;
        address guardian = vm.addr(guardianKey);
        address newRoot = address(0xBBBB);

        account.registerProver(actionSigner);
        address[] memory guardians = new address[](3);
        guardians[0] = guardian;
        guardians[1] = vm.addr(0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba);
        guardians[2] = vm.addr(0x92db14e403b83dfe3df233f83dfa3a0d7096f21ca9b0d6d6b8d88b2b4ec1564e);
        account.setGuardians(actionSigner, guardians);

        bytes32 recovDigest = keccak256(
            abi.encodePacked(
                "\x19\x01",
                account.DOMAIN_SEPARATOR(),
                keccak256(abi.encode(account.RECOVERY_TYPEHASH(), actionSigner, newRoot, uint64(block.chainid), uint64(0)))
            )
        );
        (uint8 rv, bytes32 rr, bytes32 rs) = vm.sign(guardianKey, recovDigest);
        account.initiateRecovery(actionSigner, newRoot, rv, rr, rs);

        // Build a UserOp with non-recovery callData (e.g., execute)
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(42));

        bytes memory sig = _buildUserOpSignature(scope, address(dummy), 1 ether, exp, 0.5 ether, exp, cd, 0, 0, bytes32(0));
        PackedUserOperation memory userOp = _emptyUserOp(sig);
        // callData targets a non-recovery function
        userOp.callData = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(42));

        vm.prank(ENTRY_POINT);
        vm.expectRevert(HuntKeyAccount.RecoveryBlocksUserOp.selector);
        account.validateUserOp(userOp, keccak256("test"), 0);
    }

    // -----------------------------------------------------------------------
    // 44. validationData packing — verify packed format
    // -----------------------------------------------------------------------
    function testValidationDataPackedFormat() public {
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 7200); // 2 hours
        bytes memory cd = abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(42));

        bytes memory sig = _buildUserOpSignature(scope, address(dummy), 1 ether, exp, 0.5 ether, exp, cd, 0, 0, bytes32(0));
        PackedUserOperation memory userOp = _emptyUserOp(sig);
        userOp.sender = address(account);

        vm.prank(ENTRY_POINT);
        uint256 result = account.validateUserOp(userOp, keccak256("test"), 0);

        // Extract components from packed validationData
        address authorizer = address(uint160(result));
        uint48 validUntil = uint48(result >> 160);
        uint48 validAfter = uint48(result >> 208);

        assertEq(authorizer, address(0), "authorizer should be 0 for success");
        assertEq(validUntil, uint48(exp), "validUntil should match session expiration");
        assertEq(validAfter, 0, "validAfter should be 0");
    }

    // -----------------------------------------------------------------------
    // 45. Session epoch enforcement in executeMulticall
    // -----------------------------------------------------------------------
    function testMulticallSessionEpochMismatch() public {
        // Increment epoch
        account.cancelAllSessions(actionSigner);

        address sessionAddr = vm.addr(SESSION_KEY);
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        uint64 chainId = uint64(block.chainid);

        HuntKeyAccount.Call[] memory calls = new HuntKeyAccount.Call[](1);
        calls[0] = HuntKeyAccount.Call({
            target: address(dummy),
            value: 0,
            data: abi.encodeWithSelector(DummyTarget.doSomething.selector, address(0x1234), uint256(42))
        });

        bytes32 multicallHash = keccak256(abi.encode(calls));

        bytes32 sessDigest = _sessionDigest(sessionAddr, actionSigner, scope, address(dummy), 1 ether, exp, chainId);
        (uint8 sV, bytes32 sR, bytes32 sS) = vm.sign(ACTION_KEY, sessDigest);

        ExecutionGateway.SessionParams memory sess = ExecutionGateway.SessionParams({
            session: sessionAddr, parent: actionSigner, scope: scope, target: address(dummy),
            maxValue: 1 ether, expiration: exp, chainId: chainId, v: sV, r: sR, s: sS
        });

        // Intent with sessionEpoch=0 but storage is 1
        bytes32 iDigest = _intentDigest(address(dummy), scope, address(0), address(0), multicallHash, 1 ether, exp, chainId, 0, 0, 0, 0, 0, bytes32(0));
        (uint8 iV, bytes32 iR, bytes32 iS) = vm.sign(SESSION_KEY, iDigest);

        IdentityStore.IntentParams memory intent = IdentityStore.IntentParams({
            targetContract: address(dummy), functionSig: scope,
            recipient: address(0), assetAddress: address(0), callDataHash: multicallHash,
            maxValue: 1 ether, expiration: exp, chainId: chainId, nonce: 0,
            sessionEpoch: 0, gasLimit: 0, maxFeePerGas: 0, maxPriorityFeePerGas: 0, requiredClaim: bytes32(0),
            claimProofHash: bytes32(0), paymasterMode: 0, paymaster: address(0),
            v: iV, r: iR, s: iS
        });

        vm.expectRevert(IdentityStore.SessionEpochMismatch.selector);
        account.executeMulticall(sess, intent, calls);
    }

    // -----------------------------------------------------------------------
    // 46. Deposit management — addDeposit and withdrawDepositTo
    // -----------------------------------------------------------------------

    function testDepositManagement() public {
        // Deploy a mock EntryPoint that tracks deposits
        MockEntryPoint ep = new MockEntryPoint();
        account.setEntryPoint(address(ep));

        // addDeposit
        account.addDeposit{value: 1 ether}();
        assertEq(ep.deposits(address(account)), 1 ether);

        // getDeposit
        uint256 bal = account.getDeposit();
        assertEq(bal, 1 ether);

        // withdrawDepositTo
        address payable recipient = payable(address(0xF00D));
        account.withdrawDepositTo(recipient, 0.5 ether);
        assertEq(ep.deposits(address(account)), 0.5 ether);
        assertEq(recipient.balance, 0.5 ether);
    }

    // -----------------------------------------------------------------------
    // 47. withdrawDepositTo reverts for non-owner
    // -----------------------------------------------------------------------

    function testWithdrawDepositOnlyOwner() public {
        MockEntryPoint ep = new MockEntryPoint();
        account.setEntryPoint(address(ep));
        account.addDeposit{value: 1 ether}();

        vm.prank(address(0xBEEF));
        vm.expectRevert(IdentityStore.NotOwner.selector);
        account.withdrawDepositTo(payable(address(0xBEEF)), 0.5 ether);
    }

    // -----------------------------------------------------------------------
    // 48. getDeposit returns 0 when no EntryPoint set
    // -----------------------------------------------------------------------

    function testGetDepositNoEntryPoint() public {
        HuntKeyAccount fresh = new HuntKeyAccount();
        assertEq(fresh.getDeposit(), 0);
    }
}

/// @dev Mock EntryPoint for deposit management testing
contract MockEntryPoint {
    mapping(address => uint256) public deposits;

    function depositTo(address account) external payable {
        deposits[account] += msg.value;
    }

    function withdrawTo(address payable withdrawAddress, uint256 amount) external {
        // Find the sender's account (the HuntKeyAccount that called us)
        // In practice, the EntryPoint tracks msg.sender
        deposits[msg.sender] -= amount;
        (bool success,) = withdrawAddress.call{value: amount}("");
        require(success);
    }

    function balanceOf(address account) external view returns (uint256) {
        return deposits[account];
    }

    receive() external payable {}
}

// ==========================================================================
// v2.3 — ClaimVerifier Tests
// ==========================================================================

import {ClaimVerifier} from "../src/ClaimVerifier.sol";

contract ClaimVerifierTest is Test {
    ClaimVerifier verifier;
    address issuer;
    address alice = address(0xA11CE);

    function setUp() public {
        issuer = address(this);
        verifier = new ClaimVerifier(issuer);
    }

    function testRegisterAndHasClaim() public {
        bytes32 claimType = verifier.AGE_OVER_18();
        bytes32 commitment = keccak256(abi.encodePacked(alice, claimType, bytes32(uint256(42))));

        verifier.registerClaim(alice, claimType, commitment);
        assertTrue(verifier.hasClaim(alice, claimType));
    }

    function testVerifyClaimProof() public {
        bytes32 claimType = verifier.KYC_VERIFIED();
        bytes32 secret = bytes32(uint256(12345));
        bytes32 commitment = keccak256(abi.encodePacked(alice, claimType, secret));

        verifier.registerClaim(alice, claimType, commitment);

        bytes32 proofHash = verifier.verifyClaimProof(alice, claimType, secret);
        assertEq(proofHash, commitment);
        assertTrue(verifier.usedProofs(proofHash));
    }

    function testRevertInvalidProof() public {
        bytes32 claimType = verifier.AGE_OVER_18();
        bytes32 secret = bytes32(uint256(42));
        bytes32 commitment = keccak256(abi.encodePacked(alice, claimType, secret));

        verifier.registerClaim(alice, claimType, commitment);

        bytes32 wrongSecret = bytes32(uint256(999));
        vm.expectRevert(ClaimVerifier.InvalidProof.selector);
        verifier.verifyClaimProof(alice, claimType, wrongSecret);
    }

    function testRevertProofReplay() public {
        bytes32 claimType = verifier.DAO_MEMBER();
        bytes32 secret = bytes32(uint256(77));
        bytes32 commitment = keccak256(abi.encodePacked(alice, claimType, secret));

        verifier.registerClaim(alice, claimType, commitment);
        verifier.verifyClaimProof(alice, claimType, secret);

        vm.expectRevert(ClaimVerifier.ProofAlreadyUsed.selector);
        verifier.verifyClaimProof(alice, claimType, secret);
    }

    function testRevertClaimNotRegistered() public {
        bytes32 claimType = verifier.COUNTRY_ALLOWED();

        vm.expectRevert(ClaimVerifier.ClaimNotRegistered.selector);
        verifier.verifyClaimProof(alice, claimType, bytes32(uint256(1)));
    }

    function testRevokeClaim() public {
        bytes32 claimType = verifier.AGE_OVER_18();
        bytes32 commitment = keccak256(abi.encodePacked(alice, claimType, bytes32(uint256(1))));

        verifier.registerClaim(alice, claimType, commitment);
        assertTrue(verifier.hasClaim(alice, claimType));

        verifier.revokeClaim(alice, claimType);
        assertFalse(verifier.hasClaim(alice, claimType));
    }

    function testOnlyIssuerCanRegister() public {
        bytes32 claimType = verifier.AGE_OVER_18();

        vm.prank(address(0xBEEF));
        vm.expectRevert(ClaimVerifier.OnlyIssuer.selector);
        verifier.registerClaim(alice, claimType, bytes32(uint256(1)));
    }

    function testVerifyProofHashView() public {
        bytes32 claimType = verifier.KYC_VERIFIED();
        bytes32 secret = bytes32(uint256(42));
        bytes32 commitment = keccak256(abi.encodePacked(alice, claimType, secret));

        verifier.registerClaim(alice, claimType, commitment);

        assertTrue(verifier.verifyProofHash(alice, claimType, commitment));
        assertFalse(verifier.verifyProofHash(alice, claimType, bytes32(uint256(999))));
    }

    function testClaimTypeConstants() public view {
        assertEq(verifier.AGE_OVER_18(), keccak256("AGE_OVER_18"));
        assertEq(verifier.KYC_VERIFIED(), keccak256("KYC_VERIFIED"));
        assertEq(verifier.COUNTRY_ALLOWED(), keccak256("COUNTRY_ALLOWED"));
        assertEq(verifier.DAO_MEMBER(), keccak256("DAO_MEMBER"));
    }
}

// ==========================================================================
// v2.3 — HuntKeyPaymaster Tests
// ==========================================================================

import {HuntKeyPaymaster} from "../src/HuntKeyPaymaster.sol";
import {IPaymaster, PostOpMode} from "../src/IPaymaster.sol";

/// @dev Mock ERC20 token for paymaster tests
contract MockERC20 {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        require(allowance[from][msg.sender] >= amount, "insufficient allowance");
        require(balanceOf[from] >= amount, "insufficient balance");
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        return true;
    }
}

contract HuntKeyPaymasterTest is Test {
    HuntKeyPaymaster paymaster;
    MockEntryPoint entryPoint;
    MockERC20 token;
    address alice = address(0xA11CE);

    function setUp() public {
        entryPoint = new MockEntryPoint();
        paymaster = new HuntKeyPaymaster(address(entryPoint));
        token = new MockERC20();

        vm.deal(address(paymaster), 10 ether);
    }

    function _buildUserOp(address sender, uint8 mode, address tokenAddr) internal pure returns (PackedUserOperation memory) {
        bytes memory paymasterAndData;
        if (mode == 2) {
            paymasterAndData = abi.encodePacked(address(0), mode, tokenAddr);
        } else {
            paymasterAndData = abi.encodePacked(address(0), mode);
        }

        return PackedUserOperation({
            sender: sender,
            nonce: 0,
            initCode: new bytes(0),
            callData: new bytes(0),
            accountGasLimits: bytes32(0),
            preVerificationGas: 0,
            gasFees: bytes32(0),
            paymasterAndData: paymasterAndData,
            signature: new bytes(0)
        });
    }

    function testSponsoredMode() public {
        paymaster.setSponsoredAccount(alice, true);

        PackedUserOperation memory op = _buildUserOp(alice, 1, address(0));

        vm.prank(address(entryPoint));
        (bytes memory context, uint256 validationData) = paymaster.validatePaymasterUserOp(op, bytes32(0), 0.1 ether);

        assertEq(validationData, 0);
        assertEq(context.length, 0);
    }

    function testSponsoredModeRejectUnsponsored() public {
        PackedUserOperation memory op = _buildUserOp(alice, 1, address(0));

        vm.prank(address(entryPoint));
        vm.expectRevert(HuntKeyPaymaster.AccountNotSponsored.selector);
        paymaster.validatePaymasterUserOp(op, bytes32(0), 0.1 ether);
    }

    function testTokenPayMode() public {
        paymaster.setTokenGasPrice(address(token), 1e15); // 0.001 token per gas unit

        PackedUserOperation memory op = _buildUserOp(alice, 2, address(token));

        vm.prank(address(entryPoint));
        (bytes memory context, uint256 validationData) = paymaster.validatePaymasterUserOp(op, bytes32(0), 0.1 ether);

        assertEq(validationData, 0);
        assertTrue(context.length > 0);
    }

    function testTokenPayRejectUnconfigured() public {
        PackedUserOperation memory op = _buildUserOp(alice, 2, address(token));

        vm.prank(address(entryPoint));
        vm.expectRevert(HuntKeyPaymaster.TokenNotAllowed.selector);
        paymaster.validatePaymasterUserOp(op, bytes32(0), 0.1 ether);
    }

    function testUnsupportedModeReverts() public {
        PackedUserOperation memory op = _buildUserOp(alice, 0, address(0));

        vm.prank(address(entryPoint));
        vm.expectRevert(HuntKeyPaymaster.UnsupportedMode.selector);
        paymaster.validatePaymasterUserOp(op, bytes32(0), 0.1 ether);
    }

    function testOnlyEntryPoint() public {
        PackedUserOperation memory op = _buildUserOp(alice, 1, address(0));

        vm.expectRevert(HuntKeyPaymaster.OnlyEntryPoint.selector);
        paymaster.validatePaymasterUserOp(op, bytes32(0), 0.1 ether);
    }

    function testDeposit() public {
        paymaster.deposit{value: 1 ether}();
        assertEq(entryPoint.deposits(address(paymaster)), 1 ether);
    }

    function testWithdrawOnlyOwner() public {
        paymaster.deposit{value: 1 ether}();

        vm.prank(address(0xBEEF));
        vm.expectRevert(HuntKeyPaymaster.OnlyOwner.selector);
        paymaster.withdraw(payable(address(0xBEEF)), 0.5 ether);
    }

    function testPostOpTokenCollection() public {
        paymaster.setTokenGasPrice(address(token), 1e18); // 1:1 ratio

        // Give alice tokens and approve paymaster
        token.mint(alice, 10 ether);
        vm.prank(alice);
        token.approve(address(paymaster), 10 ether);

        // Simulate postOp with context from token pay mode
        bytes memory context = abi.encode(alice, address(token), uint256(1 ether));
        uint256 actualGasCost = 0.5 ether; // gas cost in ETH

        vm.prank(address(entryPoint));
        paymaster.postOp(PostOpMode.opSucceeded, context, actualGasCost, 0);

        // Token amount = (0.5 ether * 1e18) / 1e18 = 0.5 ether in tokens
        assertEq(token.balanceOf(address(paymaster)), 0.5 ether);
        assertEq(token.balanceOf(alice), 9.5 ether);
    }
}
