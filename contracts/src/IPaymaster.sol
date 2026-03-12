// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {PackedUserOperation} from "./IAccount.sol";

/// @title IPaymaster — ERC-4337 Paymaster interface
/// @notice Minimal interface required by the EntryPoint to validate paymaster sponsorship.
interface IPaymaster {
    /// @notice Validate whether the paymaster agrees to pay for the UserOperation.
    /// @param userOp The UserOperation to validate.
    /// @param userOpHash Hash of the UserOperation.
    /// @param maxCost Maximum cost the paymaster could be charged.
    /// @return context Opaque data passed to postOp (empty if no post-op needed).
    /// @return validationData Packed (authorizer, validUntil, validAfter) or 0/1.
    function validatePaymasterUserOp(
        PackedUserOperation calldata userOp,
        bytes32 userOpHash,
        uint256 maxCost
    ) external returns (bytes memory context, uint256 validationData);

    /// @notice Post-operation handler called after UserOp execution.
    /// @param mode 0 = op succeeded, 1 = op reverted, 2 = postOp itself reverted.
    /// @param context Opaque data returned from validatePaymasterUserOp.
    /// @param actualGasCost Actual gas cost charged.
    /// @param actualUserOpFeePerGas Actual fee per gas used.
    function postOp(
        PostOpMode mode,
        bytes calldata context,
        uint256 actualGasCost,
        uint256 actualUserOpFeePerGas
    ) external;
}

/// @notice Post-operation mode enum for paymaster callback.
enum PostOpMode {
    /// UserOp succeeded.
    opSucceeded,
    /// UserOp reverted.
    opReverted,
    /// PostOp itself reverted (second call).
    postOpReverted
}
