// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract HiAbed {
    function todayInMinutes() external view returns (uint256) {
        return block.timestamp / 60;
    }
}
