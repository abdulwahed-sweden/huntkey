// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {HiAbed} from "../src/HiAbed.sol";
import {Test, console} from "forge-std/Test.sol";

contract HiAbedTest is Test {
    HiAbed hi;

    function setUp() public {
        hi = new HiAbed();
    }

    function testTodayInMinutes() public {
        vm.warp(1741700000); // ~March 11, 2025
        uint256 minutes_ = hi.todayInMinutes();
        console.log("Minutes since epoch:", minutes_);
        assertGt(minutes_, 0);
    }
}
