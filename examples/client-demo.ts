/**
 * HuntKey Client Demo
 *
 * Demonstrates how a dApp integrates with the HuntKey SDK to construct
 * and sign a SovereignIntent through the 3-layer signing chain.
 *
 * Flow:
 *   1. Compute calldata hash for the target function call
 *   2. Create a SovereignIntent binding the exact call parameters
 *   3. Sign a SessionCertificate with the action key
 *   4. Submit (session, intent, calldata) to ExecutionGateway.execute()
 *
 * SECURITY: Private keys are shown here for demonstration only.
 * In production, keys must come from a secure enclave, hardware wallet,
 * or encrypted keystore. Never hardcode private keys.
 */

import { init, IntentSigner, SessionManager, ProtocolAuditor, zeroize } from "../sdk/ts/src/index";
// In production: import huntkey_wasm from "@huntkey/wasm";

// --- Step 0: Initialize the WASM module ---
// init(huntkey_wasm);

// For this demo, we simulate the flow without actual WASM execution.
// The code below shows the exact API surface a dApp would use.

async function demo() {
  // --- Configuration ---
  const CHAIN_ID = 1;
  const GATEWAY_ADDRESS = "aa".repeat(20); // ExecutionGateway contract address
  const TARGET_CONTRACT = "bb".repeat(20); // Target contract for the call
  const TRANSFER_SELECTOR = "a9059cbb";    // transfer(address,uint256)

  // --- Step 1: Compute calldata hash ---
  // Encode the function call: transfer(0x1234...5678, 1000000)
  const recipientPadded = "0000000000000000000000001234567890abcdef1234567890abcdef12345678";
  const amountPadded = "00000000000000000000000000000000000000000000000000000000000f4240";
  const callData = TRANSFER_SELECTOR + recipientPadded + amountPadded;

  // The SDK computes keccak256(callData) — this hash is signed into the intent,
  // preventing any mutation of the calldata between signing and on-chain execution.
  console.log("1. Computing calldata hash...");
  // const callDataHash = IntentSigner.computeCallDataHash(callData);
  const callDataHash = "0".repeat(64); // placeholder for demo
  console.log(`   Hash: 0x${callDataHash}`);

  // --- Step 2: Create the SovereignIntent ---
  console.log("\n2. Creating SovereignIntent...");
  const intent = {
    targetContract: TARGET_CONTRACT,
    functionSig: TRANSFER_SELECTOR,
    recipient: "1234567890abcdef1234567890abcdef12345678",
    assetAddress: "00".repeat(20), // native ETH
    callDataHash: callDataHash,
    maxValue: "1000000000000000000", // 1 ETH in wei
    expiration: Math.floor(Date.now() / 1000) + 3600, // 1 hour from now
    chainId: CHAIN_ID,
    nonce: 0,
    sessionEpoch: 0,                 // must match on-chain sessionEpoch[root]
    gasLimit: 100000,                // ERC-4337 gas limit
    maxFeePerGas: "50000000000",          // 50 gwei
    maxPriorityFeePerGas: "2000000000",  // 2 gwei tip — anti-siphoning binding
    requiredClaim: "00".repeat(32),      // no claim required
  };
  console.log(`   Target: 0x${intent.targetContract}`);
  console.log(`   Selector: 0x${intent.functionSig}`);
  console.log(`   Max Value: ${intent.maxValue} wei`);
  console.log(`   Expiration: ${new Date(intent.expiration * 1000).toISOString()}`);

  // Validate and serialize the intent via WASM
  // const intentJson = IntentSigner.createIntent(intent);

  // --- Step 3: Sign the SessionCertificate ---
  console.log("\n3. Signing SessionCertificate...");

  // In production, the action key comes from a secure source.
  // The session address comes from HKDF derivation in the Rust core.
  const sessionCert = {
    session: "cc".repeat(20),      // derived session key address
    parent: "dd".repeat(20),       // action key address
    scope: TRANSFER_SELECTOR,
    target: TARGET_CONTRACT,
    maxValue: "2000000000000000000", // 2 ETH session cap
    expiration: Math.floor(Date.now() / 1000) + 7200,
    chainId: CHAIN_ID,
  };

  // SECURITY: actionPrivkeyHex is zeroed inside the WASM layer after signing.
  // The caller MUST also zero their local copy.
  const actionPrivkeyHex = "ab".repeat(32); // DEMO ONLY — never hardcode
  const actionKeyBytes = new Uint8Array(
    actionPrivkeyHex.match(/.{2}/g)!.map((b) => parseInt(b, 16))
  );

  // const sig = SessionManager.signSessionCert(sessionCert, GATEWAY_ADDRESS, actionPrivkeyHex);
  // console.log(`   v: ${sig.v}`);
  // console.log(`   r: 0x${sig.r}`);
  // console.log(`   s: 0x${sig.s}`);

  // Zeroize the local key copy immediately after use
  zeroize(actionKeyBytes);
  console.log("   [action key bytes zeroed]");

  // --- Step 4: Submit to ExecutionGateway ---
  console.log("\n4. Ready to submit to ExecutionGateway.execute()");
  console.log("   Payload:");
  console.log("     - SessionParams (session cert + signature)");
  console.log("     - IntentParams (intent + signature)");
  console.log("     - target address");
  console.log("     - raw calldata");
  console.log("\n   The gateway validates the full 3-layer chain:");
  console.log("     Layer 1: Session cert signed by authorized action key");
  console.log("     Layer 2: Intent signed by the declared session key");
  console.log("     Layer 3: Scope, target, selector, calldata hash all match");
  console.log("     Result:  target.call{value}(callData)");
}

demo().catch(console.error);
