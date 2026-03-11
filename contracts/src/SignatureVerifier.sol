// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @title SignatureVerifier — ECDSA signature verifier for BIP-44 derived Ethereum keys
contract SignatureVerifier {
    /// @notice Verify that a message was signed by the expected address.
    /// @param signer   Expected signer address (from BIP-44 derivation)
    /// @param message  The original message that was signed
    /// @param v        Recovery ID (27 or 28)
    /// @param r        ECDSA signature component
    /// @param s        ECDSA signature component
    /// @return valid   True if the recovered address matches the signer
    function verify(
        address signer,
        string calldata message,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external pure returns (bool valid) {
        bytes32 hash = keccak256(
            abi.encodePacked(
                "\x19Ethereum Signed Message:\n",
                _toString(bytes(message).length),
                message
            )
        );
        return ecrecover(hash, v, r, s) == signer;
    }

    /// @notice Convert uint to decimal string (for EIP-191 prefix).
    function _toString(uint256 value) internal pure returns (string memory) {
        if (value == 0) return "0";
        uint256 temp = value;
        uint256 digits;
        while (temp != 0) {
            digits++;
            temp /= 10;
        }
        bytes memory buffer = new bytes(digits);
        while (value != 0) {
            digits--;
            buffer[digits] = bytes1(uint8(48 + (value % 10)));
            value /= 10;
        }
        return string(buffer);
    }
}
