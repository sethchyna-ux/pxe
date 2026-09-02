# TFTP Boot Directory

Place your initial network bootloaders here:

| Architecture | Required Filename | Upstream Download Link |
|---|---|---|
| UEFI (x86_64) | `ipxe.efi` | `https://boot.ipxe.org/ipxe.efi` |
| Legacy BIOS (x86) | `undionly.kpxe` | `https://boot.ipxe.org/undionly.kpxe` |
| UEFI (ARM64 / AArch64) | `ipxe-arm64.efi` | `https://boot.ipxe.org/arm64-efi/ipxe.efi` |

Quick download command:
```bash
curl -o tftpboot/ipxe.efi https://boot.ipxe.org/ipxe.efi
curl -o tftpboot/undionly.kpxe https://boot.ipxe.org/undionly.kpxe
curl -o tftpboot/ipxe-arm64.efi https://boot.ipxe.org/arm64-efi/ipxe.efi
```
