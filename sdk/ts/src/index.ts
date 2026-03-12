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
  targetContract: string;   // hex, 20 bytes
  functionSig: string;      // hex, 4 bytes
  recipient: string;        // hex, 20 bytes
  assetAddress: string;     // hex, 20 bytes
  callDataHash: string;     // hex, 32 bytes
  maxValue: string;         // decimal wei string
  expiration: number;       // unix timestamp
  chainId: number;
  nonce: number;
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
