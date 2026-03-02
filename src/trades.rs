//! HuntLoan trade log — appends one CSV row per confirmed broadcast.
//!
//! File: logs/trades.csv (created if absent, directory auto-created).
//!
//! Columns:
//!   timestamp, tx_hash, target, debt_asset, collateral_asset,
//!   debt_usd, sim_net_profit_usd, estimated_gas, gas_used,
//!   base_fee_wei, bribe_wei, block_number, status,
//!   scan_ms, sim_ms, exec_ms
//!
//! Usage: call append_trade() from engine.rs after every confirmed receipt.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use tracing::warn;

/// All data needed for one row of the trade log.
pub struct TradeRecord<'a> {
    pub timestamp:          &'a str,   // ISO-8601 UTC
    pub tx_hash:            &'a str,
    pub target:             &'a str,
    pub debt_asset:         &'a str,
    pub collateral_asset:   &'a str,
    pub debt_usd:           u128,
    pub sim_net_profit_usd: i128,
    pub estimated_gas:      u64,
    pub gas_used:           u64,
    pub base_fee_wei:       u128,
    pub bribe_wei:          u128,
    pub block_number:       u64,
    pub status:             u8,   // 1 = success, 0 = failed
    pub scan_ms:            u128,
    pub sim_ms:             u128,
    pub exec_ms:            u128,
}

const CSV_HEADER: &str =
    "timestamp,tx_hash,target,debt_asset,collateral_asset,\
     debt_usd,sim_net_profit_usd,estimated_gas,gas_used,\
     base_fee_wei,bribe_wei,block_number,status,\
     scan_ms,sim_ms,exec_ms";

/// Append one trade record to logs/trades.csv.
/// Creates the logs/ directory and CSV header if needed.
/// Silently warns on I/O errors — never panics.
pub fn append_trade(r: &TradeRecord<'_>) {
    let dir  = Path::new("logs");
    let path = dir.join("trades.csv");

    if let Err(e) = std::fs::create_dir_all(dir) {
        warn!("trades.csv: cannot create logs/ dir: {e}");
        return;
    }

    let needs_header = !path.exists();

    let mut file = match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(f)  => f,
        Err(e) => { warn!("trades.csv: open error: {e}"); return; }
    };

    if needs_header && let Err(e) = writeln!(file, "{CSV_HEADER}") {
        warn!("trades.csv: header write error: {e}");
        return;
    }

    if let Err(e) = writeln!(
        file,
        "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
        r.timestamp, r.tx_hash, r.target, r.debt_asset, r.collateral_asset,
        r.debt_usd, r.sim_net_profit_usd, r.estimated_gas, r.gas_used,
        r.base_fee_wei, r.bribe_wei, r.block_number, r.status,
        r.scan_ms, r.sim_ms, r.exec_ms,
    ) {
        warn!("trades.csv: write error: {e}");
    }
}
