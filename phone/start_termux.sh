#!/data/data/com.termux/files/usr/bin/bash
# ==============================================================================
# ⚡ pxe-rs Android Phone (Termux) Launcher
# ==============================================================================
# Runs pxe-rs directly on an Android phone (over Wi-Fi or USB-C Ethernet adapter).
# - If rooted (tsu / su): binds standard PXE UDP ports 67, 4011, 69 and HTTP 8080.
# - If unrooted: uses iptables/nftables port redirect if available, or runs on
#   unprivileged ports (DHCP 6767, TFTP 6969, HTTP 8080) for HTTP/iPXE chainboot.
# ==============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$SCRIPT_DIR"

if ! command -v cargo >/dev/null 2>&1; then
    echo "==> Installing Rust & Clang in Termux..."
    pkg update -y && pkg install -y rust clang git curl
fi

if [ ! -f "./target/release/pxe" ]; then
    echo "==> Building pxe-rs release binary for Android ARM64..."
    cargo build --release
fi

echo "==> Checking network interfaces..."
./target/release/pxe status

if command -v tsu >/dev/null 2>&1 || su -c "id" >/dev/null 2>&1; then
    echo "==> Root detected! Launching full PXE Server (UDP 67/4011/69 + TCP 8080)..."
    su -c "cd '$SCRIPT_DIR' && ./target/release/pxe serve --mode proxy"
else
    echo "==> Unrooted Android mode: Launching on unprivileged ports (DHCP 6767, TFTP 6969, HTTP 8080)..."
    echo "    Tip: Open http://127.0.0.1:8080/dashboard in your phone browser!"
    ./target/release/pxe serve --dhcp-port 6767 --tftp-port 6969 --http-port 8080
fi
