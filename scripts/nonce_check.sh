#!/usr/bin/env bash
# nonce_check.sh — Nonce stability test before enabling LIVE execution.
#
# Usage:
#   bash scripts/nonce_check.sh           # 30s wait (default)
#   bash scripts/nonce_check.sh 60        # 60s wait
#   make nonce-check                      # calls this script
#
# PASS: nonce is identical at T=0 and T=SLEEP → no external tx sender active.
# FAIL: nonce changed → something sent transactions → BLOCK live execution.
#
# RPC_URL resolution order:
#   1. $RPC_URL already exported in current shell
#   2. RPC_URL= line in .env file in the same directory as this script's parent

set -euo pipefail

WALLET="0x3011BfD673a9D09f9761203A7fFCca757Af22587"
SLEEP_SEC="${1:-30}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# ── Resolve RPC_URL ─────────────────────────────────────────────────────────
if [ -z "${RPC_URL:-}" ]; then
    ENV_FILE="$REPO_ROOT/.env"
    if [ -f "$ENV_FILE" ]; then
        RPC_URL="$(grep '^RPC_URL=' "$ENV_FILE" | head -1 | cut -d= -f2-)"
    fi
fi

if [ -z "${RPC_URL:-}" ]; then
    echo ""
    echo "ERROR: RPC_URL is not set."
    echo ""
    echo "Fix options:"
    echo "  1) Add to .env:   RPC_URL=https://base-mainnet.g.alchemy.com/v2/YOUR_KEY"
    echo "  2) Export first:  export RPC_URL=https://... && make nonce-check"
    echo ""
    exit 1
fi

# ── Confirm cast is available ────────────────────────────────────────────────
if ! command -v cast &>/dev/null; then
    echo "ERROR: 'cast' not found. Install Foundry: https://getfoundry.sh"
    exit 1
fi

# ── Run the check ────────────────────────────────────────────────────────────
echo ""
echo "╔══════════════════════════════════════════════════════╗"
echo "║  HuntLoan — Nonce Stability Check                    ║"
echo "╚══════════════════════════════════════════════════════╝"
echo ""
echo "  Wallet  : $WALLET"
echo "  RPC     : $RPC_URL"
echo "  Wait    : ${SLEEP_SEC}s between readings"
echo ""

# Strip the cosmetic foundry.toml warning from cast output
get_nonce() {
    cast nonce "$WALLET" --rpc-url "$RPC_URL" 2>&1 \
        | grep -v '^Warning:' \
        | grep -E '^[0-9]+$' \
        | head -1
}

NONCE1="$(get_nonce)"
if [ -z "$NONCE1" ]; then
    echo "ERROR: Could not read nonce from RPC. Check RPC_URL is reachable."
    echo "       Test: curl -s -X POST $RPC_URL -H 'Content-Type: application/json' \\"
    echo "             -d '{\"jsonrpc\":\"2.0\",\"method\":\"eth_blockNumber\",\"params\":[],\"id\":1}'"
    exit 1
fi

echo "[$(date '+%H:%M:%S')] Check 1 — nonce = $NONCE1"
printf "  Sleeping %ss" "$SLEEP_SEC"
for _ in $(seq 1 "$SLEEP_SEC"); do printf "."; sleep 1; done
echo ""

NONCE2="$(get_nonce)"
echo "[$(date '+%H:%M:%S')] Check 2 — nonce = $NONCE2"
echo ""

# ── PASS / FAIL ──────────────────────────────────────────────────────────────
if [ "$NONCE1" = "$NONCE2" ]; then
    echo "┌─────────────────────────────────────────────────────┐"
    echo "│  PASS — nonce stable: $NONCE1 → $NONCE2            "
    echo "│  No external transactions detected.                  │"
    echo "│  Safe to proceed to the next pre-launch gate.        │"
    echo "└─────────────────────────────────────────────────────┘"
    echo ""
    exit 0
else
    DELTA=$(( NONCE2 - NONCE1 ))
    echo "┌─────────────────────────────────────────────────────┐"
    echo "│  FAIL — nonce changed: $NONCE1 → $NONCE2 (+$DELTA tx)"
    echo "│  An external process sent $DELTA transaction(s).    "
    echo "│  DO NOT set DRY_RUN=false until this is resolved.   │"
    echo "└─────────────────────────────────────────────────────┘"
    echo ""
    echo "Debugging steps:"
    echo ""
    echo "  1. Check for running Node.js processes:"
    echo "       ps aux | grep 'node\\|monitor_base' | grep -v grep"
    echo ""
    echo "  2. Check PM2 (on VPS: ssh root@159.89.21.106):"
    echo "       pm2 list"
    echo "       pm2 status mev-bot"
    echo ""
    echo "  3. Check cron jobs:"
    echo "       crontab -l"
    echo "       cat /etc/cron.d/* 2>/dev/null | grep -i mev"
    echo ""
    echo "  4. Find all processes with open connections:"
    echo "       lsof -i TCP | grep ESTABLISHED | grep -v grep"
    echo ""
    echo "  5. Trace live tx activity:"
    echo "       make nonce-watch"
    echo ""
    echo "  Fix: Stop the old bot completely before retrying."
    echo "  See: docs/DECOMMISSION_OLD_SYSTEM.md"
    echo ""
    exit 1
fi
