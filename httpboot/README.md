# 🌐 HTTP Boot Directory (`httpboot/`)

This directory contains Stage-2 assets served over **HTTP (TCP 8080)** by the Axum web engine in `pxe-rs`. It hosts Linux kernels (`vmlinuz`), initial ramdisks (`initrd`), full installer ISOs, and the interactive iPXE boot menu.

---

## Directory Layout

```text
httpboot/
├── boot.ipxe         # Dynamic interactive iPXE boot menu
├── autoexec.ipxe     # Secondary HTTP fallback entrypoint
├── ubuntu/           # Ubuntu 24.04 LTS Live Server
│   ├── vmlinuz       # Extracted live kernel (15 MB)
│   ├── initrd        # Genuine Casper live installer initrd (76 MB)
│   └── ubuntu-24.04.4-live-server-amd64.iso (3.2 GB)
├── mint/             # Linux Mint 22 (Cinnamon 64-bit)
│   ├── vmlinuz       # Live kernel (15 MB)
│   ├── initrd.lz     # Casper initrd (70 MB)
│   └── linuxmint-22-cinnamon-64bit.iso (2.8 GB)
├── arch/             # Arch Linux (Rolling Netboot)
│   ├── vmlinuz-linux (16 MB)
│   └── initramfs-linux.img (240 MB)
├── fedora/           # Fedora 44 Server (Anaconda Netinstall)
│   ├── vmlinuz       (18 MB)
│   └── initrd.img    (260 MB)
├── solus/            # Solus Budgie Desktop (Live ISO)
│   └── Solus-Budgie-Release-2026-04-18.iso (4.0 GB)
└── alpine/           # Alpine Linux (Fast RAM Boot ~20MB)
    ├── vmlinuz-virt
    ├── initramfs-virt
    └── modloop-virt
```

---

## External Volume Storage (`/Volumes/laptopcard`)

Large ISO distributions (~11 GB) are stored directly on the high-capacity external volume:
`/Volumes/laptopcard/pxe/httpboot/`

The directories are transparently symlinked into `pxe/httpboot/`:

```bash
ln -sf /Volumes/laptopcard/pxe/httpboot/ubuntu ./httpboot/ubuntu
ln -sf /Volumes/laptopcard/pxe/httpboot/mint ./httpboot/mint
ln -sf /Volumes/laptopcard/pxe/httpboot/arch ./httpboot/arch
ln -sf /Volumes/laptopcard/pxe/httpboot/fedora ./httpboot/fedora
ln -sf /Volumes/laptopcard/pxe/httpboot/solus ./httpboot/solus
```

---

## How to Extract Live Kernels and Initrds from ISOs

To netboot modern live distributions without dropping into Busybox `(initramfs)`:

### Ubuntu 24.04 LTS

Extract the genuine Casper installer files directly from the ISO using `bsdtar`:

```bash
cd httpboot/ubuntu
bsdtar -xf ubuntu-24.04.4-live-server-amd64.iso casper/vmlinuz casper/initrd
mv casper/vmlinuz ./vmlinuz
mv casper/initrd ./initrd
rmdir casper
```

### Linux Mint 22

```bash
cd httpboot/mint
bsdtar -xf linuxmint-22-cinnamon-64bit.iso casper/vmlinuz casper/initrd.lz
mv casper/vmlinuz ./vmlinuz
mv casper/initrd.lz ./initrd.lz
rmdir casper
```

---

## Editing the iPXE Menu (`boot.ipxe`)

`boot.ipxe` dynamically interpolates `${server_ip}` and `${http_port}` when served by `pxe-rs`.

### Preventing Bootloops

Ensure the menu selection timeout is set to `0` (indefinite wait):

```ipxe
# Timeout 0 waits forever for keyboard selection (no auto-bootloop)
choose --timeout 0 --default ubuntu target && goto ${target}
```
