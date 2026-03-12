// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {IPaymaster, PostOpMode} from "./IPaymaster.sol";
import {PackedUserOperation} from "./IAccount.sol";

/// @title HuntKeyPaymaster — ERC-4337 Paymaster with ETH sponsorship and ERC20 token payment
/// @notice Supports three modes:
///         - Mode 0 (SelfFunded): No paymaster involvement, user pays gas directly.
///         - Mode 1 (Sponsored): Paymaster sponsors gas in ETH from its EntryPoint deposit.
///         - Mode 2 (TokenPay): User pays gas equivalent in ERC20 tokens post-execution.
contract HuntKeyPaymaster is IPaymaster {
    // --- Paymaster mode constants ---
    uint8 public constant MODE_SELF_FUNDED = 0;
    uint8 public constant MODE_SPONSORED = 1;
    uint8 public constant MODE_TOKEN_PAY = 2;

    // --- Custom errors ---
    error OnlyOwner();
    error OnlyEntryPoint();
    error UnsupportedMode();
    error AccountNotSponsored();
    error TokenNotAllowed();
    error TokenTransferFailed();
    error InsufficientDeposit();
    error WithdrawFailed();

    // --- State ---
    address public owner;
    address public entryPoint;

    /// @notice Accounts approved for ETH sponsorship (mode 1).
    mapping(address => bool) public sponsoredAccounts;

    /// @notice ERC20 tokens accepted for gas payment (mode 2).
    ///         token => price per gas unit in token's smallest denomination.
    mapping(address => uint256) public tokenGasPrice;

    /// @notice Total gas cost owed by each account in each token (settled in postOp).
    mapping(address => mapping(address => uint256)) public tokenDebt;

    // --- Events ---
    event AccountSponsored(address indexed account, bool sponsored);
    event TokenConfigured(address indexed token, uint256 pricePerGas);
    event GasSponsored(address indexed account, uint256 maxCost);
    event TokenPaymentCollected(address indexed account, address indexed token, uint256 amount);
    event Deposited(address indexed sender, uint256 amount);
    event Withdrawn(address indexed to, uint256 amount);

    modifier onlyOwner() {
        if (msg.sender != owner) revert OnlyOwner();
        _;
    }

    modifier onlyEntryPoint() {
        if (msg.sender != entryPoint) revert OnlyEntryPoint();
        _;
    }

    constructor(address _entryPoint) {
        owner = msg.sender;
        entryPoint = _entryPoint;
    }

    // --- Sponsorship management ---

    /// @notice Approve or revoke ETH sponsorship for an account.
    function setSponsoredAccount(address account, bool sponsored) external onlyOwner {
        sponsoredAccounts[account] = sponsored;
        emit AccountSponsored(account, sponsored);
    }

    /// @notice Configure an ERC20 token for gas payment with a price per gas unit.
    /// @param token The ERC20 token address. Set pricePerGas to 0 to disable.
    /// @param pricePerGas Price in token's smallest unit per gas unit.
    function setTokenGasPrice(address token, uint256 pricePerGas) external onlyOwner {
        tokenGasPrice[token] = pricePerGas;
        emit TokenConfigured(token, pricePerGas);
    }

    // --- IPaymaster implementation ---

    /// @notice Validate a paymaster-sponsored UserOperation.
    ///         paymasterAndData format: [paymaster(20)] [mode(1)] [token(20, mode 2 only)]
    function validatePaymasterUserOp(
        PackedUserOperation calldata userOp,
        bytes32 /* userOpHash */,
        uint256 maxCost
    ) external onlyEntryPoint returns (bytes memory context, uint256 validationData) {
        // Extract mode from paymasterAndData: first 20 bytes = paymaster address, next 1 byte = mode
        if (userOp.paymasterAndData.length < 21) revert UnsupportedMode();
        uint8 mode = uint8(userOp.paymasterAndData[20]);

        if (mode == MODE_SPONSORED) {
            // ETH sponsorship: check account is approved
            if (!sponsoredAccounts[userOp.sender]) revert AccountNotSponsored();
            emit GasSponsored(userOp.sender, maxCost);
            // No context needed for pure sponsorship
            return (new bytes(0), 0);
        } else if (mode == MODE_TOKEN_PAY) {
            // ERC20 token payment: extract token address
            if (userOp.paymasterAndData.length < 41) revert TokenNotAllowed();
            address token = address(bytes20(userOp.paymasterAndData[21:41]));
            if (tokenGasPrice[token] == 0) revert TokenNotAllowed();

            // Encode context for postOp: account, token, maxCost
            context = abi.encode(userOp.sender, token, maxCost);
            return (context, 0);
        } else {
            revert UnsupportedMode();
        }
    }

    /// @notice Post-operation handler. For token payment mode, collects ERC20 tokens.
    function postOp(
        PostOpMode /* mode */,
        bytes calldata context,
        uint256 actualGasCost,
        uint256 /* actualUserOpFeePerGas */
    ) external onlyEntryPoint {
        if (context.length == 0) return; // Sponsored mode, nothing to collect

        (address account, address token, ) = abi.decode(context, (address, address, uint256));

        // Calculate token amount: actualGasCost * tokenGasPrice / 1e18
        // tokenGasPrice is in token units per gas unit (scaled by 1e18 for precision)
        uint256 tokenAmount = (actualGasCost * tokenGasPrice[token]) / 1e18;
        if (tokenAmount == 0) return;

        // Transfer tokens from account to paymaster
        // Uses transferFrom — account must have approved this paymaster
        (bool success, bytes memory data) = token.call(
            abi.encodeWithSignature("transferFrom(address,address,uint256)", account, address(this), tokenAmount)
        );
        if (!success || (data.length > 0 && !abi.decode(data, (bool)))) {
            revert TokenTransferFailed();
        }

        emit TokenPaymentCollected(account, token, tokenAmount);
    }

    // --- Deposit management ---

    /// @notice Deposit ETH into the EntryPoint for paymaster gas sponsorship.
    function deposit() external payable {
        (bool success,) = payable(entryPoint).call{value: msg.value}(
            abi.encodeWithSignature("depositTo(address)", address(this))
        );
        if (!success) revert InsufficientDeposit();
        emit Deposited(msg.sender, msg.value);
    }

    /// @notice Withdraw from the paymaster's EntryPoint deposit.
    function withdraw(address payable to, uint256 amount) external onlyOwner {
        (bool success,) = entryPoint.call(
            abi.encodeWithSignature("withdrawTo(address,uint256)", to, amount)
        );
        if (!success) revert WithdrawFailed();
        emit Withdrawn(to, amount);
    }

    /// @notice Query the paymaster's deposit balance on the EntryPoint.
    function getDeposit() external view returns (uint256) {
        (bool success, bytes memory data) = entryPoint.staticcall(
            abi.encodeWithSignature("balanceOf(address)", address(this))
        );
        if (!success || data.length < 32) return 0;
        return abi.decode(data, (uint256));
    }

    /// @notice Allow receiving ETH.
    receive() external payable {}
}
