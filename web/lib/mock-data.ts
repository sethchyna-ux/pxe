import type { BootImage, ImagePreset, StatusSnapshot } from "./types";

export const mockStatus: StatusSnapshot = {
  server_ip: "192.168.1.58",
  http_port: 8080,
  uptime_secs: 18437,
  metrics: {
    dhcp_in: 1842,
    dhcp_out: 1839,
    tftp_bytes: 247865344,
    tftp_files: 96,
    http_requests: 4217,
    errors: 2,
  },
  clients: [
    {
      mac: "F0:79:59:70:DA:7F",
      ip: "192.168.1.215",
      arch: "UEFI x86_64",
      stage: { TftpTransfer: { file: "ipxe.efi", percent: 72 } },
      stage_text: "TFTP ipxe.efi (72%)",
      last_seen_secs_ago: 2,
    },
    {
      mac: "9C:6B:00:8F:2A:11",
      ip: "192.168.1.204",
      arch: "UEFI ARM64",
      stage: { HttpBoot: "/alpine/initramfs-virt" },
      stage_text: "HTTP /alpine/initramfs-virt",
      last_seen_secs_ago: 7,
    },
    {
      mac: "30:9C:23:40:AA:18",
      ip: "192.168.1.197",
      arch: "Legacy BIOS",
      stage: "Booted",
      stage_text: "Booted",
      last_seen_secs_ago: 41,
    },
    {
      mac: "B8:27:EB:AD:44:02",
      ip: null,
      arch: "Unknown",
      stage: "Discovering",
      stage_text: "Discovering (DHCP)",
      last_seen_secs_ago: 1,
    },
  ],
  events: [
    { timestamp: "14:31:08", level: "Info", protocol: "Dhcp", message: "DISCOVER from F0:79:59:70:DA:7F (UEFI x86_64)" },
    { timestamp: "14:31:08", level: "Success", protocol: "Dhcp", message: "OFFER 192.168.1.215 with bootfile ipxe.efi" },
    { timestamp: "14:31:10", level: "Info", protocol: "Tftp", message: "RRQ ipxe.efi from 192.168.1.215; blksize=1468" },
    { timestamp: "14:31:11", level: "Success", protocol: "Tftp", message: "Transferred ipxe.efi (1.1 MB) in 0.8s" },
    { timestamp: "14:31:14", level: "Info", protocol: "Http", message: "GET /alpine/vmlinuz-virt from 192.168.1.204" },
    { timestamp: "14:31:17", level: "Warn", protocol: "System", message: "Client B8:27:EB:AD:44:02 did not include architecture option" },
    { timestamp: "14:31:20", level: "Error", protocol: "Http", message: "Requested image was not found: /custom/rescue.iso" },
  ],
};

export const mockImages: BootImage[] = [
  { name: "vmlinuz-virt", rel_path: "alpine/vmlinuz-virt", category: "alpine", size_bytes: 11239424, size_formatted: "10.7 MB", is_iso: false, is_kernel: true, is_initrd: false },
  { name: "initramfs-virt", rel_path: "alpine/initramfs-virt", category: "alpine", size_bytes: 18487296, size_formatted: "17.6 MB", is_iso: false, is_kernel: false, is_initrd: true },
  { name: "linux", rel_path: "debian/linux", category: "debian", size_bytes: 8372224, size_formatted: "8.0 MB", is_iso: false, is_kernel: true, is_initrd: false },
  { name: "initrd.gz", rel_path: "debian/initrd.gz", category: "debian", size_bytes: 39118848, size_formatted: "37.3 MB", is_iso: false, is_kernel: false, is_initrd: true },
  { name: "netboot.xyz.efi", rel_path: "tools/netboot.xyz.efi", category: "tools", size_bytes: 1190400, size_formatted: "1.1 MB", is_iso: false, is_kernel: false, is_initrd: false },
  { name: "ubuntu-24.04.4-live-server-amd64.iso", rel_path: "ubuntu/ubuntu-24.04.4-live-server-amd64.iso", category: "ubuntu", size_bytes: 3273654272, size_formatted: "3.05 GB", is_iso: true, is_kernel: false, is_initrd: false },
];

export const mockPresets: ImagePreset[] = [
  { id: "alpine", name: "Alpine Linux 3.20", category: "alpine", description: "Small RAM-boot environment for fast provisioning and recovery.", size_estimate: "~22 MB", files: [] },
  { id: "debian", name: "Debian 12 Bookworm", category: "debian", description: "Official Debian network installer kernel and ramdisk.", size_estimate: "~45 MB", files: [] },
  { id: "memtest", name: "Memtest86+", category: "tools", description: "Hardware memory diagnostics for UEFI and BIOS systems.", size_estimate: "~2 MB", files: [] },
  { id: "netboot_xyz", name: "netboot.xyz", category: "tools", description: "Chainload a maintained catalog of operating system installers.", size_estimate: "~2.5 MB", files: [] },
];

export const mockScripts = {
  boot: `#!ipxe\nset server_ip 192.168.1.58\nset base_url http://\${server_ip}:8080\n\n:start\nmenu PXE Netboot Menu\nitem alpine Alpine Linux 3.20\nitem debian Debian 12 Bookworm\nitem netboot_xyz netboot.xyz catalog\nchoose --timeout 15000 --default alpine target && goto \${target}\n`,
  autoexec: `#!ipxe\ndhcp\nchain --autofree http://192.168.1.58:8080/boot.ipxe || shell\n`,
};
