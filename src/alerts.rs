//! HuntLoan Telegram notification module — high-signal only.
//!
//! Alert classes:
//!
//!   A) EXECUTED    — live tx confirmed on-chain (success or revert)
//!   B) FAILED      — execution attempt failed (send error / revert)
//!   C) CIRCUIT     — engine stopped by circuit breaker
//!   D) SUMMARY     — hourly/daily operational summary
//!   E) OPPORTUNITY — best candidate locked in, about to execute
//!   F) APPROACHING — warm-zone borrower ETA < 10 min
//!
//! NO per-block alerts. NO simulation pass/fail spam.
//! Rate-limited per category, deduplicated per target, silent on missing creds.

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use reqwest::Client;
use tracing::warn;
use eyre::Result;

// ── Constants ────────────────────────────────────────────────────────────────
const SEP: &str = "─────────────────────────────────";

// ── Shared dedup state (per-key and per-category) ────────────────────────────
static ALERT_STATE: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

/// Returns true and records the send time if the key is not throttled.
fn throttle_guard(key: &str, limit: Duration) -> bool {
    if limit.is_zero() { return true; }
    let mut guard = ALERT_STATE.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    if map.get(key).is_some_and(|last| last.elapsed() < limit) {
        return false; // still throttled
    }
    map.insert(key.to_string(), Instant::now());
    true
}

// ── Global session stats ─────────────────────────────────────────────────────

/// Accumulates session-level metrics for the hourly summary.
/// Thread-safe via atomics + single mutex for the revert-reason map.
pub struct AlertStats {
    pub blocks_processed:  AtomicU64,
    pub opps_detected:     AtomicU64,
    pub sims_passed:       AtomicU64,
    pub execs_attempted:   AtomicU64,
    pub execs_succeeded:   AtomicU64,
    /// Total gas cost in gwei (sum of gas_used × base_fee for each confirmed tx).
    pub gas_cost_gwei:     AtomicU64,
    /// Estimated net profit in USD cents (×100) to avoid floats in atomics.
    pub net_profit_cents:  AtomicU64,
    /// Revert reason frequencies (top-N displayed in summary).
    pub revert_reasons:    Mutex<HashMap<String, u64>>,
    pub session_start:     Instant,
}

impl AlertStats {
    pub fn record_revert(&self, reason: &str) {
        let key = reason.chars().take(60).collect::<String>();
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

/// Returns the global session stats singleton.
pub fn get_stats() -> &'static AlertStats {
    STATS.get_or_init(|| AlertStats {
        blocks_processed: AtomicU64::new(0),
        opps_detected:    AtomicU64::new(0),
        sims_passed:      AtomicU64::new(0),
        execs_attempted:  AtomicU64::new(0),
        execs_succeeded:  AtomicU64::new(0),
        gas_cost_gwei:    AtomicU64::new(0),
        net_profit_cents:  AtomicU64::new(0),
        revert_reasons:   Mutex::new(HashMap::new()),
        session_start:    Instant::now(),
    })
}

// ── Core send ────────────────────────────────────────────────────────────────

/// Send a Telegram message. Silently skips if credentials are not set.
///
/// * `dedupe_key`  — suppress duplicates for `throttle_secs` (None = always send)
/// * `throttle_secs` — 0 = bypass throttle (force-send)
pub async fn send_telegram(
    text:          impl Into<String>,
    dedupe_key:    Option<&str>,
    throttle_secs: u64,
) -> Result<()> {
    let token   = std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default();
    let chat_id = std::env::var("TELEGRAM_CHAT_ID").unwrap_or_default();
    if token.is_empty() || chat_id.is_empty() { return Ok(()); }

    if let Some(key) = dedupe_key && !throttle_guard(key, Duration::from_secs(throttle_secs)) {
        return Ok(());
    }

    let mut message = text.into();
    if message.len() > 4000 {
        message.truncate(3997);
        message.push('…');
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
            Ok(r) => warn!("Telegram API error (attempt {}): {}", attempt + 1, r.status()),
            Err(e) => warn!("Telegram request error (attempt {}): {}", attempt + 1, e),
        }

        if attempt < 2 {
            tokio::time::sleep(Duration::from_secs(1u64 << attempt)).await;
        }
    }
    Ok(()) // fail silently — never crash the main process
}

// ── Format helpers ───────────────────────────────────────────────────────────

pub fn short_addr(addr: &str) -> String {
    if addr.len() < 10 { return addr.to_string(); }
    format!("{}…{}", &addr[..6], &addr[addr.len()-4..])
}

fn fmt_usd(n: f64) -> String { format!("${:.0}", n) }
fn f(label: &str) -> String  { format!("{:<8}", label) }

fn uptime_str(secs: u64) -> String {
    if secs < 60        { format!("{secs}s") }
    else if secs < 3600 { format!("{}m", secs / 60) }
    else                { format!("{}h {}m", secs / 3600, (secs % 3600) / 60) }
}

// ── CLASS A: EXECUTED ────────────────────────────────────────────────────────

/// Fired once per confirmed tx (status=1 success or status=0 revert on-chain).
#[allow(clippy::too_many_arguments)]
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
    let status_label = if status == 1 { "✅ CONFIRMED" } else { "❌ REVERTED ON-CHAIN" };
    let gas_eth  = gas_used as f64 * base_fee_wei as f64 / 1e18;
    let gas_usd  = gas_eth * eth_price_usd as f64;
    let net_usd  = sim_profit_usd as f64;
    let tx_link  = format!("https://basescan.org/tx/{tx_hash}");
    let target_s = short_addr(borrower);
    let tx_s     = short_addr(tx_hash);
    let debt_s   = fmt_usd(debt_usd as f64);
    let pnl_s    = fmt_usd(net_usd);
    let gas_s    = fmt_usd(gas_usd);
    let f_status = f("Status");
    let f_target = f("Target");
    let f_hf     = f("HF");
    let f_route  = f("Route");
    let f_debt   = f("Debt");
    let f_profit = f("Profit");
    let f_gas    = f("Gas");
    let f_block  = f("Block");
    let f_tx     = f("Tx");
    format!(
        "🐺🔥 <b>ضربة الذئاب — LIVE</b>\n{SEP}\n\
         {f_status}  {status_label}\n\
         {f_target}  <code>{target_s}</code>\n\
         {f_hf}   <b>{hf:.4}</b>\n\
         {f_route}  {collateral} → {debt_asset}\n\
         {f_debt}   <b>{debt_s}</b>  debt repaid\n\
         {f_profit}  <b>+{pnl_s}</b>  est. net\n\
         {f_gas}   {gas_eth:.4} ETH  ({gas_s})\n\
         {f_block}   #{block_num}\n\
         {f_tx}      <a href=\"{tx_link}\">{tx_s}</a>"
    )
}

// ── CLASS B: FAILED EXECUTION ────────────────────────────────────────────────

/// Fired when execution attempt fails (send error, not a confirmed on-chain revert).
pub fn fmt_failed_exec(reason: &str, borrower: &str, hint: &str) -> String {
    let reason_short = &reason[..reason.len().min(200)];
    format!(
        "🐺❌ <b>فشل الضربة</b>\n{SEP}\n\
         {}  <b>{reason_short}</b>\n\
         {}  <code>{borrower}</code>\n\
         {}  {hint}",
        f("Reason"), f("Target"), f("Hint"),
    )
}

// ── CLASS C: CIRCUIT BREAKER ─────────────────────────────────────────────────

/// Fired when the circuit breaker stops the engine.
pub fn fmt_circuit_breaker(trigger: &str, detail: &str) -> String {
    format!(
        "🚨 <b>توقف للحماية — CIRCUIT BREAKER</b>\n{SEP}\n\
         {}  <b>{trigger}</b>\n\
         {}  {detail}\n\
         {}  restart bot + check logs",
        f("Trigger"), f("Detail"), f("Action"),
    )
}

// ── CLASS D: SUMMARY ─────────────────────────────────────────────────────────

/// Fired on a configurable interval (default: hourly).
#[allow(clippy::too_many_arguments)]
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
    let win_rate = if execs_tried > 0 {
        format!("{:.0}%", execs_ok as f64 / execs_tried as f64 * 100.0)
    } else {
        "—".to_string()
    };

    let revert_lines = if top_reverts.is_empty() {
        "  none".to_string()
    } else {
        top_reverts.iter()
            .map(|(r, c)| format!("  • {c}× {r}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let uptime_s   = uptime_str(uptime_secs);
    let pnl_s      = fmt_usd(net_profit_usd);
    let gas_s      = fmt_usd(gas_cost_usd);
    let f_uptime   = f("Uptime");
    let f_blocks   = f("Blocks");
    let f_execs    = f("Execs");
    let f_pnl      = f("Net PnL");
    let f_toperr   = f("Top err");
    format!(
        "📊 <b>ملخص الذئاب</b>\n{SEP}\n\
         {f_uptime}  {uptime_s}\n\
         {f_blocks}   {blocks}  |  opps: {opps}  |  sims OK: {sims_passed}\n\
         {f_execs}    {execs_tried}  →  {execs_ok} confirmed  ({win_rate})\n\
         {f_pnl}  +{pnl_s}  (gas cost: {gas_s})\n\
         {f_toperr}  (see below)\n\
         {SEP}\n{revert_lines}"
    )
}

// ── CLASS E: BOOT ────────────────────────────────────────────────────────────

/// Formatted boot message — call via send_telegram() after config is loaded.
pub fn fmt_boot(mode: &str, contract: &str, operator: &str) -> String {
    format!(
        "🐺🚀 <b>HUNTLOAN — ONLINE</b>\n{SEP}\n\
         {}  <b>{mode}</b>\n\
         {}  <code>{}</code>\n\
         {}  <code>{}</code>\n\
         {}  Base (8453)",
        f("Mode"),
        f("Contract"), short_addr(contract),
        f("Operator"), short_addr(operator),
        f("Chain"),
    )
}

// ── CLASS E: OPPORTUNITY ─────────────────────────────────────────────────────

/// Fired just before execution — target locked, simulation passed.
/// Throttled 30 s per borrower to avoid spam on fast blocks.
pub async fn send_opportunity(
    borrower:   &str,
    hf:         f64,
    debt_usd:   u128,
    collateral: &str,
    debt_asset: &str,
    profit_usd: i128,
    score_val:  f64,
) {
    let msg = format!(
        "🎯 <b>TARGET LOCKED</b>\n{SEP}\n\
         {}  <code>{}</code>\n\
         {}  <b>{hf:.4}</b>\n\
         {}  {collateral} → {debt_asset}\n\
         {}  <b>{}</b>\n\
         {}  <b>+{}</b>  est.\n\
         {}  {score_val:.2}",
        f("Borrower"), short_addr(borrower),
        f("HF"),
        f("Route"),
        f("Debt"),   fmt_usd(debt_usd as f64),
        f("Profit"), fmt_usd(profit_usd as f64),
        f("Score"),
    );
    let key = format!("opp-{}", short_addr(borrower));
    let _ = send_telegram(msg, Some(&key), 30).await;
}

// ── CLASS F: APPROACHING ─────────────────────────────────────────────────────

/// Fired when a warm-zone borrower is < 10 min from HF = 1.0.
/// Default throttle 120 s to avoid alert fatigue on slow declines.
pub async fn send_approaching(borrower: &str, hf: f64, eta_min: f64, throttle_secs: u64) {
    let msg = format!(
        "⏳ <b>APPROACHING — ETA {eta_min:.1} min</b>\n{SEP}\n\
         {}  <code>{}</code>\n\
         {}  <b>{hf:.4}</b>",
        f("Borrower"), short_addr(borrower),
        f("HF"),
    );
    let key = format!("approach-{}", short_addr(borrower));
    let ts  = if throttle_secs == 0 { 120 } else { throttle_secs };
    let _ = send_telegram(msg, Some(&key), ts).await;
}

// ── Raw send (explicit credentials) ─────────────────────────────────────────

/// Send a pre-formatted message using explicit token + chat_id.
/// Used at startup before config is available. Never panics.
#[allow(dead_code)]
pub async fn send_telegram_raw(token: &str, chat_id: &str, text: &str) {
    let client = Client::new();
    let url    = format!("https://api.telegram.org/bot{token}/sendMessage");
    let body   = serde_json::json!({
        "chat_id":                  chat_id,
        "text":                     &text[..text.len().min(4000)],
        "parse_mode":               "HTML",
        "disable_web_page_preview": true,
    });
    if let Err(e) = client.post(&url).json(&body).send().await {
        warn!("Telegram boot alert failed: {e}");
    }
}

// ── Legacy formatters (kept for backward compatibility, not called by engine) ─

#[allow(dead_code)]
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

#[allow(dead_code)]
pub fn fmt_profit(net_eth: f64, usd: f64, tx_hash: &str, block_num: u64, target_addr: &str, gas_eth: f64, bribe_eth: f64) -> String {
    let tx_link = format!("https://basescan.org/tx/{tx_hash}");
    let gross   = net_eth + gas_eth + bribe_eth;
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

#[allow(dead_code)]
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
