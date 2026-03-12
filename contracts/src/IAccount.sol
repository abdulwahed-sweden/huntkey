// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @title IAccount — ERC-4337 Account interface
/// @notice Minimal interface required by the EntryPoint to validate UserOperations.
interface IAccount {
    /// @notice Validate a UserOperation.
    /// @param userOp The UserOperation to validate (packed format).
    /// @param userOpHash Hash of the UserOperation (without signature), used as the basis for signature.
    /// @param missingAccountFunds Amount the account must pre-fund the EntryPoint with.
    /// @return validationData Packed validation result:
    ///         - 0 for success
    ///         - 1 for signature failure
    ///         - (authorizer, validUntil, validAfter) packed into a uint256 for time-range validation
    function validateUserOp(
        PackedUserOperation calldata userOp,
        bytes32 userOpHash,
        uint256 missingAccountFunds
    ) external returns (uint256 validationData);
}

/// @notice Packed UserOperation struct (ERC-4337 v0.7+)
struct PackedUserOperation {
    address sender;
    uint256 nonce;
    bytes initCode;
    bytes callData;
    bytes32 accountGasLimits;     // callGasLimit (16 bytes) || verificationGasLimit (16 bytes)
    uint256 preVerificationGas;
    bytes32 gasFees;              // maxFeePerGas (16 bytes) || maxPriorityFeePerGas (16 bytes)
    bytes paymasterAndData;
    bytes signature;
}
