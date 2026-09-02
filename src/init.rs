use std::fs;
use std::path::Path;

pub fn initialize_workspace(target_dir: &Path) -> std::io::Result<()> {
    let tftp_dir = target_dir.join("tftpboot");
    let http_dir = target_dir.join("httpboot");

    fs::create_dir_all(&tftp_dir)?;
    fs::create_dir_all(&http_dir)?;
    fs::create_dir_all(http_dir.join("alpine"))?;
    fs::create_dir_all(http_dir.join("ubuntu"))?;
    fs::create_dir_all(http_dir.join("tools"))?;

    // Create TFTP README with download instructions for bootloader binaries
    let tftp_readme = tftp_dir.join("README.md");
    if !tftp_readme.exists() {
        fs::write(
            &tftp_readme,
            r#"# TFTP Boot Directory

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
"#,
        )?;
    }

    // Create default boot.ipxe in httpboot
    let ipxe_file = http_dir.join("boot.ipxe");
    if !ipxe_file.exists() {
        fs::write(
            &ipxe_file,
            r#"#!ipxe
# Custom PXE Netboot Menu
# Variables ${server_ip} and ${http_port} are dynamically substituted by pxe-rs

set base_url http://${server_ip}:${http_port}

:start
menu PXE Netboot Menu [Client MAC: ${mac}]
item --gap --             ---------------- Operating Systems ----------------
item alpine               Alpine Linux (Fast RAM Boot)
item ubuntu               Ubuntu 24.04 LTS Live Installer
item debian               Debian 12 Bookworm Network Installer
item --gap --             ---------------- System Diagnostics ------------------
item memtest              Memtest86+ Memory Test
item netboot_xyz          Chainload netboot.xyz (Public Multi-OS Index)
item --gap --             ---------------- Bootloader Control ------------------
item ipxe_shell           Drop to interactive iPXE shell
item reboot               Reboot host
item exit                 Exit and boot local drive
choose --timeout 15000 --default alpine target && goto ${target}

:alpine
echo Booting Alpine Linux from ${base_url}...
kernel ${base_url}/alpine/vmlinuz-virt ip=dhcp alpine_repo=https://dl-cdn.alpinelinux.org/alpine/v3.20/main/ modloop=${base_url}/alpine/modloop-virt quiet
initrd ${base_url}/alpine/initramfs-virt
boot || goto failed

:ubuntu
echo Booting Ubuntu 24.04 LTS Installer...
kernel ${base_url}/ubuntu/vmlinuz ip=dhcp url=${base_url}/ubuntu/ubuntu-24.04-live-server-amd64.iso autoinstall ds=nocloud-net;s=${base_url}/ubuntu/
initrd ${base_url}/ubuntu/initrd
boot || goto failed

:debian
echo Booting Debian 12 Installer...
kernel ${base_url}/debian/linux vga=788 --- quiet
initrd ${base_url}/debian/initrd.gz
boot || goto failed

:memtest
echo Loading Memtest86+...
kernel ${base_url}/tools/memtest.bin
boot || goto failed

:netboot_xyz
echo Loading upstream netboot.xyz catalog...
chain --autofree https://boot.netboot.xyz || goto failed

:ipxe_shell
echo Type 'exit' to return to menu, or run iPXE commands directly.
shell
goto start

:reboot
reboot

:exit
exit

:failed
echo Boot execution failed. Press any key to return to menu...
prompt
goto start
"#,
        )?;
    }

    Ok(())
}
