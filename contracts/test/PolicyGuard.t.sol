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

    /// @dev Build the EIP-712 digest that matches the contract's DOMAIN_SEPARATOR.
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

    // -----------------------------------------------------------------------
    // 1. Happy path — valid intent
    // -----------------------------------------------------------------------
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

        // Nonce should have incremented
        assertEq(guard.nonces(actionSigner), 1);
    }

    // -----------------------------------------------------------------------
    // 2. Expired intent reverts
    // -----------------------------------------------------------------------
    function testRevertExpiredIntent() public {
        address target = address(0xBEEF);
        bytes4 fnSig = bytes4(0xa9059cbb);
        uint128 maxVal = 1 ether;
        uint64 exp = uint64(block.timestamp - 1); // already expired
        uint64 chainId = uint64(block.chainid);
        uint64 nonce = 0;

        bytes32 digest = _digest(target, fnSig, maxVal, exp, chainId, nonce);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ACTION_KEY, digest);

        vm.expectRevert("intent expired");
        guard.validateIntent(target, fnSig, maxVal, exp, chainId, nonce, v, r, s);
    }

    // -----------------------------------------------------------------------
    // 3. Value exceeds cap reverts
    // -----------------------------------------------------------------------
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

    // -----------------------------------------------------------------------
    // 4. Unauthorized key reverts
    // -----------------------------------------------------------------------
    function testRevertUnauthorizedKey() public {
        // Use a different key that hasn't been authorized
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

    // -----------------------------------------------------------------------
    // 5. Nonce replay reverts (call twice, second fails)
    // -----------------------------------------------------------------------
    function testRevertNonceReplay() public {
        address target = address(0xBEEF);
        bytes4 fnSig = bytes4(0xa9059cbb);
        uint128 maxVal = 1 ether;
        uint64 exp = uint64(block.timestamp + 1 hours);
        uint64 chainId = uint64(block.chainid);
        uint64 nonce = 0;

        bytes32 digest = _digest(target, fnSig, maxVal, exp, chainId, nonce);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ACTION_KEY, digest);

        // First call succeeds
        guard.validateIntent(target, fnSig, maxVal, exp, chainId, nonce, v, r, s);

        // Second call with same nonce reverts
        vm.expectRevert("invalid nonce");
        guard.validateIntent(target, fnSig, maxVal, exp, chainId, nonce, v, r, s);
    }

    // -----------------------------------------------------------------------
    // 6. Malleable signature reverts (flip s to N-s)
    // -----------------------------------------------------------------------
    function testRevertMalleableSignature() public {
        address target = address(0xBEEF);
        bytes4 fnSig = bytes4(0xa9059cbb);
        uint128 maxVal = 1 ether;
        uint64 exp = uint64(block.timestamp + 1 hours);
        uint64 chainId = uint64(block.chainid);
        uint64 nonce = 0;

        bytes32 digest = _digest(target, fnSig, maxVal, exp, chainId, nonce);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ACTION_KEY, digest);

        // Flip s to upper half
        bytes32 flippedS = bytes32(SECP256K1_N - uint256(s));
        uint8 flippedV = v == 27 ? 28 : 27;

        vm.expectRevert("malleable signature: s too high");
        guard.validateIntent(
            target, fnSig, maxVal, exp, chainId, nonce, flippedV, r, flippedS
        );
    }
}
