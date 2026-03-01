# Phase 0 — Safe Decommission: Bitcoin-Sentinel MEV Bot

## Context

Two systems share the **same private key and operator wallet**.
Running both simultaneously causes **nonce conflicts** and doubled gas spend.

| Parameter | Old system | New system |
|---|---|---|
| Name | Bitcoin-Sentinel | HuntLoan |
| Language | Node.js | Rust |
| Location | `/root/Bitcoin-Sentinel` on VPS | Deploy from local to VPS |
| VPS IP | `159.89.21.106` | same VPS |
| Process manager | PM2 (`mev-bot`) | systemd (after deploy) |
| DRY_RUN | **false (LIVE)** | `true` (safe) |
| Private key | `0xb35c866e...` (shared) | same key |
| Operator wallet | `0x3011BfD673a9D09f9761203A7fFCca757Af22587` | same wallet |
| Active contract | `0xE5c3e80C243A6E21883E787013254BeAC829AD1E` | `0x0A0fE1f59D56716aF5c4C9D7688df742EE5949D3` |

---

## BLOCKER STATEMENT

> **The old system MUST be fully stopped before HuntLoan's `DRY_RUN` is set to `false`.**
> Failure to do so will result in nonce conflicts, wasted gas, and potential double-execution.

---

## Step 1 — Detect how old bot is running

SSH in and run ALL of these checks. Paste the output into your log.

```bash
ssh root@159.89.21.106

# Check PM2
pm2 list
pm2 status mev-bot 2>/dev/null || echo "PM2: mev-bot not found"

# Check systemd
systemctl status mev-bot 2>/dev/null || echo "SYSTEMD: mev-bot not found"
systemctl list-units --type=service | grep -i mev

# Check tmux sessions
tmux ls 2>/dev/null || echo "TMUX: no sessions"

# Check for nohup/background processes
ps aux | grep -E "monitor_base|mev-bot|node" | grep -v grep

# Check cron jobs
crontab -l 2>/dev/null || echo "No crontab"
cat /etc/cron.d/* 2>/dev/null | grep -i mev || echo "No system cron"
```

**Expected output (old system running via PM2):**
```
┌───┬──────────┬──────────┬─────────────────────────────────────────────┐
│id │ name     │ status   │ ...                                          │
├───┼──────────┼──────────┼─────────────────────────────────────────────┤
│ 0 │ mev-bot  │ online   │ ...                                          │
└───┴──────────┴──────────┴─────────────────────────────────────────────┘
```

**PASS:** You can identify the process manager.
**FAIL (BLOCKER):** If the process is not found via any method, search manually:
```bash
ps aux | grep node
lsof -i :8545 2>/dev/null
find /root -name "pm2" -o -name "*.pid" 2>/dev/null
```

---

## Step 2 — Stop the process

```bash
# If running via PM2 (most likely):
pm2 stop mev-bot
pm2 status mev-bot    # Expected: status = stopped

# If running via systemd:
systemctl stop mev-bot
systemctl status mev-bot   # Expected: Active: inactive (dead)

# If running in tmux (kill ALL node processes — confirm with ps first):
# tmux kill-session -t <session-name>
```

**Expected output (PM2 stop):**
```
[PM2] Applying action stopProcessId on app [mev-bot](ids: [ 0 ])
[PM2] [mev-bot](0) ✓
```

**PASS:** `pm2 status mev-bot` shows `stopped`.
**FAIL:** Process still shows `online` — run `pm2 kill` to nuke all PM2 processes, then verify with `ps aux | grep node`.

---

## Step 3 — Disable auto-restart permanently

```bash
# Remove from PM2 (prevents restart on next `pm2 resurrect` or reboot)
pm2 delete mev-bot
pm2 save --force

# Verify PM2 startup list is clean
pm2 list
# Expected: empty table or no mev-bot entry

# Verify PM2 dump (the file that is restored on boot)
cat ~/.pm2/dump.pm2 2>/dev/null | grep -i mev || echo "OK: mev-bot not in PM2 dump"

# Remove any systemd units if they exist
systemctl disable mev-bot 2>/dev/null && echo "Systemd unit disabled" || echo "No systemd unit"

# Remove cron if it exists
crontab -l 2>/dev/null | grep -v monitor_base | crontab -
crontab -l | grep monitor_base && echo "BLOCKER: cron still present" || echo "OK: cron clean"
```

**Expected output:**
```
[PM2] Saving current process list...
[PM2] Successfully saved in ~/.pm2/dump.pm2
OK: mev-bot not in PM2 dump
```

---

## Step 4 — Verify the old bot is truly dead

```bash
# 1. No PM2 processes
pm2 list
# Expected: empty or no mev-bot

# 2. No Node.js processes running the monitor script
ps aux | grep monitor_base | grep -v grep
# Expected: no output

# 3. Nonce stability test (run twice, 30s apart)
# On LOCAL machine (requires cast):
WALLET=0x3011BfD673a9D09f9761203A7fFCca757Af22587
RPC=https://base-mainnet.g.alchemy.com/v2/rPnh_aKOfxs07PhgeIJJX

cast nonce $WALLET --rpc-url $RPC
sleep 30
cast nonce $WALLET --rpc-url $RPC
# Expected: SAME number both times = old bot is not sending transactions
```

**PASS criteria:**
- `ps aux | grep monitor_base | grep -v grep` → empty
- `pm2 list` → no `mev-bot`
- Nonce is identical after 30 seconds

**FAIL (BLOCKER):** Nonce is incrementing → old bot is still active somewhere.
Action: `ps aux | grep node`, kill by PID: `kill -9 <PID>`, then re-verify.

---

## Step 5 — Archive the old repository

```bash
# On VPS — archive to a safe location (do NOT delete)
ARCHIVE_DIR="/home/santous/archive"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

mkdir -p "$ARCHIVE_DIR"

# Freeze the .env first (keep a record of what was running)
cp /root/Bitcoin-Sentinel/.env /root/Bitcoin-Sentinel/.env.archived-${TIMESTAMP}

# Archive the entire repo
cp -rp /root/Bitcoin-Sentinel "$ARCHIVE_DIR/bitcoin-sentinel-${TIMESTAMP}"

echo "Archived to: $ARCHIVE_DIR/bitcoin-sentinel-${TIMESTAMP}"
ls -lh "$ARCHIVE_DIR/"
```

**Expected:**
```
Archived to: /home/santous/archive/bitcoin-sentinel-20260301_170000
total 8.0K
drwxr-xr-x 1 root root  4096 Mar  1 17:00 bitcoin-sentinel-20260301_170000
```

### Restore procedure

If you ever need to restore the old system:
```bash
# 1. Restore from archive
cp -rp /home/santous/archive/bitcoin-sentinel-<TIMESTAMP> /root/Bitcoin-Sentinel-restored

# 2. Restore .env
cp /root/Bitcoin-Sentinel-restored/.env.archived-<TIMESTAMP> \
   /root/Bitcoin-Sentinel-restored/.env

# 3. Re-register with PM2
cd /root/Bitcoin-Sentinel-restored/eth_forensics/simulation
pm2 start ecosystem.config.js   # or: pm2 start scripts/monitor_base.js --name mev-bot

# 4. IMPORTANT: Stop HuntLoan first if it is running!
#    systemctl stop huntloan
```

---

## Step 6 — Confirm wallet isolation

```bash
# Check that .env files are NOT identical on the VPS
# (after HuntLoan is deployed to VPS, both .env files must NOT have the same PRIVATE_KEY
#  without the old system being decommissioned)

# Old system key (Bitcoin-Sentinel):
grep PRIVATE_KEY /root/Bitcoin-Sentinel/.env | head -1

# New system key (HuntLoan — after deploy):
grep PRIVATE_KEY /root/huntloan/.env 2>/dev/null || echo "HuntLoan .env not yet deployed"
```

**SHARED KEY RULE:**
> If both .env files contain the same `PRIVATE_KEY`, then:
> - HuntLoan **MUST** stay `DRY_RUN=true` until the old system is fully decommissioned (Steps 1–5 complete).
> - Only after Step 4's nonce stability test passes can you set `DRY_RUN=false`.

---

## Post-Decommission Gate Checklist

Complete ALL before setting `DRY_RUN=false` on HuntLoan.

```
[ ] Step 1: Old bot process detected and method identified
[ ] Step 2: pm2 stop mev-bot — shows "stopped"
[ ] Step 3: pm2 delete + pm2 save --force — mev-bot gone from dump.pm2
[ ] Step 3: No cron jobs restarting monitor_base.js
[ ] Step 4: ps aux shows no monitor_base.js process
[ ] Step 4: Nonce stability test — same nonce after 30 seconds
[ ] Step 5: Archive created at /home/santous/archive/bitcoin-sentinel-<TIMESTAMP>
[ ] Step 6: Confirmed shared key — HuntLoan in DRY_RUN=true during decommission
```

---

## Notes

- The old contract (`0xE5c3e80C243A6E21883E787013254BeAC829AD1E`) stays deployed on-chain.
  HuntLoan references it as `LEGACY_FLASH_LIQUIDATOR` (read-only constant, never called).
- Key rotation (generating a new private key) is recommended for long-term security
  but requires redeploying `HuntLoanFlashReceiver` with the new operator address.
- VPS firewall: no port change needed. HuntLoan uses outbound-only RPC connections.
