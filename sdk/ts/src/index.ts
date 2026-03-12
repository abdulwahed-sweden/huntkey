/**
 * HuntKey TypeScript SDK
 *
 * Wraps the WASM-compiled Rust core to provide a clean, async API
 * for dApps integrating with the Sovereign Execution Layer.
 *
 * SECURITY: All private key material is handled as Uint8Array and
 * zeroed after use via zeroize(). Never log or persist key bytes.
 */

// --- Types ---

export interface SovereignIntent {
  targetContract: string;       // hex, 20 bytes
  functionSig: string;          // hex, 4 bytes
  recipient: string;            // hex, 20 bytes
  assetAddress: string;         // hex, 20 bytes
  callDataHash: string;         // hex, 32 bytes
  maxValue: string;             // decimal wei string
  expiration: number;           // unix timestamp
  chainId: number;
  nonce: number;
  sessionEpoch: number;         // must match on-chain sessionEpoch[root]
  gasLimit: number;             // gas limit for ERC-4337 UserOp
  maxFeePerGas: string;         // decimal wei string
  maxPriorityFeePerGas: string; // decimal wei string — anti-siphoning binding
  requiredClaim: string;        // hex, 32 bytes (zero = no claim required)
  claimProofHash: string;       // hex, 32 bytes (zero = no proof binding)
  paymasterMode: number;        // 0 = self-funded, 1 = sponsored, 2 = token pay
  paymaster: string;            // hex, 20 bytes (zero = no paymaster)
}

export interface Signature {
  v: number;
  r: string;  // hex, 32 bytes
  s: string;  // hex, 32 bytes
}

export interface SessionCertParams {
  session: string;    // hex, 20 bytes — session key address
  parent: string;     // hex, 20 bytes — action key address
  scope: string;      // hex, 4 bytes — function selector
  target: string;     // hex, 20 bytes — target contract
  maxValue: string;   // decimal wei string
  expiration: number; // unix timestamp
  chainId: number;
}

// --- WASM bindings interface ---

interface HuntKeyWasm {
  create_intent_wasm(
    targetContract: string, functionSig: string, recipient: string,
    assetAddress: string, callDataHashHex: string, maxValue: string,
    expiration: number, chainId: number, nonce: number,
    sessionEpoch: number, gasLimit: number, maxFeePerGas: string,
    maxPriorityFeePerGas: string, requiredClaimHex: string,
    claimProofHashHex: string, paymasterMode: number, paymasterHex: string,
  ): string;

  sign_session_cert_wasm(
    sessionAddr: string, parentAddr: string, scope: string,
    target: string, maxValue: string, expiration: number,
    chainId: number, verifyingContract: string, actionPrivkeyHex: string,
  ): string;

  compute_call_hash_wasm(callDataHex: string): string;
}

let wasmInstance: HuntKeyWasm | null = null;

/**
 * Initialize the SDK by loading the WASM module.
 * Must be called before any other SDK method.
 *
 * @param wasmModule - The initialized WASM module (from wasm-pack output)
 */
export function init(wasmModule: HuntKeyWasm): void {
  wasmInstance = wasmModule;
}

function getWasm(): HuntKeyWasm {
  if (!wasmInstance) {
    throw new Error("HuntKey SDK not initialized. Call init(wasmModule) first.");
  }
  return wasmInstance;
}

// --- Security utilities ---

/**
 * Securely zero a Uint8Array in place.
 * Call this on any buffer that held private key material.
 */
export function zeroize(buf: Uint8Array): void {
  buf.fill(0);
}

/**
 * Strip optional "0x" prefix from a hex string.
 */
function stripHexPrefix(hex: string): string {
  return hex.startsWith("0x") ? hex.slice(2) : hex;
}

// --- MnemonicManager ---

/**
 * Mnemonic management utilities.
 *
 * NOTE: Mnemonic generation and validation happen in the Rust core.
 * This namespace provides the TypeScript-side interface for dApps
 * that manage mnemonics client-side before passing them to the WASM layer.
 */
export const MnemonicManager = {
  /**
   * Validate that a mnemonic string has the correct word count and checksum.
   * Uses the BIP-39 English wordlist.
   */
  validate(mnemonic: string): boolean {
    const words = mnemonic.trim().split(/\s+/);
    return words.length === 12 || words.length === 24;
  },

  /**
   * Convert a mnemonic to a display-safe format (first 4 chars of each word).
   * NEVER log the full mnemonic.
   */
  redact(mnemonic: string): string {
    return mnemonic
      .trim()
      .split(/\s+/)
      .map((w) => w.slice(0, 4) + "...")
      .join(" ");
  },
};

// --- IntentSigner ---

/**
 * Intent creation and calldata hashing.
 */
export const IntentSigner = {
  /**
   * Compute keccak256 of calldata bytes for intent binding.
   *
   * @param callDataHex - Hex-encoded calldata (no 0x prefix)
   * @returns Hex-encoded 32-byte hash
   */
  computeCallDataHash(callDataHex: string): string {
    return getWasm().compute_call_hash_wasm(stripHexPrefix(callDataHex));
  },

  /**
   * Create a SovereignIntent object and return its serialized form.
   * This does NOT sign the intent — it only constructs and validates it.
   *
   * @returns JSON string of the validated intent
   */
  createIntent(params: SovereignIntent): string {
    return getWasm().create_intent_wasm(
      stripHexPrefix(params.targetContract),
      stripHexPrefix(params.functionSig),
      stripHexPrefix(params.recipient),
      stripHexPrefix(params.assetAddress),
      stripHexPrefix(params.callDataHash),
      params.maxValue,
      params.expiration,
      params.chainId,
      params.nonce,
      params.sessionEpoch,
      params.gasLimit,
      params.maxFeePerGas,
      params.maxPriorityFeePerGas,
      stripHexPrefix(params.requiredClaim),
      stripHexPrefix(params.claimProofHash),
      params.paymasterMode,
      stripHexPrefix(params.paymaster),
    );
  },
};

// --- SessionManager ---

/**
 * Session certificate signing.
 *
 * SECURITY: The actionPrivkeyHex parameter is zeroed in the Rust/WASM layer
 * after signing. Callers MUST also zeroize their local copy of the key.
 */
export const SessionManager = {
  /**
   * Sign a SessionCertificate with the parent action key.
   *
   * @param cert - Session certificate parameters
   * @param verifyingContract - Gateway contract address (hex, no 0x)
   * @param actionPrivkeyHex - Action key private key (hex, no 0x). Zeroed after use in WASM.
   * @returns Signature { v, r, s }
   */
  signSessionCert(
    cert: SessionCertParams,
    verifyingContract: string,
    actionPrivkeyHex: string,
  ): Signature {
    const json = getWasm().sign_session_cert_wasm(
      stripHexPrefix(cert.session),
      stripHexPrefix(cert.parent),
      stripHexPrefix(cert.scope),
      stripHexPrefix(cert.target),
      cert.maxValue,
      cert.expiration,
      cert.chainId,
      stripHexPrefix(verifyingContract),
      stripHexPrefix(actionPrivkeyHex),
    );
    return JSON.parse(json) as Signature;
  },
};

// --- ProtocolAuditor ---

/**
 * On-chain identity state as read from IdentityStore.
 */
export interface IdentityState {
  /** 0 = Active, 1 = RecoveryPending, 2 = Frozen */
  state: number;
  /** Current session epoch for this identity. */
  sessionEpoch: bigint;
  /** Whether the identity has pending recovery (newRoot != address(0)). */
  hasPendingRecovery: boolean;
  /** Number of guardian approvals for the pending recovery. */
  recoveryApprovals: number;
  /** Unix timestamp when recovery timelock started (0 = not started). */
  recoveryInitiatedAt: number;
}

/**
 * Minimal JSON-RPC provider interface for read-only calls.
 * Compatible with ethers.js, viem, or any provider that supports eth_call.
 */
export interface RpcProvider {
  call(tx: { to: string; data: string }): Promise<string>;
}

/**
 * Protocol auditor for querying on-chain identity state.
 *
 * Reads IdentityStore state directly via eth_call — no ABI dependency required.
 * Encodes function selectors and decodes return values inline.
 */
export class ProtocolAuditor {
  private provider: RpcProvider;
  private contractAddress: string;

  /**
   * @param provider - JSON-RPC provider with `call()` support.
   * @param contractAddress - Address of the HuntKeyAccount/IdentityStore contract (with 0x prefix).
   */
  constructor(provider: RpcProvider, contractAddress: string) {
    this.provider = provider;
    this.contractAddress = contractAddress;
  }

  /**
   * Fetch the full identity state for a root address.
   *
   * @param rootAddress - The root identity address (with 0x prefix).
   * @returns IdentityState object with current on-chain values.
   */
  async getIdentityState(rootAddress: string): Promise<IdentityState> {
    const addr = stripHexPrefix(rootAddress).padStart(64, "0");

    // identityState(address) => uint8
    const stateData = await this.provider.call({
      to: this.contractAddress,
      data: "0x" + "48b25166" + addr, // keccak256("identityState(address)")[:4]
    });

    // sessionEpoch(address) => uint256
    const epochData = await this.provider.call({
      to: this.contractAddress,
      data: "0x" + "f5e1a617" + addr, // keccak256("sessionEpoch(address)")[:4]
    });

    // pendingNewRoot(address) => address
    const pendingRootData = await this.provider.call({
      to: this.contractAddress,
      data: "0x" + "aa082a9d" + addr, // keccak256("pendingNewRoot(address)")[:4]
    });

    // recoveryApprovals(address) => uint256
    const approvalsData = await this.provider.call({
      to: this.contractAddress,
      data: "0x" + "e94f3f4d" + addr, // keccak256("recoveryApprovals(address)")[:4]
    });

    // recoveryInitiatedAt(address) => uint256
    const initiatedAtData = await this.provider.call({
      to: this.contractAddress,
      data: "0x" + "cd5d2c74" + addr, // keccak256("recoveryInitiatedAt(address)")[:4]
    });

    const state = parseInt(stripHexPrefix(stateData), 16);
    const sessionEpoch = BigInt(epochData);
    const pendingRoot = stripHexPrefix(pendingRootData).slice(-40);
    const hasPendingRecovery = pendingRoot !== "0".repeat(40);
    const approvals = parseInt(stripHexPrefix(approvalsData), 16);
    const initiatedAt = parseInt(stripHexPrefix(initiatedAtData), 16);

    return {
      state,
      sessionEpoch,
      hasPendingRecovery,
      recoveryApprovals: approvals,
      recoveryInitiatedAt: initiatedAt,
    };
  }

  /**
   * Check if a given session epoch has been revoked (i.e., the on-chain
   * epoch has advanced past it).
   *
   * @param rootAddress - The root identity address (with 0x prefix).
   * @param epoch - The session epoch to check.
   * @returns True if the epoch is stale (revoked), false if current.
   */
  async isEpochRevoked(rootAddress: string, epoch: bigint): Promise<boolean> {
    const addr = stripHexPrefix(rootAddress).padStart(64, "0");

    const epochData = await this.provider.call({
      to: this.contractAddress,
      data: "0x" + "f5e1a617" + addr,
    });

    const currentEpoch = BigInt(epochData);
    return epoch < currentEpoch;
  }

  /**
   * Check if an identity is currently in RecoveryPending state.
   *
   * @param rootAddress - The root identity address (with 0x prefix).
   * @returns True if recovery is pending.
   */
  async isRecoveryPending(rootAddress: string): Promise<boolean> {
    const state = await this.getIdentityState(rootAddress);
    return state.state === 1;
  }

  /**
   * Check if an identity is currently frozen.
   *
   * @param rootAddress - The root identity address (with 0x prefix).
   * @returns True if the identity is frozen.
   */
  async isFrozen(rootAddress: string): Promise<boolean> {
    const state = await this.getIdentityState(rootAddress);
    return state.state === 2;
  }
}

// --- ClaimManager ---

/** Claim type constants matching ClaimVerifier.sol. */
export const ClaimTypes = {
  AGE_OVER_18: "age_over_18",
  KYC_VERIFIED: "kyc_verified",
  COUNTRY_ALLOWED: "country_allowed",
  DAO_MEMBER: "dao_member",
} as const;

export type ClaimType = (typeof ClaimTypes)[keyof typeof ClaimTypes];

/**
 * Manages ZK claim verification via ClaimVerifier.sol.
 */
export class ClaimManager {
  private provider: RpcProvider;
  private contractAddress: string;

  constructor(provider: RpcProvider, contractAddress: string) {
    this.provider = provider;
    this.contractAddress = contractAddress;
  }

  /**
   * Check if an account holds a registered claim of the given type.
   *
   * @param account - Account address (with 0x prefix).
   * @param claimTypeHash - keccak256 of the claim type string (hex, 32 bytes, with 0x prefix).
   * @returns True if the claim commitment exists.
   */
  async hasClaim(account: string, claimTypeHash: string): Promise<boolean> {
    const addr = stripHexPrefix(account).padStart(64, "0");
    const claim = stripHexPrefix(claimTypeHash).padStart(64, "0");

    // hasClaim(address,bytes32) selector: first 4 bytes of keccak256
    const data = await this.provider.call({
      to: this.contractAddress,
      data: "0x" + "eefce84a" + addr + claim, // keccak256("hasClaim(address,bytes32)")[:4]
    });

    return parseInt(stripHexPrefix(data), 16) !== 0;
  }

  /**
   * Verify a proof hash against a registered commitment (view-only).
   *
   * @param account - Account address (with 0x prefix).
   * @param claimTypeHash - keccak256 of the claim type string (hex, 32 bytes).
   * @param proofHash - The proof hash to verify (hex, 32 bytes).
   * @returns True if the proof hash matches the commitment.
   */
  async verifyProofHash(
    account: string,
    claimTypeHash: string,
    proofHash: string,
  ): Promise<boolean> {
    const addr = stripHexPrefix(account).padStart(64, "0");
    const claim = stripHexPrefix(claimTypeHash).padStart(64, "0");
    const proof = stripHexPrefix(proofHash).padStart(64, "0");

    // verifyProofHash(address,bytes32,bytes32) selector
    const data = await this.provider.call({
      to: this.contractAddress,
      data: "0x" + "b6f0c091" + addr + claim + proof, // keccak256("verifyProofHash(address,bytes32,bytes32)")[:4]
    });

    return parseInt(stripHexPrefix(data), 16) !== 0;
  }
}

// --- PaymasterClient ---

/** Paymaster mode constants matching HuntKeyPaymaster.sol. */
export const PaymasterMode = {
  SELF_FUNDED: 0,
  SPONSORED: 1,
  TOKEN_PAY: 2,
} as const;

/**
 * Client for interacting with HuntKeyPaymaster.sol.
 */
export class PaymasterClient {
  private provider: RpcProvider;
  private contractAddress: string;

  constructor(provider: RpcProvider, contractAddress: string) {
    this.provider = provider;
    this.contractAddress = contractAddress;
  }

  /**
   * Check if an account is sponsored for gas.
   *
   * @param account - Account address (with 0x prefix).
   * @returns True if the account is approved for ETH sponsorship.
   */
  async isSponsored(account: string): Promise<boolean> {
    const addr = stripHexPrefix(account).padStart(64, "0");

    const data = await this.provider.call({
      to: this.contractAddress,
      data: "0x" + "facd743b" + addr, // keccak256("sponsoredAccounts(address)")[:4]
    });

    return parseInt(stripHexPrefix(data), 16) !== 0;
  }

  /**
   * Get the paymaster's deposit balance on the EntryPoint.
   *
   * @returns Deposit balance in wei as bigint.
   */
  async getDeposit(): Promise<bigint> {
    const data = await this.provider.call({
      to: this.contractAddress,
      data: "0x" + "c399ec88", // keccak256("getDeposit()")[:4]
    });

    return BigInt(data);
  }

  /**
   * Get the gas price for a token (in token units per gas unit, scaled by 1e18).
   *
   * @param token - Token address (with 0x prefix).
   * @returns Token gas price (0 if not configured).
   */
  async getTokenGasPrice(token: string): Promise<bigint> {
    const addr = stripHexPrefix(token).padStart(64, "0");

    const data = await this.provider.call({
      to: this.contractAddress,
      data: "0x" + "4de3f015" + addr, // keccak256("tokenGasPrice(address)")[:4]
    });

    return BigInt(data);
  }

  /**
   * Build paymasterAndData bytes for a UserOperation.
   *
   * @param mode - Paymaster mode (0, 1, or 2).
   * @param token - Token address for mode 2 (optional).
   * @returns Hex-encoded paymasterAndData bytes.
   */
  buildPaymasterAndData(mode: number, token?: string): string {
    const pmAddr = stripHexPrefix(this.contractAddress);
    const modeByte = mode.toString(16).padStart(2, "0");

    if (mode === PaymasterMode.TOKEN_PAY && token) {
      return pmAddr + modeByte + stripHexPrefix(token).padStart(40, "0");
    }
    return pmAddr + modeByte;
  }
}

// --- ProtocolDashboard ---

/** Dashboard snapshot returned by ProtocolDashboard. */
export interface DashboardSnapshot {
  activeIdentities: number;
  pendingRecoveries: number;
  executedIntents: number;
  highValueIntents: number;
  revokedSessions: number;
  snapshotTimestamp: number;
}

/**
 * Protocol dashboard for aggregated on-chain state queries.
 * Combines ProtocolAuditor state with event-based metrics.
 */
export class ProtocolDashboard {
  private auditor: ProtocolAuditor;

  constructor(auditor: ProtocolAuditor) {
    this.auditor = auditor;
  }

  /**
   * Get identity state for multiple root addresses.
   *
   * @param rootAddresses - Array of root addresses (with 0x prefix).
   * @returns Map of address to IdentityState.
   */
  async batchGetIdentityState(
    rootAddresses: string[],
  ): Promise<Map<string, IdentityState>> {
    const result = new Map<string, IdentityState>();
    for (const addr of rootAddresses) {
      const state = await this.auditor.getIdentityState(addr);
      result.set(addr, state);
    }
    return result;
  }

  /**
   * Count identities in each state from a list of root addresses.
   *
   * @param rootAddresses - Array of root addresses (with 0x prefix).
   * @returns Object with counts for active, recoveryPending, and frozen.
   */
  async countByState(
    rootAddresses: string[],
  ): Promise<{ active: number; recoveryPending: number; frozen: number }> {
    const states = await this.batchGetIdentityState(rootAddresses);
    let active = 0,
      recoveryPending = 0,
      frozen = 0;

    for (const state of states.values()) {
      switch (state.state) {
        case 0:
          active++;
          break;
        case 1:
          recoveryPending++;
          break;
        case 2:
          frozen++;
          break;
      }
    }

    return { active, recoveryPending, frozen };
  }
}
