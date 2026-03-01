// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {FlashLoanSimpleReceiverBase} from "@aave/core-v3/contracts/flashloan/base/FlashLoanSimpleReceiverBase.sol";
import {IPoolAddressesProvider}       from "@aave/core-v3/contracts/interfaces/IPoolAddressesProvider.sol";
import {IPool}                        from "@aave/core-v3/contracts/interfaces/IPool.sol";
import {IERC20}                       from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20}                    from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Ownable}                      from "@openzeppelin/contracts/access/Ownable.sol";

/**
 * @title  Huntloan
 * @notice Flash-loan liquidation contract with 60/40 profit-sharing.
 *
 * Investment terms (immutable, set at deploy):
 *   - Capital: 10,000 USDC deposited by the Financier (Saeed).
 *   - Operator (Omar) executes liquidations using flash loans — zero capital risk.
 *   - After the 6-month duration, profits are split:
 *       Financier: full capital recovery + 60% of net profit.
 *       Operator:  40% of net profit (0 if net profit is negative).
 *   - Gas fees for each liquidation call are paid by the operator wallet (off-chain).
 */
contract Huntloan is FlashLoanSimpleReceiverBase, Ownable {
    using SafeERC20 for IERC20;

    // ── Investment parameters ────────────────────────────────────────────────

    address public immutable financier; // Saeed — deposited 10,000 USDC
    address public immutable operator;  // Omar  — executes liquidations
    address public immutable usdc;      // USDC contract on Base

    uint256 public immutable capital;        // 10_000e6 (USDC 6-dec)
    uint256 public immutable maturityTime;   // deploy timestamp + 6 months
    bool    public           settled;        // true once profits distributed

    uint256 public totalProfit; // accumulated net profit in USDC (6-dec)

    // ── Flash loan execution context (set per-call, cleared after) ──────────

    address private _pendingDebtAsset;
    address private _pendingCollateralAsset;
    address private _pendingBorrower;

    // ── Events ───────────────────────────────────────────────────────────────

    event LiquidationExecuted(
        address indexed borrower,
        address debtAsset,
        uint256 debtRepaid,
        uint256 collateralSeized,
        uint256 profit
    );
    event ProfitDistributed(uint256 financierShare, uint256 operatorShare);

    // ── Constructor ──────────────────────────────────────────────────────────

    constructor(
        address _provider,      // Aave V3 PoolAddressesProvider on Base
        address _financier,     // Saeed's wallet
        address _operator,      // Omar's wallet
        address _usdc,          // USDC on Base: 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913
        uint256 _capitalAmount  // 10_000e6
    )
        FlashLoanSimpleReceiverBase(IPoolAddressesProvider(_provider))
        Ownable(_operator)
    {
        financier     = _financier;
        operator      = _operator;
        usdc          = _usdc;
        capital       = _capitalAmount;
        maturityTime  = block.timestamp + 180 days;
    }

    // ── Main entry point — called by the Rust bot ────────────────────────────

    /**
     * @notice Borrow `debtAmount` of `debtAsset` via Aave flash loan,
     *         liquidate `borrower`, repay loan, keep the collateral profit.
     */
    function requestFlashLiquidation(
        address debtAsset,
        uint256 debtAmount,
        address collateralAsset,
        address borrower
    ) external {
        require(msg.sender == operator, "Only operator");
        require(!settled, "Contract settled");

        // Store context for the callback
        _pendingDebtAsset        = debtAsset;
        _pendingCollateralAsset  = collateralAsset;
        _pendingBorrower         = borrower;

        // Trigger Aave flash loan — executeOperation is called synchronously
        POOL.flashLoanSimple(
            address(this),
            debtAsset,
            debtAmount,
            "",
            0
        );
    }

    // ── Aave callback ────────────────────────────────────────────────────────

    /**
     * @dev Called by Aave immediately after funds are transferred.
     *      We liquidate the borrower, sell collateral, repay loan + fee.
     *      Any surplus stays in the contract as profit.
     */
    function executeOperation(
        address asset,
        uint256 amount,
        uint256 premium,
        address /*initiator*/,
        bytes calldata /*params*/
    ) external override returns (bool) {
        require(msg.sender == address(POOL), "Only Aave pool");

        address borrower       = _pendingBorrower;
        address collateralAsset = _pendingCollateralAsset;

        // 1. Approve Aave to pull back the debt token for liquidation
        IERC20(asset).approve(address(POOL), amount);

        // 2. Execute Aave liquidation — seize collateral
        uint256 collBefore = IERC20(collateralAsset).balanceOf(address(this));
        POOL.liquidationCall(collateralAsset, asset, borrower, amount, false);
        uint256 collSeized = IERC20(collateralAsset).balanceOf(address(this)) - collBefore;

        // 3. Swap seized collateral → debtAsset (USDC) to repay flash loan
        //    In production: call Uniswap V3 or Aerodrome here.
        //    For the scaffold this is a placeholder — replace with live swap.
        uint256 received = _swapCollateralToDebt(collateralAsset, asset, collSeized);

        // 4. Repay flash loan (amount + fee)
        uint256 owed = amount + premium;
        require(received >= owed, "Liquidation not profitable");
        IERC20(asset).approve(address(POOL), owed);

        // 5. Track profit
        uint256 profit = received - owed;
        totalProfit += profit;

        // Clear context
        _pendingDebtAsset        = address(0);
        _pendingCollateralAsset  = address(0);
        _pendingBorrower         = address(0);

        emit LiquidationExecuted(borrower, asset, amount, collSeized, profit);
        return true;
    }

    // ── Profit distribution ──────────────────────────────────────────────────

    /**
     * @notice Distribute profits after maturity.
     *         Anyone can call this after 6 months to finalise the investment.
     */
    function settle() external {
        require(block.timestamp >= maturityTime, "Not yet matured");
        require(!settled, "Already settled");
        settled = true;

        uint256 balance = IERC20(usdc).balanceOf(address(this));

        uint256 financierShare;
        uint256 operatorShare;

        if (balance <= capital) {
            // Loss: financier takes all remaining, operator gets nothing
            financierShare = balance;
            operatorShare  = 0;
        } else {
            uint256 profit = balance - capital;
            financierShare = capital + (profit * 60) / 100;
            operatorShare  = (profit * 40) / 100;
        }

        if (financierShare > 0) IERC20(usdc).safeTransfer(financier, financierShare);
        if (operatorShare  > 0) IERC20(usdc).safeTransfer(operator,  operatorShare);

        emit ProfitDistributed(financierShare, operatorShare);
    }

    // ── Emergency ────────────────────────────────────────────────────────────

    /**
     * @notice Withdraw any ERC-20 token accidentally sent to this contract.
     *         Only callable by operator (owner), and only before settlement.
     */
    function rescueToken(address token, uint256 amount) external onlyOwner {
        require(!settled, "Already settled");
        IERC20(token).safeTransfer(operator, amount);
    }

    // ── Internal — swap stub (replace with live DEX call) ───────────────────

    function _swapCollateralToDebt(
        address /*collateral*/,
        address /*debt*/,
        uint256 /*amount*/
    ) internal pure returns (uint256) {
        // TODO: integrate Uniswap V3 SwapRouter or Aerodrome on Base
        revert("Swap not implemented — add DEX router call here");
    }
}
