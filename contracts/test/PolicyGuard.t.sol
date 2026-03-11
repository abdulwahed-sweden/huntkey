// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {ExecutionGateway} from "../src/ExecutionGateway.sol";
import {IdentityStore} from "../src/IdentityStore.sol";
import {Test} from "forge-std/Test.sol";

/// @dev Dummy target contract for execute() tests
contract DummyTarget {
    uint256 public lastValue;
    address public lastSender;

    function doSomething(uint256 val) external payable {
        lastValue = val;
        lastSender = msg.sender;
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

    /// @dev Build the EIP-712 intent digest.
    function _digest(
        address targetContract,
        bytes4 functionSig,
        uint128 maxValue,
        uint64 expiration,
        uint64 intentChainId,
        uint64 nonce
    ) internal view returns (bytes32) {
        bytes32 structHash = keccak256(
            abi.encode(
                guard.INTENT_TYPEHASH(),
                targetContract,
                functionSig,
                maxValue,
                expiration,
                intentChainId,
                nonce
            )
        );
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

    // =======================================================================
    // Original 6 tests (direct authorization)
    // =======================================================================

    function testValidateIntent() public {
        address target = address(0xBEEF);
        bytes4 fnSig = bytes4(0xa9059cbb);
        uint128 maxVal = 1 ether;
        uint64 exp = uint64(block.timestamp + 1 hours);
        uint64 chainId = uint64(block.chainid);
        uint64 nonce = 0;

        bytes32 digest = _digest(target, fnSig, maxVal, exp, chainId, nonce);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ACTION_KEY, digest);

        guard.validateIntent{value: 0.5 ether}(
            target, fnSig, maxVal, exp, chainId, nonce, v, r, s
        );

        assertEq(guard.nonces(actionSigner), 1);
    }

    function testRevertExpiredIntent() public {
        address target = address(0xBEEF);
        bytes4 fnSig = bytes4(0xa9059cbb);
        uint128 maxVal = 1 ether;
        uint64 exp = uint64(block.timestamp - 1);
        uint64 chainId = uint64(block.chainid);
        uint64 nonce = 0;

        bytes32 digest = _digest(target, fnSig, maxVal, exp, chainId, nonce);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ACTION_KEY, digest);

        vm.expectRevert("intent expired");
        guard.validateIntent(target, fnSig, maxVal, exp, chainId, nonce, v, r, s);
    }

    function testRevertValueExceedsCap() public {
        address target = address(0xBEEF);
        bytes4 fnSig = bytes4(0xa9059cbb);
        uint128 maxVal = 0.5 ether;
        uint64 exp = uint64(block.timestamp + 1 hours);
        uint64 chainId = uint64(block.chainid);
        uint64 nonce = 0;

        bytes32 digest = _digest(target, fnSig, maxVal, exp, chainId, nonce);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ACTION_KEY, digest);

        vm.expectRevert("value exceeds cap");
        guard.validateIntent{value: 1 ether}(
            target, fnSig, maxVal, exp, chainId, nonce, v, r, s
        );
    }

    function testRevertUnauthorizedKey() public {
        uint256 rogue = 0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef;

        address target = address(0xBEEF);
        bytes4 fnSig = bytes4(0xa9059cbb);
        uint128 maxVal = 1 ether;
        uint64 exp = uint64(block.timestamp + 1 hours);
        uint64 chainId = uint64(block.chainid);
        uint64 nonce = 0;

        bytes32 digest = _digest(target, fnSig, maxVal, exp, chainId, nonce);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(rogue, digest);

        vm.expectRevert("unauthorized key");
        guard.validateIntent(target, fnSig, maxVal, exp, chainId, nonce, v, r, s);
    }

    function testRevertNonceReplay() public {
        address target = address(0xBEEF);
        bytes4 fnSig = bytes4(0xa9059cbb);
        uint128 maxVal = 1 ether;
        uint64 exp = uint64(block.timestamp + 1 hours);
        uint64 chainId = uint64(block.chainid);
        uint64 nonce = 0;

        bytes32 digest = _digest(target, fnSig, maxVal, exp, chainId, nonce);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ACTION_KEY, digest);

        guard.validateIntent(target, fnSig, maxVal, exp, chainId, nonce, v, r, s);

        vm.expectRevert("invalid nonce");
        guard.validateIntent(target, fnSig, maxVal, exp, chainId, nonce, v, r, s);
    }

    function testRevertMalleableSignature() public {
        address target = address(0xBEEF);
        bytes4 fnSig = bytes4(0xa9059cbb);
        uint128 maxVal = 1 ether;
        uint64 exp = uint64(block.timestamp + 1 hours);
        uint64 chainId = uint64(block.chainid);
        uint64 nonce = 0;

        bytes32 digest = _digest(target, fnSig, maxVal, exp, chainId, nonce);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ACTION_KEY, digest);

        bytes32 flippedS = bytes32(SECP256K1_N - uint256(s));
        uint8 flippedV = v == 27 ? 28 : 27;

        vm.expectRevert("malleable signature: s too high");
        guard.validateIntent(
            target, fnSig, maxVal, exp, chainId, nonce, flippedV, r, flippedS
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

        bytes32 intentDigest = _digest(target, intentFnSig, intentVal, intentExp, chainId, intentNonce);
        (uint8 iV, bytes32 iR, bytes32 iS) = vm.sign(SHOPPING_KEY, intentDigest);

        intent = IdentityStore.IntentParams({
            targetContract: target,
            functionSig: intentFnSig,
            maxValue: intentVal,
            expiration: intentExp,
            chainId: chainId,
            nonce: intentNonce,
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

        vm.expectRevert("delegation expired");
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

        bytes32 intentDigest = _digest(address(0xCAFE), scope, 0.5 ether, exp, chainId, 0);
        (uint8 iV, bytes32 iR, bytes32 iS) = vm.sign(SHOPPING_KEY, intentDigest);

        IdentityStore.IntentParams memory intent = IdentityStore.IntentParams({
            targetContract: address(0xCAFE), functionSig: scope, maxValue: 0.5 ether,
            expiration: exp, chainId: chainId, nonce: 0, v: iV, r: iR, s: iS
        });

        vm.expectRevert("unregistered prover");
        guard.validateDelegatedIntent(del, intent);
    }

    function testDelegatedIntentRevertScopeMismatch() public {
        (, address shopAddr) = _setupDelegation();

        bytes4 delegationScope = bytes4(0xa9059cbb);
        bytes4 intentScope = bytes4(0x095ea7b3);
        uint64 exp = uint64(block.timestamp + 1 hours);

        (IdentityStore.DelegationParams memory del, IdentityStore.IntentParams memory intent) =
            _buildDelegatedCall(shopAddr, delegationScope, 1 ether, exp, 0, address(0xCAFE), intentScope, 0.5 ether, exp, 0);

        vm.expectRevert("function outside delegation scope");
        guard.validateDelegatedIntent(del, intent);
    }

    function testDelegatedIntentRevertExceedsDelegationCap() public {
        (, address shopAddr) = _setupDelegation();

        bytes4 scope = bytes4(0xa9059cbb);
        uint64 exp = uint64(block.timestamp + 1 hours);

        (IdentityStore.DelegationParams memory del, IdentityStore.IntentParams memory intent) =
            _buildDelegatedCall(shopAddr, scope, 0.5 ether, exp, 0, address(0xCAFE), scope, 1 ether, exp, 0);

        vm.expectRevert("intent exceeds delegation cap");
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

        vm.expectRevert("invalid delegation nonce");
        guard.validateDelegatedIntent(del2, intent2);
    }

    function testGatedFunctionRevertWithoutDelegation() public {
        vm.expectRevert("delegation required");
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

        (uint8 v1, bytes32 r1, bytes32 s1) = vm.sign(GUARDIAN_KEY_1, digest);
        guard.supportRecovery(oldRoot, v1, r1, s1);

        assertEq(guard.recoveryApprovals(oldRoot), 2);
        assertTrue(guard.recoveryInitiatedAt(oldRoot) > 0, "timelock should have started");

        vm.expectRevert("timelock not expired");
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

        vm.prank(oldRoot);
        guard.cancelRecovery(oldRoot);

        assertEq(guard.pendingNewRoot(oldRoot), address(0));
        assertEq(guard.recoveryApprovals(oldRoot), 0);
        assertEq(guard.recoveryInitiatedAt(oldRoot), 0);
        assertTrue(guard.authorizedProvers(oldRoot));

        vm.expectRevert("no pending recovery");
        guard.finalizeRecovery(oldRoot);
    }

    function testRecoveryRevertNonGuardian() public {
        (address oldRoot, address newRoot,,,) = _setupRecovery();
        uint64 chainId = uint64(block.chainid);

        uint256 rogueKey = 0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef;
        bytes32 digest = _recoveryDigest(oldRoot, newRoot, chainId, 0);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(rogueKey, digest);

        vm.expectRevert("not a guardian");
        guard.initiateRecovery(oldRoot, newRoot, v, r, s);
    }

    function testRecoveryRevertThresholdNotMet() public {
        (address oldRoot, address newRoot,,,) = _setupRecovery();
        uint64 chainId = uint64(block.chainid);

        bytes32 digest = _recoveryDigest(oldRoot, newRoot, chainId, 0);
        (uint8 v0, bytes32 r0, bytes32 s0) = vm.sign(GUARDIAN_KEY_0, digest);
        guard.initiateRecovery(oldRoot, newRoot, v0, r0, s0);

        vm.warp(block.timestamp + 48 hours);

        vm.expectRevert("threshold not met");
        guard.finalizeRecovery(oldRoot);
    }

    function testRecoveryRevertCancelNotRoot() public {
        (address oldRoot, address newRoot,,,) = _setupRecovery();
        uint64 chainId = uint64(block.chainid);

        bytes32 digest = _recoveryDigest(oldRoot, newRoot, chainId, 0);
        (uint8 v0, bytes32 r0, bytes32 s0) = vm.sign(GUARDIAN_KEY_0, digest);
        guard.initiateRecovery(oldRoot, newRoot, v0, r0, s0);

        vm.prank(address(0xDEAD));
        vm.expectRevert("only old root can cancel");
        guard.cancelRecovery(oldRoot);
    }

    function testRecoveryRevertDuplicateApproval() public {
        (address oldRoot, address newRoot,,,) = _setupRecovery();
        uint64 chainId = uint64(block.chainid);

        bytes32 digest = _recoveryDigest(oldRoot, newRoot, chainId, 0);
        (uint8 v0, bytes32 r0, bytes32 s0) = vm.sign(GUARDIAN_KEY_0, digest);
        guard.initiateRecovery(oldRoot, newRoot, v0, r0, s0);

        vm.expectRevert("already approved");
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

    // Session key derived from ACTION_KEY: keccak256(ACTION_KEY || nonce=0)
    // We use a second key as the "session" key for testing
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
        bytes memory /* callData */
    ) internal view returns (
        ExecutionGateway.SessionParams memory sess,
        IdentityStore.IntentParams memory intent
    ) {
        uint64 chainId = uint64(block.chainid);

        // Session cert signed by ACTION_KEY
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

        // Intent signed by session key
        bytes32 intentDigest = _digest(target, scope, intentVal, intentExp, chainId, intentNonce);
        (uint8 iV, bytes32 iR, bytes32 iS) = vm.sign(sessionPrivKey, intentDigest);

        intent = IdentityStore.IntentParams({
            targetContract: target,
            functionSig: scope,
            maxValue: intentVal,
            expiration: intentExp,
            chainId: chainId,
            nonce: intentNonce,
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

        (ExecutionGateway.SessionParams memory sess, IdentityStore.IntentParams memory intent) =
            _buildExecuteCall(SESSION_KEY, sessionAddr, scope, address(dummy), 1 ether, exp, 0.5 ether, exp, 0,
                abi.encodeWithSelector(DummyTarget.doSomething.selector, uint256(42)));

        guard.execute{value: 0.1 ether}(sess, intent, address(dummy),
            abi.encodeWithSelector(DummyTarget.doSomething.selector, uint256(42)));

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

        (ExecutionGateway.SessionParams memory sess1, IdentityStore.IntentParams memory intent1) =
            _buildExecuteCall(SESSION_KEY, sessionAddr, scope, address(dummy), 1 ether, exp, 0.5 ether, exp, 0,
                abi.encodeWithSelector(DummyTarget.doSomething.selector, uint256(1)));

        guard.execute(sess1, intent1, address(dummy),
            abi.encodeWithSelector(DummyTarget.doSomething.selector, uint256(1)));

        // Second use with different nonce — should still fail because session key is burned
        (ExecutionGateway.SessionParams memory sess2, IdentityStore.IntentParams memory intent2) =
            _buildExecuteCall(SESSION_KEY, sessionAddr, scope, address(dummy), 1 ether, exp, 0.5 ether, exp, 1,
                abi.encodeWithSelector(DummyTarget.doSomething.selector, uint256(2)));

        vm.expectRevert("session key already used");
        guard.execute(sess2, intent2, address(dummy),
            abi.encodeWithSelector(DummyTarget.doSomething.selector, uint256(2)));
    }

    // -----------------------------------------------------------------------
    // 23. Selector mismatch — calldata selector != intent selector
    // -----------------------------------------------------------------------
    function testExecuteRevertSelectorMismatch() public {
        DummyTarget dummy = new DummyTarget();
        address sessionAddr = vm.addr(SESSION_KEY);
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);

        (ExecutionGateway.SessionParams memory sess, IdentityStore.IntentParams memory intent) =
            _buildExecuteCall(SESSION_KEY, sessionAddr, scope, address(dummy), 1 ether, exp, 0.5 ether, exp, 0,
                abi.encodeWithSelector(DummyTarget.doSomething.selector, uint256(1)));

        // Call with different selector in calldata
        vm.expectRevert("selector mismatch");
        guard.execute(sess, intent, address(dummy),
            abi.encodeWithSelector(DummyTarget.otherFunction.selector, uint256(1)));
    }

    // -----------------------------------------------------------------------
    // 24. Target mismatch — call target != intent target
    // -----------------------------------------------------------------------
    function testExecuteRevertTargetMismatch() public {
        DummyTarget dummy = new DummyTarget();
        DummyTarget other = new DummyTarget();
        address sessionAddr = vm.addr(SESSION_KEY);
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);

        (ExecutionGateway.SessionParams memory sess, IdentityStore.IntentParams memory intent) =
            _buildExecuteCall(SESSION_KEY, sessionAddr, scope, address(dummy), 1 ether, exp, 0.5 ether, exp, 0,
                abi.encodeWithSelector(DummyTarget.doSomething.selector, uint256(1)));

        vm.expectRevert("call target mismatch");
        guard.execute(sess, intent, address(other),
            abi.encodeWithSelector(DummyTarget.doSomething.selector, uint256(1)));
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

        (ExecutionGateway.SessionParams memory sess, IdentityStore.IntentParams memory intent) =
            _buildExecuteCall(SESSION_KEY, sessionAddr, scope, address(dummy), 1 ether, expiredTs, 0.5 ether, futureTs, 0,
                abi.encodeWithSelector(DummyTarget.doSomething.selector, uint256(1)));

        vm.expectRevert("session expired");
        guard.execute(sess, intent, address(dummy),
            abi.encodeWithSelector(DummyTarget.doSomething.selector, uint256(1)));
    }

    // -----------------------------------------------------------------------
    // 26. Unauthorized parent — session cert signed by non-authorized key
    // -----------------------------------------------------------------------
    function testExecuteRevertUnauthorizedParent() public {
        DummyTarget dummy = new DummyTarget();
        uint256 rogueParent = 0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef;
        address rogueAddr = vm.addr(rogueParent);
        address sessionAddr = vm.addr(SESSION_KEY);
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);
        uint64 chainId = uint64(block.chainid);

        // Session cert signed by rogue key (not authorized)
        bytes32 sessDigest = _sessionDigest(sessionAddr, rogueAddr, scope, address(dummy), 1 ether, exp, chainId);
        (uint8 sV, bytes32 sR, bytes32 sS) = vm.sign(rogueParent, sessDigest);

        ExecutionGateway.SessionParams memory sess = ExecutionGateway.SessionParams({
            session: sessionAddr, parent: rogueAddr, scope: scope, target: address(dummy),
            maxValue: 1 ether, expiration: exp, chainId: chainId, v: sV, r: sR, s: sS
        });

        bytes32 intentDigest = _digest(address(dummy), scope, 0.5 ether, exp, chainId, 0);
        (uint8 iV, bytes32 iR, bytes32 iS) = vm.sign(SESSION_KEY, intentDigest);

        IdentityStore.IntentParams memory intent = IdentityStore.IntentParams({
            targetContract: address(dummy), functionSig: scope, maxValue: 0.5 ether,
            expiration: exp, chainId: chainId, nonce: 0, v: iV, r: iR, s: iS
        });

        vm.expectRevert("session parent not authorized");
        guard.execute(sess, intent, address(dummy),
            abi.encodeWithSelector(DummyTarget.doSomething.selector, uint256(1)));
    }

    // -----------------------------------------------------------------------
    // 27. Intent exceeds session cap
    // -----------------------------------------------------------------------
    function testExecuteRevertExceedsSessionCap() public {
        DummyTarget dummy = new DummyTarget();
        address sessionAddr = vm.addr(SESSION_KEY);
        bytes4 scope = DummyTarget.doSomething.selector;
        uint64 exp = uint64(block.timestamp + 1 hours);

        // Session cap 0.1 ether but intent wants 1 ether
        (ExecutionGateway.SessionParams memory sess, IdentityStore.IntentParams memory intent) =
            _buildExecuteCall(SESSION_KEY, sessionAddr, scope, address(dummy), 0.1 ether, exp, 1 ether, exp, 0,
                abi.encodeWithSelector(DummyTarget.doSomething.selector, uint256(1)));

        vm.expectRevert("intent exceeds session cap");
        guard.execute(sess, intent, address(dummy),
            abi.encodeWithSelector(DummyTarget.doSomething.selector, uint256(1)));
    }
}
