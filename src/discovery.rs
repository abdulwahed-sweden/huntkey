/// HuntLoan discovery — Goldsky subgraph paginator for Aave V3 Base borrowers.
///
/// Queries the Aave V3 Base subgraph for accounts with an active borrow
/// position (currentTotalDebt > 0) and writes their addresses to the watchlist
/// JSON so the scanner can pick them up on the next block.
///
/// Endpoint: Goldsky public Aave V3 Base subgraph
/// Pagination: cursor-based via id_gt, 1000 per page.
/// Output: plain JSON string array — ["0xabc...", ...]
use alloy::primitives::Address;
use eyre::{Result, WrapErr};
use serde::Deserialize;
use tracing::{info, warn};

const GOLDSKY_URL: &str =
    "https://api.goldsky.com/api/public/project_clk74pd7lueg738tw9sjh79d6/subgraphs/aave-v3-base/1.0.0/gn";

const PAGE_SIZE: usize = 1_000;

// ── GraphQL response types ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GqlResponse {
    data: Option<GqlData>,
}

#[derive(Debug, Deserialize)]
struct GqlData {
    users: Vec<GqlUser>,
}

#[derive(Debug, Deserialize)]
struct GqlUser {
    id: String,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Fetch all active Aave V3 Base borrowers from the Goldsky subgraph.
///
/// Returns deduplicated, checksummed addresses sorted ascending.
/// On partial failure (a page errors), logs a warning and returns whatever
/// was collected so far — better to have a partial list than nothing.
pub async fn fetch_borrowers() -> Result<Vec<Address>> {
    let client  = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .wrap_err("HTTP client build failed")?;

    let mut all: Vec<Address> = Vec::new();
    let mut cursor = String::new(); // id_gt cursor — empty = start from beginning

    loop {
        let page = match fetch_page(&client, &cursor).await {
            Ok(p)  => p,
            Err(e) => {
                warn!("[discovery] Subgraph page error (cursor={:?}): {}", cursor, e);
                break;
            }
        };

        let count = page.len();

        if count == 0 {
            break;
        }

        // The last id becomes the next cursor
        if let Some(last) = page.last() {
            cursor = format!("{last:?}");
        }

        all.extend(page);

        if count < PAGE_SIZE {
            // Last page
            break;
        }
    }

    // Dedup (addresses already parsed — no duplicates from pagination but defensive)
    all.sort();
    all.dedup();

    info!(
        count = all.len(),
        "[discovery] Subgraph fetch complete"
    );

    Ok(all)
}

/// Refresh the watchlist JSON file from the subgraph.
///
/// Overwrites `path` with a fresh address list. Safe to call periodically
/// (e.g., every N blocks in the engine). The scanner reads the file each block,
/// so it will pick up new borrowers on the next iteration.
pub async fn refresh_watchlist(path: &str) -> Result<()> {
    let borrowers = fetch_borrowers().await?;

    let json = serde_json::to_string_pretty(
        &borrowers
            .iter()
            .map(|a| format!("{a:?}"))
            .collect::<Vec<_>>(),
    )
    .wrap_err("JSON serialization failed")?;

    std::fs::write(path, &json)
        .wrap_err_with(|| format!("Cannot write watchlist: {path}"))?;

    info!(
        path  = path,
        count = borrowers.len(),
        "[discovery] Watchlist written"
    );

    Ok(())
}

// ── Internal ──────────────────────────────────────────────────────────────────

async fn fetch_page(client: &reqwest::Client, cursor: &str) -> Result<Vec<Address>> {
    let id_filter = if cursor.is_empty() {
        String::new()
    } else {
        format!(", id_gt: \"{cursor}\"")
    };

    let query = format!(
        r#"{{
  users(
    first: {PAGE_SIZE}
    orderBy: id
    orderDirection: asc
    where: {{ currentTotalDebt_gt: "0"{id_filter} }}
  ) {{
    id
  }}
}}"#
    );

    let body = serde_json::json!({ "query": query });

    let resp = client
        .post(GOLDSKY_URL)
        .json(&body)
        .send()
        .await
        .wrap_err("Subgraph request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text   = resp.text().await.unwrap_or_default();
        eyre::bail!("Subgraph HTTP {}: {}", status, &text[..text.len().min(200)]);
    }

    let gql: GqlResponse = resp
        .json()
        .await
        .wrap_err("Subgraph JSON parse failed")?;

    let users = gql
        .data
        .map(|d| d.users)
        .unwrap_or_default();

    let addrs: Vec<Address> = users
        .iter()
        .filter_map(|u| u.id.parse::<Address>().ok())
        .collect();

    Ok(addrs)
}
