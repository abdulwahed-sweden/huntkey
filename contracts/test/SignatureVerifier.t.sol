// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {SignatureVerifier} from "../src/SignatureVerifier.sol";
import {Test, console} from "forge-std/Test.sol";

contract SignatureVerifierTest is Test {
    SignatureVerifier verifier;
    uint256 constant PRIV_KEY = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;
    address signer;

    function setUp() public {
        verifier = new SignatureVerifier();
        signer = vm.addr(PRIV_KEY);
    }

    function testVerifyValidSignature() public view {
        string memory message = "hi abed";
        bytes32 hash = keccak256(
            abi.encodePacked("\x19Ethereum Signed Message:\n7", message)
        );
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(PRIV_KEY, hash);

        bool valid = verifier.verify(signer, message, v, r, s);
        assertTrue(valid, "valid signature should pass");
        console.log("Signer:", signer);
    }

    function testRejectWrongSigner() public view {
        string memory message = "hi abed";
        bytes32 hash = keccak256(
            abi.encodePacked("\x19Ethereum Signed Message:\n7", message)
        );
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(PRIV_KEY, hash);

        address wrong = address(0xdead);
        bool valid = verifier.verify(wrong, message, v, r, s);
        assertFalse(valid, "wrong signer should fail");
    }
}
