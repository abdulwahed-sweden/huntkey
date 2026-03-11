//! Telegram alerts — reusable notification system.
//!
//! Alert classes:
//!   BOOT          — bot started successfully
//!   EMERGENCY STOP — circuit breaker triggered
//!   STATUS REPORT  — periodic operational summary
//!   LOW BALANCE    — wallet critically low
//!   HEARTBEAT      — daily proof-of-life

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::Client;
use tracing::warn;
use eyre::Result;

const LINE: &str = "----------------------------";

fn timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02} UTC")
}

// ─────────────────────────────────────────────────────────────────────────────
// SMART ADDRESS DISPLAY
// ─────────────────────────────────────────────────────────────────────────────

pub fn short_addr(addr: &str) -> String {
    if addr.len() < 12 { return addr.to_string(); }
    format!("{}...{}", &addr[..6], &addr[addr.len()-4..])
}

/// Resolve a Base mainnet token address to its ticker symbol.
pub fn token_name(addr: &str) -> String {
    let a = addr.to_lowercase();
    match a.as_str() {
        s if s.ends_with("0000000000000000000006")                     => "WETH".into(),
        s if s.contains("2ae3f1ec7f1f5012cfeab0185bfc7aa3cf0dec22")    => "cbETH".into(),
        s if s.contains("c1cba3fcea344f92d9239c08c0568f6f2f0ee452")    => "wstETH".into(),
        s if s.contains("04c0599ae5a44757c0af6f9ec3b93da8976c150a")    => "weETH".into(),
        s if s.contains("b6fe221fe9eef5aba221c348ba20a1bf5e73624c")    => "rETH".into(),
        s if s.contains("833589fcd6edb6e08f4c7c32d4f71b54bda02913")    => "USDC".into(),
        s if s.contains("d9aaec86b65d86f6a7b5b1b0c42ffa531710b6ca")    => "USDbC".into(),
        s if s.contains("50c5725949a6f0c72e6c4a641f24049a917db0cb")    => "DAI".into(),
        s if s.contains("4a3a6dd60a34bb2aba60d73b4c88315e9ceb6a3d")    => "USDT".into(),
        s if s.contains("cbb7c0000ab88b473b1f5afd9ef808440eed33bf")    => "cbBTC".into(),
        s if s.contains("2260fac5e5542a773aa44fbcfedf7c193bc2c599")    => "WBTC".into(),
        _ => short_addr(addr),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// HUMAN-READABLE ERROR DECODER
// ─────────────────────────────────────────────────────────────────────────────

pub fn explain_error(raw: &str) -> String {
    // ERC20 / transfer errors
    if raw.contains("STF") || raw.contains("SafeERC20") || raw.contains("TransferFailed") {
        return "Token transfer failed -- likely an approval or balance issue.".into();
    }

    // Transaction-level errors
    if raw.contains("nonce too low") {
        return "Nonce conflict -- a previous transaction was confirmed out of order.".into();
    }
    if raw.contains("replacement transaction underpriced") {
        return "Gas price too low to replace the pending transaction.".into();
    }
    if raw.contains("insufficient funds") {
        return "Wallet does not have enough ETH to cover gas fees.".into();
    }
    if raw.contains("timeout") || raw.contains("Timeout") {
        return "RPC request timed out -- network may be congested.".into();
    }

    let clean: String = raw.chars().take(150).collect();
    format!("Unexpected error: {clean}")
}

// ─────────────────────────────────────────────────────────────────────────────
// SESSION STATISTICS
// ─────────────────────────────────────────────────────────────────────────────

pub struct AlertStats {
    pub blocks_processed: AtomicU64,
    pub execs_attempted:  AtomicU64,
    pub execs_succeeded:  AtomicU64,
    pub session_start:    Instant,
}

static STATS: OnceLock<AlertStats> = OnceLock::new();

pub fn get_stats() -> &'static AlertStats {
    STATS.get_or_init(|| AlertStats {
        blocks_processed: AtomicU64::new(0),
        execs_attempted:  AtomicU64::new(0),
        execs_succeeded:  AtomicU64::new(0),
        session_start:    Instant::now(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// RATE LIMITER
// ─────────────────────────────────────────────────────────────────────────────

static ALERT_STATE: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

pub fn throttle_guard(key: &str, limit: Duration) -> bool {
    if limit.is_zero() { return true; }
    let mut guard = ALERT_STATE.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    if map.get(key).is_some_and(|last| last.elapsed() < limit) {
        return false;
    }
    map.insert(key.to_string(), Instant::now());
    true
}

// ─────────────────────────────────────────────────────────────────────────────
// TELEGRAM SEND
// ─────────────────────────────────────────────────────────────────────────────

pub async fn send_telegram(
    text:          impl Into<String>,
    dedupe_key:    Option<&str>,
    throttle_secs: u64,
) -> Result<()> {
    let token   = std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default();
    let chat_id = std::env::var("TELEGRAM_CHAT_ID").unwrap_or_default();
    if token.is_empty() || chat_id.is_empty() { return Ok(()); }

    if dedupe_key.is_some_and(|key| !throttle_guard(key, Duration::from_secs(throttle_secs))) {
        return Ok(());
    }

    let mut message = text.into();
    if message.len() > 4000 {
        message.truncate(3997);
        message.push('\u{2026}');
    }

    let client = Client::new();
    let url = format!("https://api.telegram.org/bot{token}/sendMessage");

    for attempt in 0u32..3 {
        let res = client
            .post(&url)
            .json(&serde_json::json!({
                "chat_id":                  chat_id,
                "text":                     message,
                "parse_mode":               "HTML",
                "disable_web_page_preview": true,
            }))
            .send()
            .await;

        match res {
            Ok(r) if r.status().is_success() => return Ok(()),
            Ok(r) => warn!("Telegram send failed (attempt {}): HTTP {}", attempt + 1, r.status()),
            Err(e) => warn!("Telegram send failed (attempt {}): {}", attempt + 1, e),
        }

        if attempt < 2 {
            tokio::time::sleep(Duration::from_secs(1u64 << attempt)).await;
        }
    }
    Ok(())
}

// ═════════════════════════════════════════════════════════════════════════════
//  BOOT
// ═════════════════════════════════════════════════════════════════════════════

pub fn fmt_boot(mode: &str, contract: &str, operator: &str) -> String {
    let t = timestamp();
    format!(
        "<b>Bot Online</b>\n\
         {LINE}\n\
         {t}\n\
         Mode: <b>{mode}</b>\n\
         Contract: <code>{}</code>\n\
         Operator: <code>{}</code>\n\
         Chain: Base Mainnet (8453)",
        short_addr(contract),
        short_addr(operator),
    )
}

// ═════════════════════════════════════════════════════════════════════════════
//  CIRCUIT BREAKER
// ═════════════════════════════════════════════════════════════════════════════

pub fn fmt_circuit_breaker(trigger: &str, detail: &str) -> String {
    let t = timestamp();
    let detail_clean: String = detail.chars().take(200).collect();

    format!(
        "<b>Emergency Stop -- Circuit Breaker</b>\n\
         {LINE}\n\
         {t}\n\
         \n\
         Trigger: <b>{trigger}</b>\n\
         Detail: {detail_clean}\n\
         \n\
         Action required: Check logs and restart the bot manually."
    )
}

// ═════════════════════════════════════════════════════════════════════════════
//  STATUS REPORT
// ═════════════════════════════════════════════════════════════════════════════

pub fn fmt_summary(
    uptime_secs:  u64,
    blocks:       u64,
    execs_tried:  u64,
    execs_ok:     u64,
) -> String {
    let t = timestamp();
    let uptime = format_uptime(uptime_secs);

    format!(
        "<b>Status Report</b>\n\
         {LINE}\n\
         {t}  |  Uptime: {uptime}\n\
         \n\
         Blocks: {blocks}\n\
         Executions: {execs_tried} attempted, {execs_ok} confirmed"
    )
}

// ═════════════════════════════════════════════════════════════════════════════
//  LOW BALANCE
// ═════════════════════════════════════════════════════════════════════════════

pub fn fmt_low_balance(balance_eth: f64) -> String {
    format!(
        "<b>CRITICAL: LOW BALANCE</b>\n\
         {LINE}\n\
         \n\
         Current: <b>{balance_eth:.6} ETH</b>\n\
         Action: Please fund the operator wallet immediately."
    )
}

pub async fn send_low_balance(balance_eth: f64) {
    let msg = fmt_low_balance(balance_eth);
    let _ = send_telegram(msg, Some("low-balance"), 3600).await;
}

// ═════════════════════════════════════════════════════════════════════════════
//  HEARTBEAT
// ═════════════════════════════════════════════════════════════════════════════

pub fn fmt_heartbeat(info_count: usize) -> String {
    let t = timestamp();
    format!(
        "<b>Heartbeat</b>\n\
         {LINE}\n\
         {t}\n\
         \n\
         System is UP. Items tracked: {info_count}"
    )
}

pub async fn send_heartbeat(info_count: usize) {
    let msg = fmt_heartbeat(info_count);
    let _ = send_telegram(msg, Some("heartbeat-daily"), 86400).await;
}

// ─────────────────────────────────────────────────────────────────────────────
// HELPERS
// ─────────────────────────────────────────────────────────────────────────────

fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{secs} seconds")
    } else if secs < 3600 {
        format!("{} min {} sec", secs / 60, secs % 60)
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        format!("{h} hr {m} min")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TESTS
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_name_resolves_known_tokens() {
        assert_eq!(token_name("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"), "USDC");
        assert_eq!(token_name("0x4200000000000000000000000000000000000006"), "WETH");
        assert_eq!(token_name("0xcbB7C0000aB88B473b1f5aFd9ef808440eed33Bf"), "cbBTC");
    }

    #[test]
    fn token_name_shortens_unknown() {
        let result = token_name("0xDEADBEEF00000000000000000000000000001234");
        assert!(result.contains("..."), "Unknown should be shortened: {result}");
    }

    #[test]
    fn explain_error_translates_tx_errors() {
        assert!(explain_error("nonce too low").contains("Nonce conflict"));
        assert!(explain_error("insufficient funds").contains("enough ETH"));
    }

    #[test]
    fn explain_error_handles_unknown() {
        let r = explain_error("some completely unknown error XYZ");
        assert!(r.starts_with("Unexpected error:"));
    }

    #[test]
    fn fmt_boot_is_clean() {
        let msg = fmt_boot(
            "DRY_RUN",
            "0x60d0C491dF2d35E4C95D98dF37897f908b04b46f",
            "0x3011BfD673a9D09f9761203A7fFCca757Af22587",
        );
        assert!(msg.contains("Bot Online"));
        assert!(msg.contains("DRY_RUN"));
        assert!(msg.contains("0x60d0...b46f"));
        assert!(msg.contains("Base Mainnet"));
    }

    #[test]
    fn throttle_works() {
        assert!(throttle_guard("test_skel_key", Duration::from_secs(60)));
        assert!(!throttle_guard("test_skel_key", Duration::from_secs(60)));
        assert!(throttle_guard("different_skel_key", Duration::from_secs(60)));
    }
}
