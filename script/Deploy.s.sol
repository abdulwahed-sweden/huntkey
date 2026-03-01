// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Script, console} from "forge-std/Script.sol";
import {HuntLoanFlashReceiver} from "../contracts/HuntLoanFlashReceiver.sol";

/**
 * @title Deploy
 * @notice Deploys HuntLoanFlashReceiver to Base mainnet.
 *
 * Usage:
 *   forge script script/Deploy.s.sol \
 *     --rpc-url $RPC_URL \
 *     --chain-id 8453 \
 *     --broadcast \
 *     --private-key $PRIVATE_KEY
 *
 * After deployment, copy the logged address into your .env as HUNTLOAN_CONTRACT.
 */
contract Deploy is Script {
    // ── Base mainnet constants ───────────────────────────────────────────────
    address constant AAVE_PROVIDER = 0xe20fCBdBfFC4Dd138cE8b2E6FBb6CB49777ad64D;
    address constant USDC          = 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913;

    function run() external {
        address operator = vm.envAddress("OPERATOR_ADDRESS");

        vm.startBroadcast();

        HuntLoanFlashReceiver receiver = new HuntLoanFlashReceiver(
            AAVE_PROVIDER,
            operator,   // financier = operator (self-funded operation)
            operator,   // operator wallet
            USDC,
            0           // capitalAmount = 0 (flash-loan only, no pre-deposit)
        );

        vm.stopBroadcast();

        console.log("HuntLoanFlashReceiver deployed to:", address(receiver));
        console.log("  Aave provider :", AAVE_PROVIDER);
        console.log("  Operator      :", operator);
        console.log("  USDC          :", USDC);
        console.log("");
        console.log("Next step: add to .env");
        console.log("  HUNTLOAN_CONTRACT=", address(receiver));
    }
}
