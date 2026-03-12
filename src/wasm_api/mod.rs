//! WASM bindings for TypeScript/JavaScript interop via `wasm-bindgen`.
//!
//! Gated behind the `wasm` feature flag. Exposes three functions:
//! - `create_intent_wasm` — build and serialize a SovereignIntent
//! - `sign_session_cert_wasm` — sign a SessionCertificate and return JSON
//! - `compute_call_hash_wasm` — compute keccak256 of calldata as hex

#[cfg(feature = "wasm")]
use wasm_bindgen::prelude::*;

#[cfg(feature = "wasm")]
use crate::core::call_data_hash;
#[cfg(feature = "wasm")]
use crate::intents::SovereignIntent;
#[cfg(feature = "wasm")]
use crate::sessions::{sign_session_cert, SessionCertificate};

/// Create a SovereignIntent from JS parameters and return it as JSON.
///
/// All address/bytes fields are hex-encoded strings (without 0x prefix).
#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn create_intent_wasm(
    target_contract: &str,
    function_sig: &str,
    recipient: &str,
    asset_address: &str,
    call_data_hash_hex: &str,
    max_value: &str,
    expiration: u64,
    chain_id: u64,
    nonce: u64,
    session_epoch: u64,
    gas_limit: u64,
    max_fee_per_gas: &str,
    required_claim_hex: &str,
) -> Result<String, JsValue> {
    let target: [u8; 20] = hex_to_array(target_contract)?;
    let fn_sig: [u8; 4] = hex_to_array(function_sig)?;
    let recip: [u8; 20] = hex_to_array(recipient)?;
    let asset: [u8; 20] = hex_to_array(asset_address)?;
    let data_hash: [u8; 32] = hex_to_array(call_data_hash_hex)?;
    let req_claim: [u8; 32] = hex_to_array(required_claim_hex)?;
    let max_val: u128 = max_value
        .parse()
        .map_err(|e| JsValue::from_str(&format!("invalid max_value: {}", e)))?;
    let max_fee: u128 = max_fee_per_gas
        .parse()
        .map_err(|e| JsValue::from_str(&format!("invalid max_fee_per_gas: {}", e)))?;

    let intent = SovereignIntent {
        target_contract: target,
        function_sig: fn_sig,
        recipient: recip,
        asset_address: asset,
        call_data_hash: data_hash,
        max_value: max_val,
        expiration,
        chain_id,
        nonce,
        session_epoch,
        gas_limit,
        max_fee_per_gas: max_fee,
        required_claim: req_claim,
    };

    serde_json::to_string(&serde_json::json!({
        "target_contract": hex::encode(intent.target_contract),
        "function_sig": hex::encode(intent.function_sig),
        "recipient": hex::encode(intent.recipient),
        "asset_address": hex::encode(intent.asset_address),
        "call_data_hash": hex::encode(intent.call_data_hash),
        "max_value": intent.max_value.to_string(),
        "expiration": intent.expiration,
        "chain_id": intent.chain_id,
        "nonce": intent.nonce,
        "session_epoch": intent.session_epoch,
        "gas_limit": intent.gas_limit,
        "max_fee_per_gas": intent.max_fee_per_gas.to_string(),
        "required_claim": hex::encode(intent.required_claim),
    }))
    .map_err(|e| JsValue::from_str(&format!("serialization failed: {}", e)))
}

/// Sign a SessionCertificate and return `{ v, r, s }` as JSON.
///
/// All address/bytes fields and the private key are hex-encoded (no 0x prefix).
#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn sign_session_cert_wasm(
    session_addr: &str,
    parent_addr: &str,
    scope: &str,
    target: &str,
    max_value: &str,
    expiration: u64,
    chain_id: u64,
    verifying_contract: &str,
    action_privkey_hex: &str,
) -> Result<String, JsValue> {
    let session: [u8; 20] = hex_to_array(session_addr)?;
    let parent: [u8; 20] = hex_to_array(parent_addr)?;
    let scope_bytes: [u8; 4] = hex_to_array(scope)?;
    let target_bytes: [u8; 20] = hex_to_array(target)?;
    let contract: [u8; 20] = hex_to_array(verifying_contract)?;
    let privkey: [u8; 32] = hex_to_array(action_privkey_hex)?;
    let max_val: u128 = max_value
        .parse()
        .map_err(|e| JsValue::from_str(&format!("invalid max_value: {}", e)))?;

    let cert = SessionCertificate {
        session,
        parent,
        scope: scope_bytes,
        target: target_bytes,
        max_value: max_val,
        expiration,
        chain_id,
    };

    let signed = sign_session_cert(&cert, &contract, &privkey);

    serde_json::to_string(&serde_json::json!({
        "v": signed.v,
        "r": hex::encode(signed.r),
        "s": hex::encode(signed.s),
    }))
    .map_err(|e| JsValue::from_str(&format!("serialization failed: {}", e)))
}

/// Compute keccak256 of hex-encoded calldata, return hex hash string.
#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn compute_call_hash_wasm(call_data_hex: &str) -> Result<String, JsValue> {
    let data = hex::decode(call_data_hex)
        .map_err(|e| JsValue::from_str(&format!("invalid hex: {}", e)))?;
    let hash = call_data_hash(&data);
    Ok(hex::encode(hash))
}

/// Helper: decode a hex string into a fixed-size byte array.
#[cfg(feature = "wasm")]
fn hex_to_array<const N: usize>(hex_str: &str) -> Result<[u8; N], JsValue> {
    let bytes = hex::decode(hex_str)
        .map_err(|e| JsValue::from_str(&format!("invalid hex: {}", e)))?;
    if bytes.len() != N {
        return Err(JsValue::from_str(&format!(
            "expected {} bytes, got {}",
            N,
            bytes.len()
        )));
    }
    let mut arr = [0u8; N];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}
