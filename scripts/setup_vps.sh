#!/usr/bin/env bash
# setup_vps.sh — One-shot prerequisites installer for Ubuntu/Debian VPS.
#
# Run as root or a user with sudo. Idempotent — safe to run multiple times.
#
# Installs:
#   - build-essential, pkg-config, libssl-dev, curl, git, tmux
#   - Rust stable toolchain (rustup)
#   - Foundry (forge + cast)
#
# Usage:
#   bash scripts/setup_vps.sh

set -euo pipefail

info()  { echo "[$(date '+%H:%M:%S')] INFO  $*"; }
ok()    { echo "[$(date '+%H:%M:%S')] OK    $*"; }
fail()  { echo "[$(date '+%H:%M:%S')] FAIL  $*"; exit 1; }

info "=== HuntLoan VPS Setup ==="
info "User: $(whoami)  Hostname: $(hostname)"

# ── System packages ──────────────────────────────────────────────────────────
info "Installing system packages..."
if command -v apt-get &>/dev/null; then
    apt-get update -qq
    apt-get install -y -qq \
        build-essential \
        pkg-config \
        libssl-dev \
        curl \
        git \
        tmux \
        jq
    ok "System packages installed"
elif command -v yum &>/dev/null; then
    yum install -y -q \
        gcc \
        openssl-devel \
        curl \
        git \
        tmux \
        jq
    ok "System packages installed (yum)"
else
    echo "WARNING: Unknown package manager — install build-essential, libssl-dev, curl, git, tmux manually"
fi

# ── Rust toolchain ────────────────────────────────────────────────────────────
if command -v rustup &>/dev/null; then
    info "Rust already installed — updating to stable..."
    rustup update stable --no-self-update
    ok "Rust: $(rustc --version)"
else
    info "Installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --default-toolchain stable --no-modify-path
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env"
    ok "Rust: $(rustc --version)"
fi

# Ensure cargo is on PATH
export PATH="$HOME/.cargo/bin:$PATH"

# ── Foundry (forge + cast) ────────────────────────────────────────────────────
if command -v cast &>/dev/null; then
    ok "Foundry already installed: $(cast --version 2>&1 | head -1)"
else
    info "Installing Foundry..."
    curl -L https://foundry.paradigm.xyz | bash
    # shellcheck source=/dev/null
    source "$HOME/.foundry/bin/foundry.sh" 2>/dev/null || true
    export PATH="$HOME/.foundry/bin:$PATH"
    foundryup
    ok "Foundry: $(cast --version 2>&1 | head -1)"
fi

# ── Sanity checks ─────────────────────────────────────────────────────────────
info "=== Sanity Checks ==="
echo "  rustc   : $(rustc --version)"
echo "  cargo   : $(cargo --version)"
echo "  cast    : $(cast --version 2>&1 | head -1)"
echo "  git     : $(git --version)"
echo "  tmux    : $(tmux -V)"
echo ""
ok "=== Setup complete ==="
echo ""
echo "Next steps:"
echo "  1. cd /home/santous/projects/huntloan"
echo "  2. cp .env.example .env && vi .env    (fill in RPC_URL, PRIVATE_KEY, etc.)"
echo "  3. cargo build --release"
echo "  4. make nonce-check"
echo "  5. make dry"
