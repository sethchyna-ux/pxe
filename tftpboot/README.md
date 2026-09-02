# 🗂️ TFTP Boot Directory (`tftpboot/`)

This directory contains the initial Stage-1 network bootloaders served over **TFTP (UDP 69)** to bare-metal UEFI and Legacy BIOS clients.

---

## Bootloader Binaries

| Client Architecture | Recommended File | Fallback File | Upstream Source | Description |
| :--- | :--- | :--- | :--- | :--- |
| **UEFI (x86_64)** | **`ipxe.efi`** (symlink to `snponly.efi`) | `ipxe.efi0`, `bootx64.efi` | `https://boot.ipxe.org/x86_64-efi/snponly.efi` | **Recommended**: Uses UEFI SNP drivers. Avoids PCIe hardware reset hangs on Intel Haswell/Z97 chipsets. |
| **UEFI Native Driver (x86_64)** | `ipxe-full.efi` | — | `https://boot.ipxe.org/x86_64-efi/ipxe.efi` | Contains full internal Intel/Realtek PCIe drivers. |
| **Legacy BIOS (x86)** | **`undionly.kpxe`** | `undionly.kpxe0` | `https://boot.ipxe.org/undionly.kpxe` | Uses the motherboard's Intel Boot Agent Option ROM UNDI interface. |
| **UEFI (ARM64 / AArch64)** | **`ipxe-arm64.efi`** | `bootaa64.efi` | `https://boot.ipxe.org/arm64-efi/ipxe.efi` | 64-bit ARM netboot driver. |

---

## Automatic Entrypoint: `autoexec.ipxe`

When official iPXE binaries initialize on bare metal, their default embedded script automatically queries TFTP for **`autoexec.ipxe`**.

This file acts as our bridge to HTTP:

```ipxe
#!ipxe
echo Loading PXE Netboot Menu...
chain --autofree http://192.168.1.58:8080/boot.ipxe || shell
```

- Immediately chainloads the interactive HTTP boot menu from `httpboot/boot.ipxe`.
- Drops to an interactive `iPXE>` shell if the menu cannot be downloaded, preventing black-screen hangs.

---

## Legacy ROM Aliases & Quirks

Certain older AMI/Phoenix UEFI firmwares and Intel PXE Option ROMs append suffixes or query alternative filenames:

- `ipxe.efi0` ➔ Symlink to `ipxe.efi` (Queried by Intel PXE ROMs that append `.0`)
- `bootx64.efi` ➔ Symlink to `ipxe.efi` (Standard fallback UEFI name)
