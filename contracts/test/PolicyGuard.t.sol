// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {PolicyGuard} from "../src/PolicyGuard.sol";
import {Test, console} from "forge-std/Test.sol";

contract PolicyGuardTest is Test {
    PolicyGuard guard;

    uint256 constant ACTION_KEY = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;
    address actionSigner;

    uint256 constant SECP256K1_N =
        0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141;

    function setUp() public {
        guard = new PolicyGuard();
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

    /// @dev Helper to build and sign a delegation + intent pair.
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
        PolicyGuard.DelegationParams memory del,
        PolicyGuard.IntentParams memory intent
    ) {
        uint64 chainId = uint64(block.chainid);

        // Sign delegation
        bytes32 delDigest = _delegationDigest(shopAddr, scope, delegationCap, delegationExp, chainId, delegationNonce);
        (uint8 dV, bytes32 dR, bytes32 dS) = vm.sign(ROOT_KEY, delDigest);

        del = PolicyGuard.DelegationParams({
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

        // Sign intent
        bytes32 intentDigest = _digest(target, intentFnSig, intentVal, intentExp, chainId, intentNonce);
        (uint8 iV, bytes32 iR, bytes32 iS) = vm.sign(SHOPPING_KEY, intentDigest);

        intent = PolicyGuard.IntentParams({
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

    // -----------------------------------------------------------------------
    // 7. Happy path — delegated intent (Temporary Shopping Key scenario)
    // -----------------------------------------------------------------------
    function testDelegatedIntentHappyPath() public {
        (address rootAddr, address shopAddr) = _setupDelegation();

        bytes4 scope = bytes4(0xa9059cbb);
        uint64 exp = uint64(block.timestamp + 1 hours);

        (PolicyGuard.DelegationParams memory del, PolicyGuard.IntentParams memory intent) =
            _buildDelegatedCall(
                shopAddr, scope, 2 ether, exp, 0,
                address(0xCAFE), scope, 0.5 ether, exp, 0
            );

        guard.validateDelegatedIntent{value: 0.3 ether}(del, intent);

        assertEq(guard.delegationNonces(rootAddr), 1, "delegation nonce should increment");
        assertEq(guard.nonces(shopAddr), 1, "intent nonce should increment");
    }

    // -----------------------------------------------------------------------
    // 8. Delegation expired
    // -----------------------------------------------------------------------
    function testDelegatedIntentRevertExpiredDelegation() public {
        (, address shopAddr) = _setupDelegation();

        bytes4 scope = bytes4(0xa9059cbb);
        uint64 expiredTs = uint64(block.timestamp - 1);
        uint64 futureTs = uint64(block.timestamp + 1 hours);

        (PolicyGuard.DelegationParams memory del, PolicyGuard.IntentParams memory intent) =
            _buildDelegatedCall(
                shopAddr, scope, 1 ether, expiredTs, 0,
                address(0xCAFE), scope, 0.5 ether, futureTs, 0
            );

        vm.expectRevert("delegation expired");
        guard.validateDelegatedIntent(del, intent);
    }

    // -----------------------------------------------------------------------
    // 9. Unregistered prover
    // -----------------------------------------------------------------------
    function testDelegatedIntentRevertUnregisteredProver() public {
        uint256 rogueRoot = 0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6;
        address shopAddr = vm.addr(SHOPPING_KEY);

        bytes4 scope = bytes4(0xa9059cbb);
        uint64 chainId = uint64(block.chainid);
        uint64 exp = uint64(block.timestamp + 1 hours);

        // Sign delegation with rogue root
        bytes32 delDigest = _delegationDigest(shopAddr, scope, 1 ether, exp, chainId, 0);
        (uint8 dV, bytes32 dR, bytes32 dS) = vm.sign(rogueRoot, delDigest);

        PolicyGuard.DelegationParams memory del = PolicyGuard.DelegationParams({
            delegate: shopAddr,
            scope: scope,
            maxValue: 1 ether,
            expiration: exp,
            chainId: chainId,
            nonce: 0,
            v: dV,
            r: dR,
            s: dS
        });

        bytes32 intentDigest = _digest(address(0xCAFE), scope, 0.5 ether, exp, chainId, 0);
        (uint8 iV, bytes32 iR, bytes32 iS) = vm.sign(SHOPPING_KEY, intentDigest);

        PolicyGuard.IntentParams memory intent = PolicyGuard.IntentParams({
            targetContract: address(0xCAFE),
            functionSig: scope,
            maxValue: 0.5 ether,
            expiration: exp,
            chainId: chainId,
            nonce: 0,
            v: iV,
            r: iR,
            s: iS
        });

        vm.expectRevert("unregistered prover");
        guard.validateDelegatedIntent(del, intent);
    }

    // -----------------------------------------------------------------------
    // 10. Function outside delegation scope
    // -----------------------------------------------------------------------
    function testDelegatedIntentRevertScopeMismatch() public {
        (, address shopAddr) = _setupDelegation();

        bytes4 delegationScope = bytes4(0xa9059cbb); // transfer
        bytes4 intentScope = bytes4(0x095ea7b3);     // approve — not authorized
        uint64 exp = uint64(block.timestamp + 1 hours);

        (PolicyGuard.DelegationParams memory del, PolicyGuard.IntentParams memory intent) =
            _buildDelegatedCall(
                shopAddr, delegationScope, 1 ether, exp, 0,
                address(0xCAFE), intentScope, 0.5 ether, exp, 0
            );

        vm.expectRevert("function outside delegation scope");
        guard.validateDelegatedIntent(del, intent);
    }

    // -----------------------------------------------------------------------
    // 11. Intent exceeds delegation cap
    // -----------------------------------------------------------------------
    function testDelegatedIntentRevertExceedsDelegationCap() public {
        (, address shopAddr) = _setupDelegation();

        bytes4 scope = bytes4(0xa9059cbb);
        uint64 exp = uint64(block.timestamp + 1 hours);

        (PolicyGuard.DelegationParams memory del, PolicyGuard.IntentParams memory intent) =
            _buildDelegatedCall(
                shopAddr, scope, 0.5 ether, exp, 0,
                address(0xCAFE), scope, 1 ether, exp, 0
            );

        vm.expectRevert("intent exceeds delegation cap");
        guard.validateDelegatedIntent(del, intent);
    }

    // -----------------------------------------------------------------------
    // 12. Delegation nonce replay
    // -----------------------------------------------------------------------
    function testDelegatedIntentRevertDelegationNonceReplay() public {
        (, address shopAddr) = _setupDelegation();

        bytes4 scope = bytes4(0xa9059cbb);
        uint64 exp = uint64(block.timestamp + 1 hours);

        // First call succeeds
        (PolicyGuard.DelegationParams memory del1, PolicyGuard.IntentParams memory intent1) =
            _buildDelegatedCall(
                shopAddr, scope, 1 ether, exp, 0,
                address(0xCAFE), scope, 0.5 ether, exp, 0
            );

        guard.validateDelegatedIntent(del1, intent1);

        // Second call with same delegation nonce (0) but new intent nonce (1)
        (PolicyGuard.DelegationParams memory del2, PolicyGuard.IntentParams memory intent2) =
            _buildDelegatedCall(
                shopAddr, scope, 1 ether, exp, 0,
                address(0xCAFE), scope, 0.5 ether, exp, 1
            );

        vm.expectRevert("invalid delegation nonce");
        guard.validateDelegatedIntent(del2, intent2);
    }

    // -----------------------------------------------------------------------
    // 13. Gated function without delegation reverts
    // -----------------------------------------------------------------------
    function testGatedFunctionRevertWithoutDelegation() public {
        vm.expectRevert("delegation required");
        guard.gatedPurchase(address(0xCAFE), 1 ether);
    }
}
