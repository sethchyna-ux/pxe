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

    fn create_udp_socket(port: u16) -> std::io::Result<UdpSocket> {
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
        socket.set_nonblocking(true)?;

        let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
        socket.bind(&bind_addr.into())?;

        let std_socket: std::net::UdpSocket = socket.into();
        UdpSocket::from_std(std_socket)
    }

    pub async fn run(self: Arc<Self>) -> Result<()> {
        let dhcp_sock = match Self::create_udp_socket(self.dhcp_port) {
            Ok(s) => s,
            Err(e) => {
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
            match Self::create_udp_socket(self.proxy_port) {
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
                        if let Err(e) = this_main.handle_packet(&dhcp_sock, &buf[..len], src).await {
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
                            if let Err(e) = this_proxy.handle_packet(&sock, &buf[..len], src).await {
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
                    None,
                    Some(arch.display_name()),
                    BootStage::Discovering,
                );

                if let Some(reply) = self.build_offer(&packet, is_ipxe, arch)? {
                    self.send_reply(sock, &reply, src).await?;
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
                    self.state.log(
                        EventLevel::Info,
                        Protocol::Dhcp,
                        format!("DHCPREQUEST from {mac} [{arch_desc}]"),
                    );

                    if let Some(reply) = self.build_ack(&packet, is_ipxe, arch)? {
                        self.send_reply(sock, &reply, src).await?;
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn build_offer(
        &self,
        req: &DhcpPacket,
        is_ipxe: bool,
        arch: ClientArch,
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

        // Copy bootfile into 'file' field
        let bootfile_bytes = bootfile.as_bytes();
        let copy_len = bootfile_bytes.len().min(127);
        reply.file[..copy_len].copy_from_slice(&bootfile_bytes[..copy_len]);

        // Option 53: DHCP Offer
        reply.options.insert(53, vec![DhcpMessageType::Offer as u8]);

        // Option 54: Server Identifier
        reply
            .options
            .insert(54, self.server_ip.octets().to_vec());

        if self.mode == ServerMode::Proxy {
            // Option 60: Vendor Class Identifier = "PXEClient" (Essential for ProxyDHCP!)
            reply
                .options
                .insert(60, b"PXEClient".to_vec());

            // Echo client machine UUID (Option 97)
            if let Some(uuid) = req.client_uuid() {
                reply.options.insert(97, uuid);
            }
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
        }

        // Option 66: TFTP Server Name / IP
        reply
            .options
            .insert(66, self.server_ip.to_string().into_bytes());

        // Option 67: Bootfile Name
        reply
            .options
            .insert(67, bootfile.into_bytes());

        self.state.update_client(
            &req.mac_string(),
            if yiaddr.is_unspecified() { None } else { Some(yiaddr) },
            Some(arch.display_name()),
            BootStage::Offered(yiaddr),
        );

        Ok(Some(reply))
    }

    fn build_ack(
        &self,
        req: &DhcpPacket,
        is_ipxe: bool,
        arch: ClientArch,
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

        let bootfile_bytes = bootfile.as_bytes();
        let copy_len = bootfile_bytes.len().min(127);
        reply.file[..copy_len].copy_from_slice(&bootfile_bytes[..copy_len]);

        // Option 53: DHCP ACK
        reply.options.insert(53, vec![DhcpMessageType::Ack as u8]);

        // Option 54: Server Identifier
        reply
            .options
            .insert(54, self.server_ip.octets().to_vec());

        if self.mode == ServerMode::Proxy {
            reply.options.insert(60, b"PXEClient".to_vec());
            if let Some(uuid) = req.client_uuid() {
                reply.options.insert(97, uuid);
            }
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
        }

        // Option 66: TFTP Server Name
        reply
            .options
            .insert(66, self.server_ip.to_string().into_bytes());

        // Option 67: Bootfile Name
        reply
            .options
            .insert(67, bootfile.into_bytes());

        Ok(Some(reply))
    }

    async fn send_reply(
        &self,
        sock: &UdpSocket,
        reply: &DhcpPacket,
        src: SocketAddr,
    ) -> Result<()> {
        let bytes = reply.serialize();

        // Determine destination: broadcast 255.255.255.255:68 or specific client port
        let target_dest = if src.ip().is_unspecified() || reply.flags & 0x8000 != 0 {
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, 68))
        } else if src.port() == self.proxy_port {
            src
        } else {
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, 68))
        };

        match sock.send_to(&bytes, target_dest).await {
            Ok(_) => {
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
                    format!("Sent {msg_name} to {mac} via {target_dest}"),
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
