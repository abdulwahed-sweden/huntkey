// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {FlashLoanSimpleReceiverBase} from "@aave/core-v3/contracts/flashloan/base/FlashLoanSimpleReceiverBase.sol";
import {IPoolAddressesProvider}       from "@aave/core-v3/contracts/interfaces/IPoolAddressesProvider.sol";
import {IPool}                        from "@aave/core-v3/contracts/interfaces/IPool.sol";
import {IERC20}                       from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20}                    from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Ownable}                      from "@openzeppelin/contracts/access/Ownable.sol";

/**
 * @title  HuntLoanFlashReceiver
 * @notice Aave V3 flash-loan liquidation contract for the HuntLoan execution engine.
 *         Executes undercollateralised position liquidations on Base mainnet.
 *
 * Investment terms (immutable, set at deploy):
 *   - Capital deposited by the Financier.
 *   - Operator executes liquidations using flash loans — zero capital risk per tx.
 *   - After the 6-month duration, profits are split:
 *       Financier: full capital recovery + 60% of net profit.
 *       Operator:  40% of net profit (0 if net profit is negative).
 *   - Gas fees for each liquidation call are paid by the operator wallet (off-chain).
 */
contract HuntLoanFlashReceiver is FlashLoanSimpleReceiverBase, Ownable {
    using SafeERC20 for IERC20;

    // ── Investment parameters ────────────────────────────────────────────────

    address public immutable financier;   // capital provider
    address public immutable operator;    // execution bot wallet
    address public immutable usdc;        // USDC contract on Base

    uint256 public immutable capital;        // initial capital in USDC (6-dec)
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

    // ── Errors ───────────────────────────────────────────────────────────────

    error OnlyOperator();
    error ContractSettled();
    error OnlyAavePool();
    error LiquidationUnprofitable(uint256 received, uint256 owed);
    error SwapNotImplemented();

    // ── Constructor ──────────────────────────────────────────────────────────

    constructor(
        address _provider,      // Aave V3 PoolAddressesProvider on Base
        address _financier,     // financier wallet
        address _operator,      // operator wallet (execution bot)
        address _usdc,          // USDC on Base: 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913
        uint256 _capitalAmount  // initial capital in USDC (6-dec), e.g. 10_000e6
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

    // ── Main entry point — called by the HuntLoan Rust bot ──────────────────

    /**
     * @notice Borrow `debtAmount` of `debtAsset` via Aave V3 flash loan,
     *         liquidate `borrower`, repay loan + premium, keep collateral surplus.
     * @param debtAsset       Token to borrow and repay (must match borrower's debt).
     * @param debtAmount      Amount to borrow (up to 50% of borrower's debt).
     * @param collateralAsset Collateral token to seize.
     * @param borrower        Target underwater position.
     */
    function requestFlashLiquidation(
        address debtAsset,
        uint256 debtAmount,
        address collateralAsset,
        address borrower
    ) external {
        if (msg.sender != operator) revert OnlyOperator();
        if (settled) revert ContractSettled();

        // Store context for the executeOperation callback
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

    // ── Aave V3 callback ─────────────────────────────────────────────────────

    /**
     * @dev Called by Aave immediately after flash loan funds are transferred.
     *      Liquidates the borrower, swaps collateral to debt token, repays loan.
     *      Any surplus above (amount + premium) stays in contract as profit.
     */
    function executeOperation(
        address asset,
        uint256 amount,
        uint256 premium,
        address /*initiator*/,
        bytes calldata /*params*/
    ) external override returns (bool) {
        if (msg.sender != address(POOL)) revert OnlyAavePool();

        address borrower        = _pendingBorrower;
        address collateralAsset = _pendingCollateralAsset;

        // 1. Approve Aave pool to pull debt token for liquidation call
        IERC20(asset).approve(address(POOL), amount);

        // 2. Execute Aave V3 liquidation — seize collateral at bonus
        uint256 collBefore = IERC20(collateralAsset).balanceOf(address(this));
        POOL.liquidationCall(collateralAsset, asset, borrower, amount, false);
        uint256 collSeized = IERC20(collateralAsset).balanceOf(address(this)) - collBefore;

        // 3. Swap seized collateral → debtAsset to repay the flash loan
        //    Integrate Uniswap V3 SwapRouter or Aerodrome here (see TODO).
        uint256 received = _swapCollateralToDebt(collateralAsset, asset, collSeized);

        // 4. Verify profitability and repay flash loan (amount + Aave premium)
        uint256 owed = amount + premium;
        if (received < owed) revert LiquidationUnprofitable(received, owed);
        IERC20(asset).approve(address(POOL), owed);

        // 5. Accumulate net profit
        uint256 profit = received - owed;
        totalProfit += profit;

        // Clear per-call context
        _pendingDebtAsset        = address(0);
        _pendingCollateralAsset  = address(0);
        _pendingBorrower         = address(0);

        emit LiquidationExecuted(borrower, asset, amount, collSeized, profit);
        return true;
    }

    // ── Profit distribution ──────────────────────────────────────────────────

    /**
     * @notice Distribute profits after the 6-month maturity period.
     *         Anyone can trigger this to finalise the investment agreement.
     *         Financier receives capital + 60% profit; operator receives 40% profit.
     */
    function settle() external {
        require(block.timestamp >= maturityTime, "HuntLoan: not yet matured");
        if (settled) revert ContractSettled();
        settled = true;

        uint256 balance = IERC20(usdc).balanceOf(address(this));
        uint256 financierShare;
        uint256 operatorShare;

        if (balance <= capital) {
            // Loss scenario: financier recovers what remains, operator gets nothing
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

    // ── Emergency recovery ───────────────────────────────────────────────────

    /**
     * @notice Withdraw any ERC-20 token accidentally sent to this contract.
     *         Only callable by operator (owner), only before settlement.
     */
    function rescueToken(address token, uint256 amount) external onlyOwner {
        if (settled) revert ContractSettled();
        IERC20(token).safeTransfer(operator, amount);
    }

    // ── Internal — DEX swap stub ─────────────────────────────────────────────

    /**
     * @dev Swap `amount` of `collateral` token into `debt` token.
     *      TODO: Implement via Uniswap V3 SwapRouter (0x2626664c...) or
     *            Aerodrome Router (0xcF77a3Ba...) on Base mainnet.
     */
    function _swapCollateralToDebt(
        address /*collateral*/,
        address /*debt*/,
        uint256 /*amount*/
    ) internal pure returns (uint256) {
        revert SwapNotImplemented();
    }
}
