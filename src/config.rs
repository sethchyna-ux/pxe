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

pub fn list_network_interfaces() -> Vec<(String, Ipv4Addr)> {
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

                if is_up && !is_loopback && !ifa.ifa_addr.is_null() {
                    if (*ifa.ifa_addr).sa_family as libc::c_int == libc::AF_INET {
                        let sin = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                        let ip = Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
                        if !ip.is_loopback() && !ip.is_link_local() {
                            let name = std::ffi::CStr::from_ptr(ifa.ifa_name)
                                .to_string_lossy()
                                .to_string();
                            results.push((name, ip));
                        }
                    }
                }
                cur = ifa.ifa_next;
            }
            libc::freeifaddrs(ifap);
        }
    }
    results
}

pub fn detect_primary_ipv4() -> Option<Ipv4Addr> {
    let interfaces = list_network_interfaces();

    // 1. Prefer wired Ethernet interfaces (eno*, eth*, enp*, enx*, usb*) if present
    for (name, ip) in &interfaces {
        if name.starts_with("eno")
            || name.starts_with("eth")
            || name.starts_with("enp")
            || name.starts_with("enx")
            || name.starts_with("usb")
        {
            return Some(*ip);
        }
    }

    // 2. Attempt UDP routing probe if internet/gateway route is available
    if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("1.1.1.1:80").is_ok() {
            if let Ok(local_addr) = socket.local_addr() {
                if let IpAddr::V4(ipv4) = local_addr.ip() {
                    if !ipv4.is_loopback() {
                        return Some(ipv4);
                    }
                }
            }
        }
    }

    // 3. Offline fallback (e.g. Mac or Android phone plugged directly into ASUS Z97-A via Ethernet cable)
    interfaces.first().map(|(_, ip)| *ip)
}
