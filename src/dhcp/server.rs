use crate::config::ServerMode;
use crate::dhcp::packet::{
    ClientArch, DhcpMessageType, DhcpPacket, BOOTREPLY, HLEN_ETHER, HTYPE_ETHER,
};
use crate::error::{DhcpError, Result};
use crate::state::{BootStage, EventLevel, Protocol, SharedState};
use socket2::{Domain, Protocol as SocketProto, Socket, Type};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

#[derive(Debug, Clone)]
pub struct DhcpConfig {
    pub server_ip: Ipv4Addr,
    pub mode: ServerMode,
    pub dhcp_port: u16,
    pub proxy_port: u16,
    pub http_port: u16,
    pub subnet_mask: Ipv4Addr,
    pub router: Option<Ipv4Addr>,
    pub dns: Option<Ipv4Addr>,
    pub pool_range: Option<(Ipv4Addr, Ipv4Addr)>,
}

pub struct DhcpServer {
    state: Arc<SharedState>,
    server_ip: Ipv4Addr,
    mode: ServerMode,
    dhcp_port: u16,
    proxy_port: u16,
    http_port: u16,
    subnet_mask: Ipv4Addr,
    router: Option<Ipv4Addr>,
    dns: Option<Ipv4Addr>,
    pool_range: Option<(Ipv4Addr, Ipv4Addr)>,
    leases: Mutex<HashMap<String, (Ipv4Addr, Instant)>>,
}

impl DhcpServer {
    pub fn new(state: Arc<SharedState>, config: DhcpConfig) -> Self {
        Self {
            state,
            server_ip: config.server_ip,
            mode: config.mode,
            dhcp_port: config.dhcp_port,
            proxy_port: config.proxy_port,
            http_port: config.http_port,
            subnet_mask: config.subnet_mask,
            router: config.router,
            dns: config.dns,
            pool_range: config.pool_range,
            leases: Mutex::new(HashMap::new()),
        }
    }

    fn create_udp_socket(port: u16, _server_ip: Ipv4Addr) -> std::io::Result<UdpSocket> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(SocketProto::UDP))?;
        socket.set_broadcast(true)?;
        socket.set_reuse_address(true)?;
        #[cfg(unix)]
        unsafe {
            use std::os::fd::AsRawFd;
            let optval: libc::c_int = 1;
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_REUSEPORT,
                &optval as *const _ as *const libc::c_void,
                std::mem::size_of_val(&optval) as libc::socklen_t,
            );
        }
        #[cfg(target_os = "macos")]
        if !_server_ip.is_loopback() && !_server_ip.is_unspecified() {
            for (if_name, ip) in crate::config::list_network_interfaces() {
                if ip == _server_ip {
                    if let Ok(c_name) = std::ffi::CString::new(if_name) {
                        unsafe {
                            use std::os::fd::AsRawFd;
                            let idx = libc::if_nametoindex(c_name.as_ptr());
                            if idx != 0 {
                                const IP_BOUND_IF: libc::c_int = 25;
                                libc::setsockopt(
                                    socket.as_raw_fd(),
                                    libc::IPPROTO_IP,
                                    IP_BOUND_IF,
                                    &idx as *const _ as *const libc::c_void,
                                    std::mem::size_of_val(&idx) as libc::socklen_t,
                                );
                            }
                        }
                    }
                    break;
                }
            }
        }
        socket.set_nonblocking(true)?;

        let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
        socket.bind(&bind_addr.into())?;

        let std_socket: std::net::UdpSocket = socket.into();
        UdpSocket::from_std(std_socket)
    }

    pub async fn run(self: Arc<Self>) -> Result<()> {
        let dhcp_sock = match Self::create_udp_socket(self.dhcp_port, self.server_ip) {
            Ok(s) => s,
            Err(e) => {
                #[allow(unused_mut)]
                let mut msg = format!(
                    "Failed to bind DHCP port {}: {e}",
                    self.dhcp_port
                );
                #[cfg(target_os = "macos")]
                if e.raw_os_error() == Some(48) && self.dhcp_port == 67 {
                    msg.push_str(" -> Port 67 is held by macOS built-in bootpd daemon. Free it with: sudo launchctl bootout system/com.apple.bootpd");
                }
                self.state.log(EventLevel::Error, Protocol::Dhcp, &msg);
                return Err(DhcpError::Socket(e).into());
            }
        };

        self.state.log(
            EventLevel::Success,
            Protocol::Dhcp,
            format!(
                "DHCP engine listening on UDP {} [{}]",
                self.dhcp_port, self.mode
            ),
        );

        // Optional secondary socket for ProxyDHCP port (4011) if in proxy mode
        let proxy_sock = if self.mode == ServerMode::Proxy && self.proxy_port != self.dhcp_port {
            match Self::create_udp_socket(self.proxy_port, self.server_ip) {
                Ok(s) => {
                    self.state.log(
                        EventLevel::Success,
                        Protocol::Dhcp,
                        format!("ProxyDHCP listener active on UDP {}", self.proxy_port),
                    );
                    Some(s)
                }
                Err(e) => {
                    self.state.log(
                        EventLevel::Warn,
                        Protocol::Dhcp,
                        format!("Could not bind ProxyDHCP port {}: {e}", self.proxy_port),
                    );
                    None
                }
            }
        } else {
            None
        };

        let this_main = Arc::clone(&self);
        let main_task = tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            loop {
                match dhcp_sock.recv_from(&mut buf).await {
                    Ok((len, src)) => {
                        this_main.state.record_dhcp_in();
                        if let Err(e) = this_main.handle_packet(&dhcp_sock, &buf[..len], src, false).await {
                            this_main.state.record_error();
                            this_main.state.log(
                                EventLevel::Warn,
                                Protocol::Dhcp,
                                format!("Error processing DHCP from {src}: {e}"),
                            );
                        }
                    }
                    Err(e) => {
                        this_main.state.record_error();
                        this_main.state.log(
                            EventLevel::Error,
                            Protocol::Dhcp,
                            format!("DHCP recv error: {e}"),
                        );
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            }
        });

        let proxy_task = if let Some(sock) = proxy_sock {
            let this_proxy = Arc::clone(&self);
            Some(tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                loop {
                    match sock.recv_from(&mut buf).await {
                        Ok((len, src)) => {
                            this_proxy.state.record_dhcp_in();
                            if let Err(e) = this_proxy.handle_packet(&sock, &buf[..len], src, true).await {
                                this_proxy.state.record_error();
                                this_proxy.state.log(
                                    EventLevel::Warn,
                                    Protocol::Dhcp,
                                    format!("ProxyDHCP packet error from {src}: {e}"),
                                );
                            }
                        }
                        Err(_e) => {
                            this_proxy.state.record_error();
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    }
                }
            }))
        } else {
            None
        };

        if let Some(pt) = proxy_task {
            tokio::select! {
                _ = main_task => {},
                _ = pt => {},
            }
        } else {
            main_task.await.ok();
        }

        Ok(())
    }

    async fn handle_packet(
        &self,
        sock: &UdpSocket,
        buf: &[u8],
        src: SocketAddr,
        is_proxy_port: bool,
    ) -> Result<()> {
        let packet = match DhcpPacket::parse(buf) {
            Ok(p) => p,
            Err(e) => {
                return Err(e.into());
            }
        };

        let msg_type = match packet.message_type() {
            Some(t) => t,
            None => return Ok(()), // Ignore non-DHCP BOOTP packets
        };

        let mac = packet.mac_string();
        let arch = packet.client_arch();
        let is_ipxe = packet.is_ipxe();

        let observed_ip = if !packet.ciaddr.is_unspecified() {
            Some(packet.ciaddr)
        } else if let Some(req_ip) = packet.requested_ip() {
            Some(req_ip)
        } else if let SocketAddr::V4(v4) = src {
            if !v4.ip().is_unspecified() && !v4.ip().is_broadcast() {
                Some(*v4.ip())
            } else {
                None
            }
        } else {
            None
        };

        match msg_type {
            DhcpMessageType::Discover => {
                let arch_desc = arch.display_name();
                let ipxe_suffix = if is_ipxe { " (iPXE Stage 2)" } else { "" };
                self.state.log(
                    EventLevel::Info,
                    Protocol::Dhcp,
                    format!("DHCPDISCOVER from {mac} [{arch_desc}{ipxe_suffix}]"),
                );

                self.state.update_client(
                    &mac,
                    observed_ip,
                    Some(arch.display_name()),
                    BootStage::Discovering,
                );

                if let Some(reply) = self.build_offer(&packet, is_ipxe, arch, observed_ip)? {
                    self.send_reply(sock, &reply, src, is_proxy_port).await?;
                }
            }
            DhcpMessageType::Request | DhcpMessageType::Inform => {
                // Check if server identifier in request matches ours or is unassigned
                let is_for_us = match packet.options.get(&54) {
                    Some(s) if s.len() == 4 => {
                        let req_server = Ipv4Addr::new(s[0], s[1], s[2], s[3]);
                        req_server == self.server_ip
                    }
                    _ => true,
                };

                if is_for_us || self.mode == ServerMode::Proxy {
                    let arch_desc = arch.display_name();
                    let port_tag = if is_proxy_port { " (ProxyDHCP :4011)" } else { "" };
                    self.state.log(
                        EventLevel::Info,
                        Protocol::Dhcp,
                        format!("DHCPREQUEST from {mac} [{arch_desc}{port_tag}]"),
                    );

                    if let Some(reply) = self.build_ack(&packet, is_ipxe, arch, observed_ip)? {
                        self.send_reply(sock, &reply, src, is_proxy_port).await?;
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn populate_boot_fields(&self, reply: &mut DhcpPacket, bootfile: &str) {
        // Populate 64-byte server host name (sname) with server IP string
        let sname_str = self.server_ip.to_string();
        let sname_bytes = sname_str.as_bytes();
        let sname_len = sname_bytes.len().min(63);
        reply.sname[..sname_len].copy_from_slice(&sname_bytes[..sname_len]);

        // Populate 128-byte bootfile field in fixed header
        let bootfile_bytes = bootfile.as_bytes();
        let copy_len = bootfile_bytes.len().min(127);
        reply.file[..copy_len].copy_from_slice(&bootfile_bytes[..copy_len]);

        // Option 66: TFTP Server Name / IP (RFC 2132 length-delimited string, no trailing null inside TLV)
        reply
            .options
            .insert(66, sname_str.into_bytes());

        // Option 67: Bootfile Name (no trailing null inside TLV so iPXE HTTP URLs & strict UEFI parsers succeed)
        reply
            .options
            .insert(67, bootfile.as_bytes().to_vec());
    }

    fn populate_pxe_vendor_options(&self, req: &DhcpPacket, reply: &mut DhcpPacket) {
        // Option 60: Vendor Class Identifier = "PXEClient" (Essential for ProxyDHCP & Intel Boot Agent / UEFI PXE)
        reply.options.insert(60, b"PXEClient".to_vec());

        // Option 43: Vendor-Specific Information -> PXE_DISCOVERY_CONTROL (Sub-option 6 = 8)
        // Bit 3 (0x08): Disable broadcast/multicast bootserver discovery and use bootfile from DHCP/ProxyDHCP packet directly.
        // Required by Intel I218-V Boot Agent GE & AMI Aptio IV UEFI PXE on ASUS Z97-A.
        reply.options.insert(43, vec![6, 1, 8, 255]);

        // Echo client machine UUID (Option 97)
        if let Some(uuid) = req.client_uuid() {
            reply.options.insert(97, uuid);
        }
    }

    fn build_offer(
        &self,
        req: &DhcpPacket,
        is_ipxe: bool,
        arch: ClientArch,
        observed_ip: Option<Ipv4Addr>,
    ) -> Result<Option<DhcpPacket>> {
        let (yiaddr, lease_time): (Ipv4Addr, u32) = match self.mode {
            ServerMode::Proxy => {
                // ProxyDHCP does NOT allocate IP (client gets IP from router)
                (Ipv4Addr::UNSPECIFIED, 0)
            }
            ServerMode::Standalone => {
                let allocated_ip = self.allocate_ip(&req.mac_string())?;
                (allocated_ip, 86400)
            }
        };

        let mut reply = DhcpPacket {
            op: BOOTREPLY,
            htype: HTYPE_ETHER,
            hlen: HLEN_ETHER,
            hops: 0,
            xid: req.xid,
            secs: 0,
            flags: req.flags,
            ciaddr: Ipv4Addr::UNSPECIFIED,
            yiaddr,
            siaddr: self.server_ip,
            giaddr: req.giaddr,
            chaddr: req.chaddr,
            sname: [0u8; 64],
            file: [0u8; 128],
            options: HashMap::new(),
        };

        // Determine Bootfile and Next Server
        let bootfile = if is_ipxe {
            format!("http://{}:{}/boot.ipxe", self.server_ip, self.http_port)
        } else {
            arch.default_bootfile().to_string()
        };

        self.populate_boot_fields(&mut reply, &bootfile);

        // Option 53: DHCP Offer
        reply.options.insert(53, vec![DhcpMessageType::Offer as u8]);

        // Option 54: Server Identifier
        reply
            .options
            .insert(54, self.server_ip.octets().to_vec());

        if self.mode == ServerMode::Proxy {
            self.populate_pxe_vendor_options(req, &mut reply);
        } else {
            // Standalone mode options
            reply
                .options
                .insert(51, lease_time.to_be_bytes().to_vec());
            reply
                .options
                .insert(1, self.subnet_mask.octets().to_vec());

            if let Some(r) = self.router {
                reply.options.insert(3, r.octets().to_vec());
            }
            if let Some(d) = self.dns {
                reply.options.insert(6, d.octets().to_vec());
            }
            if let Some(uuid) = req.client_uuid() {
                reply.options.insert(97, uuid);
            }
        }

        let tracked_ip = if !yiaddr.is_unspecified() {
            Some(yiaddr)
        } else {
            observed_ip
        };

        self.state.update_client(
            &req.mac_string(),
            tracked_ip,
            Some(arch.display_name()),
            BootStage::Offered(tracked_ip.unwrap_or(yiaddr)),
        );

        Ok(Some(reply))
    }

    fn build_ack(
        &self,
        req: &DhcpPacket,
        is_ipxe: bool,
        arch: ClientArch,
        observed_ip: Option<Ipv4Addr>,
    ) -> Result<Option<DhcpPacket>> {
        let (yiaddr, lease_time): (Ipv4Addr, u32) = match self.mode {
            ServerMode::Proxy => {
                (req.ciaddr, 0)
            }
            ServerMode::Standalone => {
                let ip = self.allocate_ip(&req.mac_string())?;
                (ip, 86400)
            }
        };

        let mut reply = DhcpPacket {
            op: BOOTREPLY,
            htype: HTYPE_ETHER,
            hlen: HLEN_ETHER,
            hops: 0,
            xid: req.xid,
            secs: 0,
            flags: req.flags,
            ciaddr: req.ciaddr,
            yiaddr,
            siaddr: self.server_ip,
            giaddr: req.giaddr,
            chaddr: req.chaddr,
            sname: [0u8; 64],
            file: [0u8; 128],
            options: HashMap::new(),
        };

        let bootfile = if is_ipxe {
            format!("http://{}:{}/boot.ipxe", self.server_ip, self.http_port)
        } else {
            arch.default_bootfile().to_string()
        };

        self.populate_boot_fields(&mut reply, &bootfile);

        // Option 53: DHCP ACK
        reply.options.insert(53, vec![DhcpMessageType::Ack as u8]);

        // Option 54: Server Identifier
        reply
            .options
            .insert(54, self.server_ip.octets().to_vec());

        if self.mode == ServerMode::Proxy {
            self.populate_pxe_vendor_options(req, &mut reply);
        } else {
            reply
                .options
                .insert(51, lease_time.to_be_bytes().to_vec());
            reply
                .options
                .insert(1, self.subnet_mask.octets().to_vec());
            if let Some(r) = self.router {
                reply.options.insert(3, r.octets().to_vec());
            }
            if let Some(d) = self.dns {
                reply.options.insert(6, d.octets().to_vec());
            }
            if let Some(uuid) = req.client_uuid() {
                reply.options.insert(97, uuid);
            }
        }

        let tracked_ip = if !yiaddr.is_unspecified() {
            Some(yiaddr)
        } else {
            observed_ip
        };

        self.state.update_client(
            &req.mac_string(),
            tracked_ip,
            Some(arch.display_name()),
            BootStage::Offered(tracked_ip.unwrap_or(yiaddr)),
        );

        Ok(Some(reply))
    }

    async fn send_reply(
        &self,
        sock: &UdpSocket,
        reply: &DhcpPacket,
        src: SocketAddr,
        is_proxy_port: bool,
    ) -> Result<()> {
        let bytes = reply.serialize();

        // Determine destination:
        // 1. ProxyDHCP port (4011) or non-standard port simulation: reply directly to unicast peer if IP is known
        // 2. Unicast request (broadcast bit 0x8000 clear and valid client IP): reply directly to src
        // 3. Broadcast request on standard DHCP port 67: send to 255.255.255.255:68
        let target_dest = if is_proxy_port && !src.ip().is_unspecified() {
            src
        } else if !src.ip().is_unspecified() && (reply.flags & 0x8000 == 0 || self.dhcp_port != 67) {
            src
        } else if !reply.ciaddr.is_unspecified() && reply.flags & 0x8000 == 0 {
            SocketAddr::V4(SocketAddrV4::new(reply.ciaddr, 68))
        } else {
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, 68))
        };

        let subnet_bcast = Ipv4Addr::from(u32::from(self.server_ip) | !u32::from(self.subnet_mask));
        let subnet_dest = SocketAddr::V4(SocketAddrV4::new(subnet_bcast, 68));

        let send_res = match sock.send_to(&bytes, target_dest).await {
            Ok(n) => {
                // On multi-homed hosts (e.g. Mac with Wi-Fi + USB Ethernet, or Linux with wlan0 + eno1),
                // also transmit to subnet-directed broadcast (e.g. 192.168.1.255:68) so the kernel
                // routes the packet out the exact subnet interface even if 255.255.255.255 went to default route.
                if target_dest.ip() == Ipv4Addr::BROADCAST
                    && !self.server_ip.is_loopback()
                    && subnet_bcast != Ipv4Addr::BROADCAST
                {
                    let _ = sock.send_to(&bytes, subnet_dest).await;
                }
                Ok((n, target_dest))
            }
            Err(e) if target_dest.ip() == Ipv4Addr::BROADCAST && subnet_bcast != Ipv4Addr::BROADCAST => {
                // Offline direct-cable fallback when 255.255.255.255 returns EHOSTUNREACH / ENETUNREACH
                sock.send_to(&bytes, subnet_dest)
                    .await
                    .map(|n| (n, subnet_dest))
                    .map_err(|_| e)
            }
            Err(e) => Err(e),
        };

        match send_res {
            Ok((_, actual_dest)) => {
                self.state.record_dhcp_out();
                let mac = reply.mac_string();
                let msg_name = match reply.message_type() {
                    Some(DhcpMessageType::Offer) => "DHCPOFFER",
                    Some(DhcpMessageType::Ack) => "DHCPACK",
                    _ => "REPLY",
                };
                self.state.log(
                    EventLevel::Success,
                    Protocol::Dhcp,
                    format!("Sent {msg_name} to {mac} via {actual_dest}"),
                );
                Ok(())
            }
            Err(e) => {
                self.state.record_error();
                Err(DhcpError::Socket(e).into())
            }
        }
    }

    fn allocate_ip(&self, mac: &str) -> Result<Ipv4Addr> {
        let (start, end) = self.pool_range.ok_or_else(|| {
            crate::error::PxeError::Config(
                "Standalone DHCP mode requested but no --dhcp-range specified".to_string(),
            )
        })?;

        let mut lock = self.leases.lock().map_err(|_| {
            crate::error::PxeError::Config("Lease lock poisoned".to_string())
        })?;

        // Check if MAC already has a valid lease
        if let Some((ip, _)) = lock.get(mac) {
            return Ok(*ip);
        }

        // Find first free IP in range
        let start_u32 = u32::from(start);
        let end_u32 = u32::from(end);

        let active_ips: Vec<Ipv4Addr> = lock.values().map(|(ip, _)| *ip).collect();

        for candidate_u32 in start_u32..=end_u32 {
            let candidate = Ipv4Addr::from(candidate_u32);
            if !active_ips.contains(&candidate) {
                lock.insert(mac.to_string(), (candidate, Instant::now()));
                return Ok(candidate);
            }
        }

        Err(DhcpError::PoolExhausted.into())
    }
}
