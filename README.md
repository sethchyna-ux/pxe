# ⚡ pxe-rs: High-Performance Rust PXE & Netboot Appliance

A blazing fast, memory-safe, all-in-one PXE server daemon written in Rust. Features integrated **DHCP / ProxyDHCP**, RFC-compliant **TFTP** with `blksize` acceleration, high-throughput **HTTP** asset streaming, and an interactive **Ratatui Terminal UI** (TUI).

---

## Key Features

- **Coexists With Existing Routers (ProxyDHCP)**: Runs out of the box on existing home/lab networks alongside your existing router (UniFi, pfSense, OPNsense, OpenWrt, ISP router) without causing DHCP IP collisions.
- **Standalone DHCP Mode**: Can alternatively function as an authoritative DHCP server with an integrated IP lease manager.
- **Hardware Architecture Auto-Detection**:
  - `0x0000` (Legacy BIOS x86) ➔ Serves `undionly.kpxe`
  - `0x0007` / `0x0009` (UEFI x86_64) ➔ Serves `ipxe.efi`
  - `0x000B` / `0x000C` (UEFI ARM64 / AArch64) ➔ Serves `ipxe-arm64.efi`
- **iPXE Stage-2 HTTP Chainloading**: Detects when an iPXE client boots (Option 77 / Option 175) and immediately chainloads `http://<server-ip>:<http-port>/boot.ipxe`, downloading kernels and ISOs over HTTP at line speed.
- **RFC-Compliant Accelerated TFTP**:
  - RFC 1350 (TFTP protocol)
  - RFC 2347 / 2348 (`blksize` option negotiation up to 1468 MTU)
  - RFC 2349 (`tsize` and `timeout` negotiation)
  - Built-in path canonicalization preventing directory traversal (`../`) attacks
- **HTTP Engine (Axum)**:
  - Serves kernels, initrds, and live ISOs with HTTP range-request support.
  - Dynamically renders `boot.ipxe` with dynamic server IP and port substitution.
- **Real-Time Terminal Dashboard (Ratatui + Crossterm)**:
  - Live activity stream with color-coded protocol events.
  - Active client lease table and detected hardware architectures.
  - Bandwidth counters, transfer metrics, and error rates.
  - Headless fallback (`--no-tui` or when piping stdout).

---

## Quick Start

### 1. Initialize Workspace Directories

Run the `init` command to set up directory structures and default iPXE boot menus:

```bash
cargo run --release -- init
```

This generates:

```text
pxe/
├── tftpboot/        # Network bootloader binaries (ipxe.efi, undionly.kpxe)
│   └── README.md
└── httpboot/        # HTTP netboot files (kernels, ISOs, menus)
    ├── boot.ipxe    # Dynamic netboot menu
    ├── alpine/
    ├── ubuntu/
    └── tools/
```

### 2. Download Official Bootloader Binaries

Download the signed upstream iPXE bootloaders into `tftpboot/`:

```bash
# UEFI x86_64
curl -L -o tftpboot/ipxe.efi https://boot.ipxe.org/ipxe.efi

# Legacy BIOS (x86)
curl -L -o tftpboot/undionly.kpxe https://boot.ipxe.org/undionly.kpxe

# UEFI ARM64 (AArch64)
curl -L -o tftpboot/ipxe-arm64.efi https://boot.ipxe.org/arm64-efi/ipxe.efi
```

### 3. Check Network Diagnostic

Verify that your primary LAN IPv4 address is automatically recognized:

```bash
cargo run --release -- status
```

### 4. Launch PXE Server

Start the server in ProxyDHCP mode (requires `sudo` to bind standard UDP port 67 and UDP port 69):

```bash
sudo ./target/release/pxe serve --mode proxy
```

---

## Operating Modes

### Mode 1: ProxyDHCP (Default & Recommended)

In ProxyDHCP mode, your normal home router allocates the IP address to the machine. `pxe-rs` listens on UDP 67 (and UDP 4011) and provides only the PXE boot instructions (Option 60 `PXEClient`, Option 66 TFTP IP, and Option 67 bootloader filename).

```bash
sudo ./target/release/pxe serve --mode proxy
```

### Mode 2: Standalone DHCP Server

If you are on an isolated lab network or switch without a router, `pxe-rs` can act as the authoritative DHCP server:

```bash
sudo ./target/release/pxe serve \
  --mode standalone \
  --dhcp-range 192.168.1.200,192.168.1.240 \
  --dhcp-router 192.168.1.1 \
  --dhcp-dns 1.1.1.1
```

### Mode 3: Unprivileged / Lab Testing (Non-Root)

To test in virtual machines or development environments without `sudo`:

```bash
./target/release/pxe serve \
  --no-tui \
  --dhcp-port 6767 \
  --tftp-port 6969 \
  --proxy-port 4012 \
  --http-port 8085
```

---

## Terminal UI Hotkeys

| Key | Description |
| :--- | :--- |
| `q` or `Esc` | Cleanly terminate server and restore terminal |
| `p` | Toggle live auto-scrolling on activity log |
| `c` | Clear current event logs |
| `d` | Filter activity log to DHCP events only |
| `t` | Filter activity log to TFTP events only |
| `h` | Filter activity log to HTTP events only |
| `↑` / `↓` | Scroll log buffer up / down |
| `PageUp` / `PageDown` | Scroll log buffer by 10 lines |

---

## Adding Operating System Images

### Alpine Linux Netboot (Fast RAM Boot)

Download Alpine's netboot kernel and initramfs into `httpboot/alpine/`:

```bash
curl -o httpboot/alpine/vmlinuz-virt https://dl-cdn.alpinelinux.org/alpine/v3.20/releases/x86_64/netboot/vmlinuz-virt
curl -o httpboot/alpine/initramfs-virt https://dl-cdn.alpinelinux.org/alpine/v3.20/releases/x86_64/netboot/initramfs-virt
curl -o httpboot/alpine/modloop-virt https://dl-cdn.alpinelinux.org/alpine/v3.20/releases/x86_64/netboot/modloop-virt
```

### Ubuntu 24.04 LTS Live Installer

Place the Ubuntu 24.04 ISO and extract `vmlinuz` and `initrd`:

```bash
# Place Ubuntu ISO inside httpboot/ubuntu/
cp /path/to/ubuntu-24.04-live-server-amd64.iso httpboot/ubuntu/
# Extract kernel and initrd from the ISO into httpboot/ubuntu/vmlinuz and httpboot/ubuntu/initrd
```

---

## Troubleshooting

### Address already in use (os error 48) on macOS

On macOS, Apple's built-in Internet Sharing / NetBoot daemon (`/usr/libexec/bootpd`) is socket-activated by `launchd` and holds UDP port 67 (`bootps`).

To free UDP port 67 for your PXE server, run:

```bash
sudo launchctl bootout system/com.apple.bootpd
```

To re-enable Apple's built-in service later (if ever needed):

```bash
sudo launchctl bootstrap system /System/Library/LaunchDaemons/bootps.plist
```

---

## Running Automated Tests

```bash
cargo test
cargo clippy -- -D warnings
```

