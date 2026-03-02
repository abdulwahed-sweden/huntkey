// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {FlashLoanSimpleReceiverBase} from "@aave/core-v3/contracts/flashloan/base/FlashLoanSimpleReceiverBase.sol";
import {IPoolAddressesProvider}       from "@aave/core-v3/contracts/interfaces/IPoolAddressesProvider.sol";
import {IPool}                        from "@aave/core-v3/contracts/interfaces/IPool.sol";
import {IERC20}                       from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20}                    from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Ownable}                      from "@openzeppelin/contracts/access/Ownable.sol";
import {ReentrancyGuard}              from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";

// ── DEX interfaces (inline — no extra package dependency) ───────────────────

/// @dev Uniswap V3 SwapRouter02 — exactInputSingle only
interface IUniswapV3Router {
    struct ExactInputSingleParams {
        address tokenIn;
        address tokenOut;
        uint24  fee;
        address recipient;
        uint256 amountIn;
        uint256 amountOutMinimum;
        uint160 sqrtPriceLimitX96;
    }
    function exactInputSingle(ExactInputSingleParams calldata params)
        external payable returns (uint256 amountOut);
}

/// @dev Aerodrome Router (Solidly fork) — swapExactTokensForTokens only
interface IAerodromeRouter {
    struct Route {
        address from;
        address to;
        bool    stable;
        address factory;
    }
    function swapExactTokensForTokens(
        uint256        amountIn,
        uint256        amountOutMin,
        Route[] calldata routes,
        address        to,
        uint256        deadline
    ) external returns (uint256[] memory amounts);
}

// ─────────────────────────────────────────────────────────────────────────────

/**
 * @title  HuntLoanFlashReceiver
 * @notice Aave V3 flash-loan liquidation contract for the HuntLoan execution engine.
 *         Executes undercollateralised position liquidations on Base mainnet.
 *
 * Swap routing (in priority order):
 *   1. Uniswap V3 — fee tiers 500 (0.05%), 3000 (0.3%), 10000 (1%)
 *   2. Aerodrome   — volatile pool
 *   3. Aerodrome   — stable pool
 *   Reverts with SwapFailed() if no route returns >= minAmountOut.
 *
 * Investment terms (immutable, set at deploy):
 *   - Capital deposited by the Financier.
 *   - Operator executes liquidations using flash loans — zero capital risk per tx.
 *   - After the 6-month duration, profits are split:
 *       Financier: full capital recovery + 60% of net profit.
 *       Operator:  40% of net profit (0 if net profit is negative).
 *   - Gas fees for each liquidation call are paid by the operator wallet (off-chain).
 */
contract HuntLoanFlashReceiver is FlashLoanSimpleReceiverBase, Ownable, ReentrancyGuard {
    using SafeERC20 for IERC20;

    // ── DEX addresses (Base mainnet — immutable) ─────────────────────────────

    address private constant UNISWAP_ROUTER    = 0x2626664c2603336E57B271c5C0b26F421741e481;
    address private constant AERODROME_ROUTER  = 0xcF77a3Ba9A5CA399B7c97c74d54e5b1Beb874E43;
    address private constant AERODROME_FACTORY = 0x420DD381b31aEf6683db6B902084cB0FFECe40Da;

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
    event SwapRouteUsed(address tokenIn, address tokenOut, string route, uint256 amountOut);

    // ── Errors ───────────────────────────────────────────────────────────────

    error OnlyOperator();
    error ContractSettled();
    error OnlyAavePool();
    error LiquidationUnprofitable(uint256 received, uint256 owed);
    /// Raised when all swap routes (Uniswap V3 + Aerodrome) fail or return < minAmountOut.
    error SwapFailed(address tokenIn, address tokenOut, uint256 amountIn, uint256 minOut);

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
    ) external nonReentrant {
        if (msg.sender != operator) revert OnlyOperator();
        if (settled) revert ContractSettled();

        _pendingDebtAsset        = debtAsset;
        _pendingCollateralAsset  = collateralAsset;
        _pendingBorrower         = borrower;

        POOL.flashLoanSimple(address(this), debtAsset, debtAmount, "", 0);
    }

    // ── Aave V3 callback ─────────────────────────────────────────────────────

    /**
     * @dev Called by Aave immediately after flash loan funds are transferred.
     *      Liquidates the borrower, routes collateral swap, repays loan.
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
        // forceApprove resets to 0 before setting, avoiding non-zero allowance revert on USDT
        IERC20(asset).forceApprove(address(POOL), amount);

        // 2. Execute Aave V3 liquidation — seize collateral at bonus
        uint256 collBefore = IERC20(collateralAsset).balanceOf(address(this));
        POOL.liquidationCall(collateralAsset, asset, borrower, amount, false);
        uint256 collSeized = IERC20(collateralAsset).balanceOf(address(this)) - collBefore;
        require(collSeized > 0, "Liquidation yielded zero collateral");

        // 3. Amount owed to Aave: principal + 0.05% premium
        uint256 owed = amount + premium;

        // 4. Swap collateral → debt token; must return at least `owed`
        uint256 received = _swapCollateralToDebt(collateralAsset, asset, collSeized, owed);

        // 5. Safety check (swap already enforces minAmountOut = owed)
        if (received < owed) revert LiquidationUnprofitable(received, owed);

        // 6. Approve Aave to pull repayment (forceApprove resets first)
        IERC20(asset).forceApprove(address(POOL), owed);

        // 7. Accumulate net profit
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
     *         Anyone can trigger to finalise the investment agreement.
     */
    function settle() external {
        require(block.timestamp >= maturityTime, "HuntLoan: not yet matured");
        if (settled) revert ContractSettled();
        settled = true;

        uint256 balance = IERC20(usdc).balanceOf(address(this));
        uint256 financierShare;
        uint256 operatorShare;

        if (balance <= capital) {
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

    function rescueToken(address token, uint256 amount) external onlyOwner {
        if (settled) revert ContractSettled();
        IERC20(token).safeTransfer(operator, amount);
    }

    // ── Internal — DEX swap with route fallback ──────────────────────────────

    /**
     * @dev Swap `amountIn` of `collateral` → `debt`.
     *
     *      Route priority:
     *        1. Uniswap V3 fee=500   (stablecoin / tight pairs)
     *        2. Uniswap V3 fee=3000  (most ETH-paired assets)
     *        3. Uniswap V3 fee=10000 (long-tail assets)
     *        4. Aerodrome volatile pool
     *        5. Aerodrome stable pool
     *
     *      The first route that returns >= minAmountOut is used.
     *      Reverts SwapFailed if no route succeeds.
     *
     * @param collateral   Token received from Aave liquidation.
     * @param debt         Token to return to repay the flash loan.
     * @param amountIn     Amount of collateral to swap.
     * @param minAmountOut Minimum acceptable output (= flash loan owed amount).
     * @return amountOut   Actual tokens received after swap.
     */
    function _swapCollateralToDebt(
        address collateral,
        address debt,
        uint256 amountIn,
        uint256 minAmountOut
    ) internal returns (uint256 amountOut) {

        // ── Route 1-3: Uniswap V3 ────────────────────────────────────────────
        IERC20(collateral).approve(UNISWAP_ROUTER, amountIn);

        uint24[3] memory feeTiers = [uint24(500), uint24(3000), uint24(10000)];
        for (uint256 i = 0; i < 3; i++) {
            try IUniswapV3Router(UNISWAP_ROUTER).exactInputSingle(
                IUniswapV3Router.ExactInputSingleParams({
                    tokenIn:           collateral,
                    tokenOut:          debt,
                    fee:               feeTiers[i],
                    recipient:         address(this),
                    amountIn:          amountIn,
                    amountOutMinimum:  minAmountOut,
                    sqrtPriceLimitX96: 0
                })
            ) returns (uint256 out) {
                // Success — reset approval and return
                IERC20(collateral).approve(UNISWAP_ROUTER, 0);
                emit SwapRouteUsed(collateral, debt, _feeLabel(feeTiers[i]), out);
                return out;
            } catch { /* try next tier */ }
        }
        IERC20(collateral).approve(UNISWAP_ROUTER, 0); // reset if all tiers failed

        // ── Routes 4-5: Aerodrome (volatile then stable) ─────────────────────
        IERC20(collateral).approve(AERODROME_ROUTER, amountIn);

        IAerodromeRouter.Route[] memory routes = new IAerodromeRouter.Route[](1);
        routes[0] = IAerodromeRouter.Route({
            from:    collateral,
            to:      debt,
            stable:  false,
            factory: AERODROME_FACTORY
        });

        // Volatile pool
        try IAerodromeRouter(AERODROME_ROUTER).swapExactTokensForTokens(
            amountIn, minAmountOut, routes, address(this), block.timestamp + 120
        ) returns (uint256[] memory amounts) {
            if (amounts.length > 0 && amounts[amounts.length - 1] >= minAmountOut) {
                IERC20(collateral).approve(AERODROME_ROUTER, 0);
                emit SwapRouteUsed(collateral, debt, "AERODROME_VOLATILE", amounts[amounts.length - 1]);
                return amounts[amounts.length - 1];
            }
        } catch { /* try stable */ }

        // Stable pool
        routes[0].stable = true;
        try IAerodromeRouter(AERODROME_ROUTER).swapExactTokensForTokens(
            amountIn, minAmountOut, routes, address(this), block.timestamp + 120
        ) returns (uint256[] memory amounts) {
            if (amounts.length > 0 && amounts[amounts.length - 1] >= minAmountOut) {
                IERC20(collateral).approve(AERODROME_ROUTER, 0);
                emit SwapRouteUsed(collateral, debt, "AERODROME_STABLE", amounts[amounts.length - 1]);
                return amounts[amounts.length - 1];
            }
        } catch { /* all routes failed */ }

        IERC20(collateral).approve(AERODROME_ROUTER, 0);
        revert SwapFailed(collateral, debt, amountIn, minAmountOut);
    }

    /// @dev Human-readable label for Uniswap fee tier (for event logging).
    function _feeLabel(uint24 fee) private pure returns (string memory) {
        if (fee == 500)   return "UNISWAP_V3_0.05%";
        if (fee == 3000)  return "UNISWAP_V3_0.3%";
        if (fee == 10000) return "UNISWAP_V3_1%";
        return "UNISWAP_V3_UNKNOWN";
    }
}
