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
