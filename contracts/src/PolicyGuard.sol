// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @title PolicyGuard — EIP-712 intent validation with ephemeral key authorization
contract PolicyGuard {
    // --- EIP-712 constants ---
    bytes32 public constant INTENT_TYPEHASH =
        keccak256(
            "SovereignIntent(address targetContract,bytes4 functionSig,uint128 maxValue,uint64 expiration,uint64 chainId,uint64 nonce)"
        );

    bytes32 public immutable DOMAIN_SEPARATOR;

    // --- Malleability guard ---
    uint256 private constant SECP256K1_N_DIV_2 =
        0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A0;

    // --- State ---
    address public owner;
    mapping(address => bool) public authorizedKeys;
    mapping(address => uint256) public nonces;

    // --- Events ---
    event KeyAuthorized(address indexed key);
    event KeyRevoked(address indexed key);
    event IntentValidated(
        address indexed signer,
        address indexed targetContract,
        uint128 maxValue,
        uint64 nonce
    );

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    constructor() {
        owner = msg.sender;
        DOMAIN_SEPARATOR = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256("HuntKey"),
                keccak256("1"),
                block.chainid,
                address(this)
            )
        );
    }

    /// @notice Authorize an ephemeral key for intent validation.
    function authorizeKey(address key) external onlyOwner {
        authorizedKeys[key] = true;
        emit KeyAuthorized(key);
    }

    /// @notice Revoke an ephemeral key.
    function revokeKey(address key) external onlyOwner {
        authorizedKeys[key] = false;
        emit KeyRevoked(key);
    }

    /// @notice Validate and execute an EIP-712 signed intent.
    function validateIntent(
        address targetContract,
        bytes4 functionSig,
        uint128 maxValue,
        uint64 expiration,
        uint64 intentChainId,
        uint64 nonce,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external payable {
        // 1. Check expiration
        require(block.timestamp <= expiration, "intent expired");

        // 2. Check value cap
        require(msg.value <= maxValue, "value exceeds cap");

        // 3. Malleability check
        require(uint256(s) <= SECP256K1_N_DIV_2, "malleable signature: s too high");
        require(v == 27 || v == 28, "invalid v value");

        // 4. EIP-712 digest
        bytes32 structHash = keccak256(
            abi.encode(
                INTENT_TYPEHASH,
                targetContract,
                functionSig,
                maxValue,
                expiration,
                intentChainId,
                nonce
            )
        );
        bytes32 digest = keccak256(
            abi.encodePacked("\x19\x01", DOMAIN_SEPARATOR, structHash)
        );

        // 5. Recover signer
        address signer = ecrecover(digest, v, r, s);
        require(signer != address(0), "ecrecover failed");

        // 6. Authorization check
        require(authorizedKeys[signer], "unauthorized key");

        // 7. Replay protection — per-signer nonce
        require(nonce == nonces[signer], "invalid nonce");
        nonces[signer]++;

        emit IntentValidated(signer, targetContract, maxValue, nonce);
    }
}
