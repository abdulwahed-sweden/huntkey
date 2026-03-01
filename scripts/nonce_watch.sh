#!/usr/bin/env bash
# nonce_watch.sh — Continuous nonce monitor. Warns on any external tx activity.
#
# Usage:
#   bash scripts/nonce_watch.sh           # poll every 10s (default)
#   bash scripts/nonce_watch.sh 5         # poll every 5s
#   POLL=5 make nonce-watch               # via Makefile
#
# Output:
#   [HH:MM:SS] OK    — nonce stable: N
#   [HH:MM:SS] WARN  — nonce changed: N → M (+K tx) ← something sent transactions
#
# Press Ctrl+C to stop.

POLL_SEC="${1:-10}"
WALLET="0x3011BfD673a9D09f9761203A7fFCca757Af22587"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# ── Resolve RPC_URL ─────────────────────────────────────────────────────────
if [ -z "${RPC_URL:-}" ]; then
    ENV_FILE="$REPO_ROOT/.env"
    if [ -f "$ENV_FILE" ]; then
        RPC_URL="$(grep '^RPC_URL=' "$ENV_FILE" | head -1 | cut -d= -f2-)"
    fi
fi

if [ -z "${RPC_URL:-}" ]; then
    echo "ERROR: RPC_URL not set. Add to .env or export RPC_URL=... first."
    exit 1
fi

if ! command -v cast &>/dev/null; then
    echo "ERROR: 'cast' not found. Install Foundry: https://getfoundry.sh"
    exit 1
fi

get_nonce() {
    cast nonce "$WALLET" --rpc-url "$RPC_URL" 2>&1 \
        | grep -v '^Warning:' \
        | grep -E '^[0-9]+$' \
        | head -1
}

echo ""
echo "╔══════════════════════════════════════════════════════╗"
echo "║  HuntLoan — Nonce Watcher                            ║"
echo "╚══════════════════════════════════════════════════════╝"
echo ""
echo "  Wallet  : $WALLET"
echo "  RPC     : $RPC_URL"
echo "  Poll    : every ${POLL_SEC}s"
echo "  Stop    : Ctrl+C"
echo ""

PREV="$(get_nonce)"
if [ -z "$PREV" ]; then
    echo "ERROR: Could not read initial nonce. Check RPC_URL."
    exit 1
fi

echo "[$(date '+%H:%M:%S')] Starting nonce: $PREV"
echo ""

WARN_COUNT=0

while true; do
    sleep "$POLL_SEC"
    CURR="$(get_nonce)"

    if [ -z "$CURR" ]; then
        echo "[$(date '+%H:%M:%S')] ERROR — RPC call failed (connection issue)"
        continue
    fi

    if [ "$CURR" != "$PREV" ]; then
        DELTA=$(( CURR - PREV ))
        WARN_COUNT=$(( WARN_COUNT + 1 ))
        echo "[$(date '+%H:%M:%S')] WARN  — nonce changed: $PREV → $CURR (+$DELTA tx) [warning #$WARN_COUNT]"
        echo "                  External tx detected! Check: ps aux | grep node && pm2 list"
        PREV="$CURR"
    else
        echo "[$(date '+%H:%M:%S')] OK    — nonce stable: $CURR"
    fi
done
