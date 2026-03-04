/// HuntLoan Telegram alerts — v3 complete rewrite.
///
/// Design principles:
///   - Clear English (no abbreviations like "HF" without context)
///   - Every icon matches the event meaning
///   - Smart address shortening (symbols for known tokens, 0xAB…CD for unknown)
///   - Timestamp on every message
///   - No raw hex dumps or error bytes — human-readable reasons
///   - Clean vertical layout optimized for mobile Telegram
///
/// Alert classes:
///   🟢 BOOT          — bot started successfully
///   💰 LIQUIDATION    — tx confirmed on-chain (profit or loss)
///   ❌ EXECUTION FAILED — tx failed to send or reverted
///   🚨 EMERGENCY STOP  — circuit breaker triggered
///   📊 STATUS REPORT   — periodic operational summary
///   🎯 TARGET LOCKED   — profitable opportunity found, executing now
///   ⏳ TARGET APPROACHING — warm position nearing liquidation threshold

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::Client;
use tracing::warn;
use eyre::Result;

// ─────────────────────────────────────────────────────────────────────────────
// CONSTANTS
// ─────────────────────────────────────────────────────────────────────────────

const LINE: &str = "────────────────────────────";

// ─────────────────────────────────────────────────────────────────────────────
// TIMESTAMP
// ─────────────────────────────────────────────────────────────────────────────

/// Returns "14:32:05 UTC" — compact, clear.
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

/// Shorten any hex address to "0xABCD…1234" (10 chars).
pub fn short_addr(addr: &str) -> String {
    if addr.len() < 12 { return addr.to_string(); }
    format!("{}…{}", &addr[..6], &addr[addr.len()-4..])
}

/// Resolve a Base mainnet token address to its ticker symbol.
/// Known tokens get clean names; unknown tokens get shortened hex.
pub fn token_name(addr: &str) -> String {
    let a = addr.to_lowercase();
    match a.as_str() {
        // ETH family
        s if s.ends_with("0000000000000000000006")                     => "WETH".into(),
        s if s.contains("2ae3f1ec7f1f5012cfeab0185bfc7aa3cf0dec22")    => "cbETH".into(),
        s if s.contains("c1cba3fcea344f92d9239c08c0568f6f2f0ee452")    => "wstETH".into(),
        s if s.contains("04c0599ae5a44757c0af6f9ec3b93da8976c150a")    => "weETH".into(),
        s if s.contains("b6fe221fe9eef5aba221c348ba20a1bf5e73624c")    => "rETH".into(),
        // Stablecoins
        s if s.contains("833589fcd6edb6e08f4c7c32d4f71b54bda02913")    => "USDC".into(),
        s if s.contains("d9aaec86b65d86f6a7b5b1b0c42ffa531710b6ca")    => "USDbC".into(),
        s if s.contains("50c5725949a6f0c72e6c4a641f24049a917db0cb")    => "DAI".into(),
        s if s.contains("4a3a6dd60a34bb2aba60d73b4c88315e9ceb6a3d")    => "USDT".into(),
        // BTC family
        s if s.contains("cbb7c0000ab88b473b1f5afd9ef808440eed33bf")    => "cbBTC".into(),
        s if s.contains("2260fac5e5542a773aa44fbcfedf7c193bc2c599")    => "WBTC".into(),
        // Unknown
        _ => short_addr(addr),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// HUMAN-READABLE ERROR DECODER
// ─────────────────────────────────────────────────────────────────────────────

/// Translate raw revert data into plain English a human can act on.
pub fn explain_error(raw: &str) -> String {
    // Aave V3 errors
    if raw.contains("HEALTH_FACTOR_NOT_BELOW_THRESHOLD") || raw.contains("0x35") {
        return "The position is no longer underwater — another bot liquidated it first.".into();
    }
    if raw.contains("SPECIFIED_CURRENCY_NOT_BORROWED_BY_USER") || raw.contains("0x40") {
        return "The borrower does not owe this token — wrong debt asset selected.".into();
    }
    if raw.contains("COLLATERAL_CANNOT_BE_LIQUIDATED") {
        return "This collateral type cannot be liquidated on Aave V3.".into();
    }

    // HuntLoan contract errors
    if raw.contains("OnlyOperator") {
        return "Access denied — the caller is not the authorized operator.".into();
    }
    if raw.contains("ContractSettled") {
        return "The contract has already been settled and closed.".into();
    }
    if raw.contains("SwapFailed") {
        return "All DEX routes failed — no liquidity path found for this token pair.".into();
    }
    if raw.contains("LiquidationUnprofitable") {
        return "The swap returned less than the flash loan owed — trade would be a loss.".into();
    }

    // ERC20 / transfer errors
    if raw.contains("STF") || raw.contains("SafeERC20") || raw.contains("TransferFailed") {
        return "Token transfer failed — likely an approval or balance issue.".into();
    }

    // Transaction-level errors
    if raw.contains("nonce too low") {
        return "Nonce conflict — a previous transaction was confirmed out of order.".into();
    }
    if raw.contains("replacement transaction underpriced") {
        return "Gas price too low to replace the pending transaction.".into();
    }
    if raw.contains("insufficient funds") {
        return "Wallet does not have enough ETH to cover gas fees.".into();
    }
    if raw.contains("timeout") || raw.contains("Timeout") {
        return "RPC request timed out — network may be congested.".into();
    }

    // Fallback: truncate raw error to something readable
    let clean: String = raw.chars().take(150).collect();
    format!("Unexpected error: {clean}")
}

// ─────────────────────────────────────────────────────────────────────────────
// SESSION STATISTICS
// ─────────────────────────────────────────────────────────────────────────────

pub struct AlertStats {
    pub blocks_processed: AtomicU64,
    pub opps_detected:    AtomicU64,
    pub sims_passed:      AtomicU64,
    pub execs_attempted:  AtomicU64,
    pub execs_succeeded:  AtomicU64,
    pub gas_cost_gwei:    AtomicU64,
    pub net_profit_cents: AtomicU64,
    pub revert_reasons:   Mutex<HashMap<String, u64>>,
    pub session_start:    Instant,
}

impl AlertStats {
    pub fn record_revert(&self, reason: &str) {
        let explained = explain_error(reason);
        let key: String = explained.chars().take(100).collect();
        let mut map = self.revert_reasons.lock().unwrap();
        *map.entry(key).or_insert(0) += 1;
    }

    pub fn top_reverts(&self, n: usize) -> Vec<(String, u64)> {
        let map = self.revert_reasons.lock().unwrap();
        let mut v: Vec<_> = map.iter().map(|(k, &c)| (k.clone(), c)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        v.truncate(n);
        v
    }
}

static STATS: OnceLock<AlertStats> = OnceLock::new();

pub fn get_stats() -> &'static AlertStats {
    STATS.get_or_init(|| AlertStats {
        blocks_processed: AtomicU64::new(0),
        opps_detected:    AtomicU64::new(0),
        sims_passed:      AtomicU64::new(0),
        execs_attempted:  AtomicU64::new(0),
        execs_succeeded:  AtomicU64::new(0),
        gas_cost_gwei:    AtomicU64::new(0),
        net_profit_cents: AtomicU64::new(0),
        revert_reasons:   Mutex::new(HashMap::new()),
        session_start:    Instant::now(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// RATE LIMITER
// ─────────────────────────────────────────────────────────────────────────────

static ALERT_STATE: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

fn throttle_guard(key: &str, limit: Duration) -> bool {
    if limit.is_zero() { return true; }
    let mut guard = ALERT_STATE.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    if let Some(last) = map.get(key) {
        if last.elapsed() < limit {
            return false;
        }
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

    if let Some(key) = dedupe_key {
        if !throttle_guard(key, Duration::from_secs(throttle_secs)) {
            return Ok(());
        }
    }

    let mut message = text.into();
    if message.len() > 4000 {
        message.truncate(3997);
        message.push_str("…");
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
//  🟢 BOOT — Bot started
// ═════════════════════════════════════════════════════════════════════════════

pub fn fmt_boot(mode: &str, contract: &str, operator: &str) -> String {
    let t = timestamp();
    format!(
        "🟢 <b>Bot Online</b>\n\
         {LINE}\n\
         🕐  {t}\n\
         ⚙️  Mode: <b>{mode}</b>\n\
         📜  Contract: <code>{}</code>\n\
         👤  Operator: <code>{}</code>\n\
         ⛓️  Chain: Base Mainnet (8453)",
        short_addr(contract),
        short_addr(operator),
    )
}

// ═════════════════════════════════════════════════════════════════════════════
//  💰 PROFIT CAUGHT — Transaction confirmed on-chain
// ═════════════════════════════════════════════════════════════════════════════

pub fn fmt_executed(
    borrower:       &str,
    hf:             f64,
    debt_usd:       u128,
    collateral:     &str,
    debt_asset:     &str,
    sim_profit_usd: i128,
    gas_used:       u64,
    base_fee_wei:   u128,
    eth_price_usd:  u128,
    tx_hash:        &str,
    block_num:      u64,
    status:         u8,
) -> String {
    let t = timestamp();
    let coll = token_name(collateral);
    let debt = token_name(debt_asset);
    let gas_eth = gas_used as f64 * base_fee_wei as f64 / 1e18;
    let gas_usd = gas_eth * eth_price_usd as f64;
    let tx_link = format!("https://basescan.org/tx/{tx_hash}");

    if status == 1 {
        format!(
            "💰 <b>PROFIT CAUGHT!</b>\n\
             {LINE}\n\
             🕐  {t}  ·  Block #{block_num}\n\
             \n\
             Amount: <b>${sim_profit_usd}</b> USD\n\
             TX: <code>{tx_hash}</code>\n\
             Contract: <code>{}</code>\n\
             \n\
             Route: {coll} → {debt}  ·  Debt: ${debt_usd}\n\
             HF: {hf:.4}  ·  Gas: {gas_eth:.5} ETH (${gas_usd:.2})\n\
             🔗 <a href=\"{tx_link}\">BaseScan</a>",
            short_addr(borrower),
        )
    } else {
        format!(
            "⚠️ <b>ATTEMPT FAILED</b>\n\
             {LINE}\n\
             🕐  {t}  ·  Block #{block_num}\n\
             \n\
             Reason: Transaction reverted on-chain\n\
             Potential Loss: {gas_eth:.5} ETH (${gas_usd:.2})\n\
             TX: <code>{tx_hash}</code>\n\
             \n\
             Borrower: <code>{}</code>  ·  Route: {coll} → {debt}",
            short_addr(borrower),
        )
    }
}

// ═════════════════════════════════════════════════════════════════════════════
//  ⚠️ ATTEMPT FAILED — Transaction could not be sent or reverted
// ═════════════════════════════════════════════════════════════════════════════

pub fn fmt_failed_exec(reason: &str, borrower: &str, hint: &str) -> String {
    let t = timestamp();
    let explained = explain_error(reason);

    format!(
        "⚠️ <b>ATTEMPT FAILED</b>\n\
         {LINE}\n\
         🕐  {t}\n\
         \n\
         Reason: {explained}\n\
         Borrower: <code>{}</code>\n\
         Suggestion: {hint}",
        short_addr(borrower),
    )
}

// ═════════════════════════════════════════════════════════════════════════════
//  🚨 EMERGENCY STOP — Circuit breaker activated
// ═════════════════════════════════════════════════════════════════════════════

pub fn fmt_circuit_breaker(trigger: &str, detail: &str) -> String {
    let t = timestamp();
    let detail_clean: String = detail.chars().take(200).collect();

    format!(
        "🚨 <b>Emergency Stop — Circuit Breaker</b>\n\
         {LINE}\n\
         🕐  {t}\n\
         \n\
         💥  Trigger: <b>{trigger}</b>\n\
         📋  Detail: {detail_clean}\n\
         \n\
         🔧  Action required: Check logs and restart the bot manually."
    )
}

// ═════════════════════════════════════════════════════════════════════════════
//  📊 STATUS REPORT — Periodic operational summary
// ═════════════════════════════════════════════════════════════════════════════

pub fn fmt_summary(
    uptime_secs:    u64,
    blocks:         u64,
    opps:           u64,
    sims_passed:    u64,
    execs_tried:    u64,
    execs_ok:       u64,
    gas_cost_usd:   f64,
    net_profit_usd: f64,
    top_reverts:    &[(String, u64)],
) -> String {
    let t = timestamp();
    let uptime = format_uptime(uptime_secs);

    let win_rate = if execs_tried > 0 {
        format!("{:.0}%", execs_ok as f64 / execs_tried as f64 * 100.0)
    } else {
        "N/A".into()
    };

    let scan_rate = if blocks > 0 {
        format!("{:.2}%", opps as f64 / blocks as f64 * 100.0)
    } else {
        "0%".into()
    };

    let sim_rate = if opps > 0 {
        format!("{:.0}%", sims_passed as f64 / opps as f64 * 100.0)
    } else {
        "N/A".into()
    };

    let pnl = if net_profit_usd >= 0.0 {
        format!("+${net_profit_usd:.2}")
    } else {
        format!("-${:.2}", net_profit_usd.abs())
    };

    let errors_section = if top_reverts.is_empty() {
        "  ✅ No errors recorded".into()
    } else {
        top_reverts.iter()
            .map(|(reason, count)| format!("  · {count}× — {reason}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "📊 <b>Status Report</b>\n\
         {LINE}\n\
         🕐  {t}  ·  Uptime: {uptime}\n\
         \n\
         <b>Pipeline:</b>\n\
         📡  {blocks} blocks scanned → {opps} opportunities ({scan_rate})\n\
         🧪  {opps} simulated → {sims_passed} profitable ({sim_rate})\n\
         ⚔️  {execs_tried} executed → {execs_ok} confirmed ({win_rate})\n\
         \n\
         <b>Financials:</b>\n\
         💵  Net profit: <b>{pnl}</b>\n\
         ⛽  Gas spent: ${gas_cost_usd:.2}\n\
         \n\
         <b>Top errors:</b>\n\
         {errors_section}"
    )
}

// ═════════════════════════════════════════════════════════════════════════════
//  🎯 TARGET LOCKED — Profitable opportunity, about to execute
// ═════════════════════════════════════════════════════════════════════════════

pub fn fmt_opportunity(
    borrower:       &str,
    hf:             f64,
    debt_usd:       u128,
    collateral:     &str,
    debt_asset:     &str,
    net_profit_usd: i128,
    score:          f64,
) -> String {
    let t = timestamp();
    let coll = token_name(collateral);
    let debt = token_name(debt_asset);

    format!(
        "🎯 <b>Target Locked — Executing Now</b>\n\
         {LINE}\n\
         🕐  {t}\n\
         \n\
         👤  Borrower: <code>{}</code>\n\
         ❤️  Health Factor: <b>{hf:.4}</b>\n\
         📊  Priority Score: {score:.1}\n\
         🔄  Route: {coll} → {debt}\n\
         💳  Debt: ${debt_usd}\n\
         💵  Expected profit: <b>+${net_profit_usd}</b>",
        short_addr(borrower),
    )
}

pub async fn send_opportunity(
    borrower: &str, hf: f64, debt_usd: u128,
    collateral: &str, debt_asset: &str,
    net_profit_usd: i128, score: f64,
) {
    let msg = fmt_opportunity(borrower, hf, debt_usd, collateral, debt_asset, net_profit_usd, score);
    let key = format!("opp-{}", &borrower[..borrower.len().min(10)]);
    let _ = send_telegram(msg, Some(&key), 120).await;
}

// ═════════════════════════════════════════════════════════════════════════════
//  ⏳ TARGET APPROACHING — Warm position nearing liquidation
// ═════════════════════════════════════════════════════════════════════════════

pub fn fmt_approaching(
    borrower:    &str,
    hf:          f64,
    eta_minutes: f64,
    debt_usd:    u128,
) -> String {
    let t = timestamp();

    let urgency = if eta_minutes < 3.0 {
        "🔴 Imminent"
    } else if eta_minutes < 10.0 {
        "🟠 Approaching"
    } else {
        "🟡 Watching"
    };

    format!(
        "⏳ <b>Target Approaching — {urgency}</b>\n\
         {LINE}\n\
         🕐  {t}\n\
         \n\
         👤  Borrower: <code>{}</code>\n\
         ❤️  Health Factor: <b>{hf:.4}</b>\n\
         ⏰  Estimated time to liquidation: <b>~{eta_minutes:.0} minutes</b>\n\
         💳  Debt at risk: ${debt_usd}",
        short_addr(borrower),
    )
}

pub async fn send_approaching(
    borrower: &str, hf: f64, eta_minutes: f64, debt_usd: u128,
) {
    let msg = fmt_approaching(borrower, hf, eta_minutes, debt_usd);
    let key = format!("approach-{}", &borrower[..borrower.len().min(10)]);
    let _ = send_telegram(msg, Some(&key), 300).await;
}

// ═════════════════════════════════════════════════════════════════════════════
//  🛑 LOW BALANCE — Wallet critically low
// ═════════════════════════════════════════════════════════════════════════════

pub fn fmt_low_balance(balance_eth: f64) -> String {
    format!(
        "🛑 <b>CRITICAL: LOW BALANCE</b>\n\
         {LINE}\n\
         \n\
         Current: <b>{balance_eth:.6} ETH</b>\n\
         Action: Please fund the operator wallet immediately."
    )
}

pub async fn send_low_balance(balance_eth: f64) {
    let msg = fmt_low_balance(balance_eth);
    // Throttle to once per hour — avoid spamming on every block
    let _ = send_telegram(msg, Some("low-balance"), 3600).await;
}

// ═════════════════════════════════════════════════════════════════════════════
//  🤖 HEARTBEAT — Daily proof-of-life
// ═════════════════════════════════════════════════════════════════════════════

pub fn fmt_heartbeat(candidates: usize) -> String {
    let t = timestamp();
    format!(
        "🤖 <b>HuntLoan Heartbeat</b>\n\
         {LINE}\n\
         🕐  {t}\n\
         \n\
         System is UP. Scanning {candidates}+ addresses.\n\
         Status: Hunting..."
    )
}

/// Send a daily heartbeat — throttled to once per 24 hours.
pub async fn send_heartbeat(candidates: usize) {
    let msg = fmt_heartbeat(candidates);
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
        assert_eq!(token_name("0xc1CBa3fCea344f92D9239c08C0568f6F2F0ee452"), "wstETH");
    }

    #[test]
    fn token_name_shortens_unknown() {
        let result = token_name("0xDEADBEEF00000000000000000000000000001234");
        assert!(result.contains("…"), "Unknown should be shortened: {result}");
    }

    #[test]
    fn explain_error_translates_aave_errors() {
        let r = explain_error("execution reverted: HEALTH_FACTOR_NOT_BELOW_THRESHOLD");
        assert!(r.contains("no longer underwater"));

        let r = explain_error("SwapFailed(0x123, 0x456, 1000, 500)");
        assert!(r.contains("DEX routes failed"));
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
    fn fmt_executed_shows_profit_caught() {
        let msg = fmt_executed(
            "0x1234567890abcdef1234567890abcdef12345678",
            0.9850, 50_000,
            "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", // USDC
            "0x4200000000000000000000000000000000000006",   // WETH
            250, 450_000, 5_000_000, 2_500,
            "0xABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD",
            12345678, 1,
        );
        assert!(msg.contains("PROFIT CAUGHT"), "Should show profit caught: {msg}");
        assert!(msg.contains("USDC → WETH"), "Should show token symbols: {msg}");
        assert!(msg.contains("$250"), "Should show amount");
        assert!(msg.contains("BaseScan"), "Should have explorer link");
    }

    #[test]
    fn fmt_summary_shows_pipeline_funnel() {
        let msg = fmt_summary(
            7261, 3600, 12, 5, 3, 2, 4.50, 180.0,
            &[("The position is no longer underwater".into(), 7)],
        );
        assert!(msg.contains("Status Report"));
        assert!(msg.contains("3600 blocks scanned"));
        assert!(msg.contains("12 opportunities"));
        assert!(msg.contains("2 confirmed"));
        assert!(msg.contains("+$180.00"));
        assert!(msg.contains("7×"));
    }

    #[test]
    fn fmt_boot_is_clean() {
        let msg = fmt_boot(
            "DRY_RUN",
            "0x0A0fE1f59D56716aF5c4C9D7688df742EE5949D3",
            "0x3011BfD673a9D09f9761203A7fFCca757Af22587",
        );
        assert!(msg.contains("Bot Online"));
        assert!(msg.contains("DRY_RUN"));
        assert!(msg.contains("0x0A0f…49D3"));  // shortened
        assert!(msg.contains("Base Mainnet"));
    }

    #[test]
    fn throttle_works() {
        assert!(throttle_guard("test_unique_key", Duration::from_secs(60)));
        assert!(!throttle_guard("test_unique_key", Duration::from_secs(60)));
        assert!(throttle_guard("different_key", Duration::from_secs(60)));
    }
}
