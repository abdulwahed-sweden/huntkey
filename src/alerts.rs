/// alerts.rs — Telegram notification module.
///
/// Ported from: Bitcoin-Sentinel/eth_forensics/simulation/scripts/telegram.js
/// Format: clean structured text with HTML tags, no emoji.
/// Same formatter names, same field logic, same throttle/dedupe semantics.

use eyre::Result;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::warn;

// ── Separator line ──────────────────────────────────────────────────────────
const SEP: &str = "─────────────────────────────────";

// ── Dedupe state ────────────────────────────────────────────────────────────
static LAST_SENT: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

fn dedupe_guard(key: &str, throttle: Duration) -> bool {
    let mut guard = LAST_SENT.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    if let Some(last) = map.get(key) {
        if last.elapsed() < throttle {
            return false; // still throttled
        }
    }
    map.insert(key.to_string(), Instant::now());
    true
}

// ── Core send ───────────────────────────────────────────────────────────────

/// Send a Telegram message. Silently skips if credentials are not set.
///
/// * `dedupe_key` — if provided, suppress duplicates for `throttle_secs`
/// * `force`      — bypass deduplication (e.g. profit events)
pub async fn send_telegram(
    text:          impl Into<String>,
    dedupe_key:    Option<&str>,
    throttle_secs: u64,
    force:         bool,
) -> Result<()> {
    let token   = std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default();
    let chat_id = std::env::var("TELEGRAM_CHAT_ID").unwrap_or_default();
    if token.is_empty() || chat_id.is_empty() { return Ok(()); }

    if let Some(key) = dedupe_key {
        if !force && !dedupe_guard(key, Duration::from_secs(throttle_secs)) {
            return Ok(());
        }
    }

    let mut message = text.into();
    if message.len() > 4000 {
        message.truncate(3997);
        message.push_str("…");
    }

    let client = Client::new();
    let url    = format!("https://api.telegram.org/bot{token}/sendMessage");

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
            Ok(r) => {
                warn!("Telegram API error (attempt {}): {}", attempt + 1, r.status());
            }
            Err(e) => {
                warn!("Telegram request error (attempt {}): {}", attempt + 1, e);
            }
        }

        if attempt < 2 {
            tokio::time::sleep(Duration::from_secs(1u64 << attempt)).await;
        }
    }
    Ok(()) // fail silently — never crash the main process
}

// ── Format helpers ──────────────────────────────────────────────────────────

pub fn short_addr(addr: &str) -> String {
    if addr.len() < 10 { return addr.to_string(); }
    format!("{}…{}", &addr[..6], &addr[addr.len()-4..])
}

fn fmt_usd(n: f64) -> String {
    format!("${:.0}", n)
}

fn f(label: &str) -> String {
    format!("{:<8}", label)
}

// ── Formatters — mirroring telegram.js ──────────────────────────────────────

/// CASE A — Critical target locked for monitoring / pre-strike.
pub fn fmt_critical(addr: &str, hf: f64, debt_usd: f64, est_profit_usd: f64, eta: Option<&str>, tier: &str) -> String {
    let drop_pct = if hf > 0.0 { (1.0 - 1.0 / hf) * 100.0 } else { 0.0 };
    let header = match tier {
        "STRIKE"   => "[ STRIKE ZONE — LIQUIDATION IMMINENT ]",
        "CRITICAL" => "[ CRITICAL TARGET — LOCKED ]",
        _          => "[ HOT TARGET — ENTERING RANGE ]",
    };
    let status = match tier {
        "STRIKE"   => "EXECUTING NOW",
        "CRITICAL" => "pre-strike monitoring",
        _          => "warming up",
    };
    let eta_line = eta.map(|e| format!("\n{}    <b>{}</b>", f("ETA"), e)).unwrap_or_default();
    format!(
        "<b>{header}</b>\n{SEP}\n\
         {}    <code>{addr}</code>\n\
         {}      <b>{:.4}</b>  needs {:.1}% drop to liquidate\n\
         {}    <b>{}</b>\n\
         {}  <b>+{}</b> est. net{eta_line}\n\
         {}  {status}",
        f("Addr"), f("HF"), hf, drop_pct,
        f("Debt"), fmt_usd(debt_usd),
        f("Profit"), fmt_usd(est_profit_usd),
        f("Status"),
    )
}

/// CASE B — Kill shot incoming: all gates passed, tx being submitted.
pub fn fmt_strike(addr: &str, hf: f64, route: &str, bribe_eth: f64, bribe_usd: f64, max_fee_gwei: f64, max_pri_gwei: f64) -> String {
    format!(
        "<b>[ KILL SHOT — BROADCASTING ]</b>\n{SEP}\n\
         {}  <code>{addr}</code>\n\
         {}      <b>{:.4}</b>  LIQUIDATABLE\n\
         {}   <b>{route}</b>\n\
         {}   <b>{:.6} ETH</b>  ({})\n\
         {}     {:.3} / {:.3} gwei\n\
         {}     PASSED — tx submitted",
        f("Target"), f("HF"), hf,
        f("Route"),
        f("Bribe"), bribe_eth, fmt_usd(bribe_usd),
        f("Gas"), max_fee_gwei, max_pri_gwei,
        f("Sim"),
    )
}

/// CASE C — Kill shot confirmed: on-chain profit secured.
pub fn fmt_profit(
    net_eth: f64, usd: f64, tx_hash: &str, block_num: u64,
    target_addr: &str, gas_eth: f64, bribe_eth: f64,
) -> String {
    let tx_link  = format!("https://basescan.org/tx/{tx_hash}");
    let gross    = net_eth + gas_eth + bribe_eth;
    format!(
        "<b>[ PROFIT SECURED ]</b>\n{SEP}\n\
         {}     <b>+{}  ({:.6} ETH)</b>\n\
         {}   {:.6} ETH\n\
         {}   {:.6} ETH\n\
         {}     {:.6} ETH\n\
         {}  <code>{target_addr}</code>\n\
         {}   #{block_num}\n\
         {}      <a href=\"{tx_link}\">{}</a>",
        f("Net"), fmt_usd(usd), net_eth,
        f("Gross"), gross,
        f("Bribe"), bribe_eth,
        f("Gas"), gas_eth,
        f("Target"),
        f("Block"),
        f("Tx"), short_addr(tx_hash),
    )
}

/// CASE D — Failed attempt.
pub fn fmt_failed(reason: &str, addr: &str, attempt: u8, max_attempts: u8, next_action: &str) -> String {
    let cooldown = if next_action == "evicted" { String::new() } else { format!("\n{}5 min", f("Cooldown")) };
    format!(
        "<b>[ FAILED — ATTEMPT {attempt}/{max_attempts} ]</b>\n{SEP}\n\
         {}  <b>{reason}</b>\n\
         {}  <code>{addr}</code>\n\
         {}    {next_action}{cooldown}",
        f("Reason"), f("Target"), f("Next"),
    )
}

/// Market regime change alert.
pub fn fmt_regime(regime: &str, eth_usd: f64, pct_change: f64) -> String {
    let sign   = if pct_change >= 0.0 { "+" } else { "" };
    let bribes = match regime {
        "CRASH"    => "MAXIMUM  (90%)",
        "VOLATILE" => "HIGH     (78%)",
        _          => "STANDARD (62%)",
    };
    let mode = match regime {
        "CRASH"    => "max aggression — all positions in scope",
        "VOLATILE" => "elevated aggression",
        _          => "standard operation",
    };
    format!(
        "<b>[ REGIME CHANGE — {regime} ]</b>\n{SEP}\n\
         {}     <b>{}</b>  ({}{:.2}% / 5 min)\n\
         {}   {bribes}\n\
         {}    {mode}",
        f("ETH"), fmt_usd(eth_usd), sign, pct_change * 100.0,
        f("Bribe"), f("Mode"),
    )
}

/// PARALLEL ATTACK — variant tx submitted to mempool.
pub fn fmt_shot_fired(label: &str, hf: f64, route: &str, tier: &str, idx: usize, total: usize) -> String {
    let plural = if total > 1 { "S" } else { "" };
    format!(
        "<b>[ SHOT FIRED — {idx}/{total} VARIANT{plural} ]</b>\n{SEP}\n\
         {} <b>{label}</b>\n\
         {}      <b>{:.4}</b>\n\
         {}   {route}\n\
         {}    {tier}",
        f("Variant"), f("HF"), hf, f("Route"), f("Tier"),
    )
}

/// PARALLEL ATTACK — fee escalation on unconfirmed tx.
pub fn fmt_escalate(round: u32, nonce: u64, new_fee_gwei: f64) -> String {
    format!(
        "<b>[ ESCALATING — ROUND {round} ]</b>\n{SEP}\n\
         {}   <b>{nonce}</b>\n\
         {} <b>{:.3} gwei</b>  (+15%)\n\
         {}  replacing unconfirmed tx",
        f("Nonce"), f("New Fee"), new_fee_gwei, f("Action"),
    )
}

/// PARALLEL ATTACK — variant aborted.
pub fn fmt_aborted(label: &str, reason: &str) -> String {
    let category = if label.starts_with("SIM_FAIL")  { "SIM REVERT" }
                   else if label.starts_with("CAP_FAIL")  { "CAPITAL INSUFFICIENT" }
                   else if label.starts_with("SEND_FAIL") { "BROADCAST FAILURE" }
                   else { "ABORTED" };
    let trimmed = &reason[..reason.len().min(200)];
    format!(
        "<b>[ {category} — {label} ]</b>\n{SEP}\n\
         {}  <b>{trimmed}</b>",
        f("Reason"),
    )
}
