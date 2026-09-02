# ⚡ pxe-rs: High-Performance Rust PXE & Netboot Appliance

A blazing fast, memory-safe, all-in-one PXE netboot server daemon written in Rust. Features integrated **DHCP / ProxyDHCP**, RFC-compliant **TFTP** with `blksize` acceleration, high-throughput **HTTP** asset streaming, an interactive **Ratatui Terminal UI** (TUI), a real-time **Web Dashboard**, and a native **macOS Application (`PXE Server.app`)**.

---

## 🌟 Key Features

- **One-Click Native macOS App (`PXE Server.app`)**:
  - Launches with automatic Touch ID / Admin elevation to bind low-level ports (`UDP 67/69`).
  - Native WebKit (`WKWebView`) dark-mode operations dashboard.
  - Zero heavy third-party framework bloat (pure Swift AppKit + native Rust).
- **Real-Time Web Dashboard (`/dashboard`)**:
  - Accessible locally in the app and remotely from any phone, tablet, or browser on your LAN at `http://<server-ip>:8080/dashboard`.
  - Real-time stat cards: active bare-metal nodes, DHCP in/out, TFTP bandwidth, HTTP requests, and error rate.
  - Live client session tracking: MAC, assigned IP, architecture, boot stage, and last-seen timers.
  - Streaming color-coded activity log: `[DHCP]`, `[TFTP]`, `[HTTP]`, `[SYS]`.
- **Coexists With Existing Routers (ProxyDHCP)**:
  - Runs on existing home/lab networks alongside your existing router (UniFi, pfSense, OPNsense, OpenWrt, ISP router) without causing DHCP IP collisions.
- **Standalone DHCP Mode**:
  - Functions as an authoritative DHCP server with an integrated IP lease manager for isolated switches.
- **Hardware Architecture Auto-Detection**:
  - `0x0000` (Legacy BIOS x86) ➔ Serves `undionly.kpxe`
  - `0x0007` / `0x0009` (UEFI x86_64) ➔ Serves `ipxe.efi` (`snponly.efi`)
  - `0x000B` / `0x000C` (UEFI ARM64 / AArch64) ➔ Serves `ipxe-arm64.efi`
- **RFC-Compliant Accelerated TFTP**:
  - RFC 1350 (TFTP protocol + strict EOF zero-byte packet handling)
  - RFC 2347 / 2348 (`blksize` option negotiation up to 1468 MTU)
  - RFC 2349 (`tsize` and `timeout` negotiation)
  - Built-in path canonicalization preventing directory traversal (`../`) attacks
- **High-Throughput HTTP Engine (Axum)**:
  - Streams gigabyte-sized Linux live ISOs, kernels, and initrds at full wire speed.
  - Dynamically interpolates server IP and port into iPXE scripts.
- **Terminal UI (Ratatui + Crossterm)**:
  - Keyboard-driven terminal dashboard for headless servers and SSH sessions.

---

## 🚀 Quick Start

### Method 1: One-Click macOS App (Recommended)

Double-click to launch from:

- 🖥️ **Desktop**: `~/Desktop/PXE Server.app`
- 📁 **Applications**: `~/Applications/PXE Server.app`
- 🛠️ **Project Root**: `./PXE Server.app`

macOS will prompt for Touch ID or your password once to authorize port binding, and the dashboard window will immediately open.

#### Rebuilding the macOS App

```bash
./macos/build_app.sh
```

---

### Method 2: Command Line (CLI / TUI)

#### 1. Initialize Directories

```bash
cargo run --release -- init
```

#### 2. Run Network Diagnostics

Check your host's active IP address and verify port availability:

```bash
cargo run --release -- status
```

#### 3. Start in ProxyDHCP Mode (Alongside Existing Router)

```bash
sudo ./target/release/pxe serve --mode proxy
```

#### 4. Start in Standalone DHCP Mode (Dedicated Server)

```bash
sudo ./target/release/pxe serve \
  --mode standalone \
  --dhcp-range 192.168.1.200,192.168.1.240 \
  --dhcp-router 192.168.1.1 \
  --dhcp-dns 1.1.1.1
```

#### 5. Headless Daemon Mode (No TUI)

```bash
sudo ./target/release/pxe serve --mode standalone --dhcp-range 192.168.1.200,192.168.1.240 --no-tui
```

---

## 📊 Live Web Dashboard & REST API

The server exposes an HTTP API and interactive dashboard on TCP port 8080:

- **Dashboard UI**: `http://127.0.0.1:8080/dashboard` (or `http://192.168.1.58:8080/dashboard`)
- **Raw iPXE Script**: `http://127.0.0.1:8080/boot.ipxe`
- **JSON Telemetry API**: `http://127.0.0.1:8080/api/status`

### `GET /api/status` Response Example

```json
{
  "server_ip": "192.168.1.58",
  "http_port": 8080,
  "uptime_secs": 184,
  "metrics": {
    "dhcp_in": 12,
    "dhcp_out": 12,
    "tftp_bytes": 1159680,
    "tftp_files": 2,
    "http_requests": 8,
    "errors": 0
  },
  "clients": [
    {
      "mac": "f0:79:59:70:da:7f",
      "ip": "192.168.1.200",
      "arch": "UEFI (x86_64)",
      "stage": "Booted",
      "stage_text": "Booted",
      "last_seen_secs_ago": 15
    }
  ],
  "events": [
    {
      "timestamp": "13:10:00",
      "level": "Success",
      "protocol": "System",
      "message": "Starting PXE-RS server on 192.168.1.58 [Standalone]"
    }
  ]
}
```

---

## 📁 Storage Architecture & OS Images

All large distribution ISOs (~11 GB) are housed on `/Volumes/laptopcard/pxe/httpboot/` and symlinked directly into `httpboot/`:

| Distribution | Files in `httpboot/<distro>/` | Size | Boot Mode |
| :--- | :--- | :--- | :--- |
| **Ubuntu 24.04 LTS Server** | `ubuntu-24.04.4-live-server-amd64.iso`, `vmlinuz`, `initrd` | 3.2 GB | Casper HTTP ISO Loopback |
| **Linux Mint 22 (Cinnamon)** | `linuxmint-22-cinnamon-64bit.iso`, `vmlinuz`, `initrd.lz` | 2.8 GB | Casper Netboot |
| **Arch Linux** | `vmlinuz-linux`, `initramfs-linux.img` | 250 MB | Rolling Official Netboot |
| **Fedora 44 Server** | `vmlinuz`, `initrd.img` | 270 MB | Anaconda Netinstall |
| **Solus Budgie Desktop** | `Solus-Budgie-Release-2026-04-18.iso` | 4.0 GB | Direct SANBOOT |
| **Alpine Linux** | `vmlinuz-virt`, `initramfs-virt`, `modloop-virt` | 20 MB | Fast RAM Boot |
| **netboot.xyz** | Upstream catalog chainload | Dynamic | 50+ Cloud Distros |

---

## 🖥️ Bare-Metal Motherboard Deployment (Hardware Quirks Guide)

Deploying bare-metal netboot across real motherboards (such as the **ASUS Z97-A** / Intel Haswell) requires addressing several low-level firmware quirks:

### 1. Haswell / 9-Series Intel I218-V PCIe Lockup

- **Symptom**: `ipxe.efi` loads, displays `iPXE initialising devices...`, and completely freezes the system.
- **Cause**: The full-driver build of `ipxe.efi` contains a native Intel PCIe driver that attempts to reset the PHY registers while UEFI firmware still controls the bus.
- **Solution**: Deploy the official **`snponly.efi`** build (`301 KB`). It communicates exclusively through the UEFI Simple Network Protocol, avoiding the PCIe reset freeze.

### 2. ASUS BIOS CSM & Option ROM Configuration

For the motherboard to discover network boot targets:

1. Enter BIOS (`Del` / `F2`).
2. Go to **Advanced ➔ Onboard Devices Configuration** ➔ Set **Intel PXE OPROM** to **Enabled**.
3. Go to **Boot ➔ CSM (Compatibility Support Module)** ➔ Set **Launch CSM** to **Enabled**.
4. Set **Boot from Network Devices** to **UEFI driver first** (or **Legacy only**).
5. In the Boot Menu (`F8`), select **`UEFI: IP4 Intel Ethernet Connection I218-V`** (or **`IBA GE Slot`** for Legacy).

### 3. AMI UEFI Option 67 Null-Termination

- **Symptom**: TFTP logs show errors like `File not found: ipxe.efi\ufffd`.
- **Cause**: AMI UEFI reads DHCP Option 67 past the string boundary if not explicitly null-terminated.
- **Solution**: `pxe-rs` appends an explicit `\0` byte to DHCP Options 66 and 67, and filters non-ASCII graphics in the TFTP engine.

### 4. TFTP RFC 1350 Exact Block Multiples (EOF Bug)

- **Symptom**: TFTP transfers freeze indefinitely on the last block (e.g., block 465 of 465).
- **Cause**: RFC 1350 dictates that when a file length is an exact multiple of the block size (e.g. `238,080 / 512 = 465`), the server **must** send a terminating 0-byte DATA packet to signal EOF.
- **Solution**: Handled automatically in `src/tftp/server.rs`.

### 5. Preventing iPXE Bootloops

In `httpboot/boot.ipxe`, the menu selection timeout is set to `0`:

```ipxe
choose --timeout 0 --default ubuntu target && goto ${target}
```

This keeps the menu on screen indefinitely until you press Enter, preventing continuous reboots.

---

## ⌨️ Terminal UI Controls

When running via `./target/release/pxe serve`:

| Key | Action |
| :--- | :--- |
| `q` / `Esc` | Cleanly terminate server and restore terminal |
| `p` | Toggle auto-scroll on activity log stream |
| `c` | Clear log buffer |
| `d` | Filter log to **DHCP** events only |
| `t` | Filter log to **TFTP** events only |
| `h` | Filter log to **HTTP** events only |
| `↑` / `↓` | Scroll log buffer up / down |
| `PageUp` / `PageDown` | Scroll log buffer by 10 lines |

---

## 🛠️ Troubleshooting

### Port 67 Busy on macOS (`Address already in use - os error 48`)

macOS runs a background NetBoot/DHCP helper (`bootpd`). To release UDP 67:

```bash
sudo launchctl bootout system/com.apple.bootpd
```

### Ubuntu Drops into Busybox `(initramfs)`

- **Cause**: Booting with an Ubuntu Cloud-Image initrd (~29 MB) instead of the Casper Live Installer initrd (~76 MB).
- **Fix**: Extract `casper/vmlinuz` and `casper/initrd` directly from `ubuntu-24.04.4-live-server-amd64.iso` (see [`httpboot/README.md`](httpboot/README.md)).

---

## 🧪 Testing

```bash
# Run unit and integration tests
cargo test

# Validate codebase with zero warnings
cargo clippy -- -D warnings
```
