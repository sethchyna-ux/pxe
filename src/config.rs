use clap::{Parser, Subcommand, ValueEnum};
use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(name = "pxe", author, version, about = "High-Performance Rust PXE & Netboot Server")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Operating mode: 'proxy' (ProxyDHCP coexists with existing router) or 'standalone' (authoritative DHCP)
    #[arg(short, long, value_enum, default_value_t = ServerMode::Proxy, global = true)]
    pub mode: ServerMode,

    /// IP address of this PXE server (auto-detected if omitted)
    #[arg(short, long, global = true)]
    pub bind_ip: Option<Ipv4Addr>,

    /// DHCP listening UDP port (requires sudo/root for standard port 67)
    #[arg(long, default_value_t = 67, global = true)]
    pub dhcp_port: u16,

    /// ProxyDHCP listening UDP port
    #[arg(long, default_value_t = 4011, global = true)]
    pub proxy_port: u16,

    /// TFTP listening UDP port (requires sudo/root for standard port 69)
    #[arg(long, default_value_t = 69, global = true)]
    pub tftp_port: u16,

    /// HTTP listening TCP port for iPXE scripts and kernels/ISOs
    #[arg(long, default_value_t = 8080, global = true)]
    pub http_port: u16,

    /// Root directory for TFTP boot files
    #[arg(long, default_value = "./tftpboot", global = true)]
    pub tftp_dir: PathBuf,

    /// Root directory for HTTP assets (kernels, initrds, ISOs, boot.ipxe)
    #[arg(long, default_value = "./httpboot", global = true)]
    pub http_dir: PathBuf,

    /// Disable Ratatui interactive TUI dashboard and use stdout logging
    #[arg(long, default_value_t = false, global = true)]
    pub no_tui: bool,

    /// Standalone DHCP IP pool range: START,END (e.g. 192.168.1.200,192.168.1.250)
    #[arg(long, value_parser = parse_ip_range, global = true)]
    pub dhcp_range: Option<(Ipv4Addr, Ipv4Addr)>,

    /// Standalone DHCP Subnet mask
    #[arg(long, default_value = "255.255.255.0", global = true)]
    pub dhcp_mask: Ipv4Addr,

    /// Standalone DHCP Gateway / Router IP
    #[arg(long, global = true)]
    pub dhcp_router: Option<Ipv4Addr>,

    /// Standalone DHCP DNS Server IP
    #[arg(long, global = true)]
    pub dhcp_dns: Option<Ipv4Addr>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Start the PXE server daemon
    Serve,
    /// Initialize workspace directory structure (tftpboot, httpboot, default boot.ipxe menu)
    Init {
        /// Target directory to initialize
        #[arg(default_value = ".")]
        target_dir: PathBuf,
    },
    /// Inspect local network interfaces and show auto-detected IPs
    Status,
    /// Run full end-to-end PXE boot simulation (ASUS Z97-A Intel I218-V UEFI & Legacy BIOS)
    Simulate,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerMode {
    Proxy,
    Standalone,
}

impl std::fmt::Display for ServerMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Proxy => write!(f, "ProxyDHCP"),
            Self::Standalone => write!(f, "Standalone"),
        }
    }
}

fn parse_ip_range(s: &str) -> Result<(Ipv4Addr, Ipv4Addr), String> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 2 {
        return Err("Expected START_IP,END_IP format".to_string());
    }
    let start: Ipv4Addr = parts[0]
        .trim()
        .parse()
        .map_err(|e| format!("Invalid start IP: {e}"))?;
    let end: Ipv4Addr = parts[1]
        .trim()
        .parse()
        .map_err(|e| format!("Invalid end IP: {e}"))?;
    if u32::from(start) >= u32::from(end) {
        return Err("Start IP must be less than end IP".to_string());
    }
    Ok((start, end))
}

pub fn list_network_interfaces_detailed() -> Vec<(String, Ipv4Addr, Ipv4Addr)> {
    let mut results = Vec::new();
    #[cfg(unix)]
    unsafe {
        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifap) == 0 {
            let mut cur = ifap;
            while !cur.is_null() {
                let ifa = &*cur;
                let flags = ifa.ifa_flags as libc::c_int;
                let is_up = (flags & libc::IFF_UP) != 0;
                let is_loopback = (flags & libc::IFF_LOOPBACK) != 0;

                if is_up
                    && !is_loopback
                    && !ifa.ifa_addr.is_null()
                    && (*ifa.ifa_addr).sa_family as libc::c_int == libc::AF_INET
                {
                    let sin = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                    let ip = Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
                    let mask = if !ifa.ifa_netmask.is_null()
                        && (*ifa.ifa_netmask).sa_family as libc::c_int == libc::AF_INET
                    {
                        let sin_mask = &*(ifa.ifa_netmask as *const libc::sockaddr_in);
                        Ipv4Addr::from(u32::from_be(sin_mask.sin_addr.s_addr))
                    } else {
                        Ipv4Addr::new(255, 255, 255, 0)
                    };
                    if !ip.is_loopback() && !ip.is_link_local() {
                        let name = std::ffi::CStr::from_ptr(ifa.ifa_name)
                            .to_string_lossy()
                            .to_string();
                        results.push((name, ip, mask));
                    }
                }
                cur = ifa.ifa_next;
            }
            libc::freeifaddrs(ifap);
        }
    }
    results
}

pub fn list_network_interfaces() -> Vec<(String, Ipv4Addr)> {
    list_network_interfaces_detailed()
        .into_iter()
        .map(|(name, ip, _)| (name, ip))
        .collect()
}

pub fn detect_interface_netmask(target_ip: Ipv4Addr) -> Option<Ipv4Addr> {
    list_network_interfaces_detailed()
        .into_iter()
        .find(|(_, ip, _)| *ip == target_ip)
        .map(|(_, _, mask)| mask)
}

pub fn detect_default_route_ipv4() -> Option<Ipv4Addr> {
    if let Ok(socket) = UdpSocket::bind("0.0.0.0:0")
        && socket.connect("1.1.1.1:80").is_ok()
        && let Ok(local_addr) = socket.local_addr()
        && let IpAddr::V4(ipv4) = local_addr.ip()
        && !ipv4.is_loopback()
    {
        return Some(ipv4);
    }
    None
}

/// Returns true if `server_ip` is on a secondary/direct-cable Ethernet interface
/// that does not share a subnet with the host's default internet gateway route.
pub fn is_direct_ethernet_link(server_ip: Ipv4Addr) -> bool {
    if server_ip.is_loopback() || server_ip.is_unspecified() {
        return false;
    }
    match detect_default_route_ipv4() {
        Some(default_ip) => default_ip != server_ip,
        None => true, // No internet default route at all -> isolated offline link
    }
}

pub fn detect_primary_ipv4() -> Option<Ipv4Addr> {
    // 1. Prefer the interface carrying the active default LAN/gateway route (e.g. 192.168.1.x)
    if let Some(ipv4) = detect_default_route_ipv4() {
        return Some(ipv4);
    }

    let interfaces = list_network_interfaces();

    // 2. Prefer wired Ethernet interfaces if no default route is available
    for (name, ip) in &interfaces {
        let is_linux_wired = name.starts_with("eno")
            || name.starts_with("eth")
            || name.starts_with("enp")
            || name.starts_with("enx")
            || name.starts_with("usb");
        let is_macos_wired = cfg!(target_os = "macos")
            && name.starts_with("en")
            && name != "en0"
            && name[2..].chars().all(|c| c.is_ascii_digit());
        if is_linux_wired || is_macos_wired {
            return Some(*ip);
        }
    }

    // 3. Offline fallback
    interfaces.first().map(|(_, ip)| *ip)
}

/// Given a client's IPv4 address, find the local interface IPv4 address on the same subnet.
pub fn resolve_server_ip_for_client(client_ip: Ipv4Addr, fallback_ip: Ipv4Addr) -> Ipv4Addr {
    if client_ip.is_loopback() || client_ip.is_unspecified() {
        return fallback_ip;
    }
    let client_u32 = u32::from(client_ip);
    for (_, ip, mask) in list_network_interfaces_detailed() {
        let mask_u32 = u32::from(mask);
        if (u32::from(ip) & mask_u32) == (client_u32 & mask_u32) {
            return ip;
        }
    }
    fallback_ip
}
