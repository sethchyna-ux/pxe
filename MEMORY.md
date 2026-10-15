# Agent Memory & Interaction Preferences

## Output Format Configuration
- **Output Format Policy**: Standard Markdown (all-JSON format disabled per user request).

## Project Context
- **Workspace**: `/Volumes/laptopcard/pxe-fix-pxe-custom-bootloaded-c293cd24`
- **Application**: Rust PXE Server (`pxe-rs`)
- **Key Completed Fixes**:
  - DHCP Option 43 TLV formatted for Legacy BIOS (`BiosX86`) clients (sub-options 6, 8, 9, 10).
  - ProxyDHCP `yiaddr = 0.0.0.0` strictly enforced across `DHCPOFFER` and `DHCPACK`.
  - Router DHCP lease isolation: ignoring foreign `DHCPREQUEST` packets on UDP port 67 (`Option 54 != self.server_ip`).
  - ProxyDHCP port 4011 replies dispatched via unicast directly to client source socket.
  - Replaced hardcoded IP addresses with dynamic `${next-server}` in iPXE chainloader scripts (`autoexec.ipxe`).
  - Standalone mode fallback DNS `8.4.4.1` added to Option 6.
  - All 10 unit tests passing with zero compiler/clippy warnings.
  - Native macOS bundle rebuilt and deployed at `PXE Server.app` and `~/Applications/PXE Server.app`.
