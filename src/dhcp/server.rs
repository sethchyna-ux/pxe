use crate::config::ServerMode;
use crate::dhcp::packet::{
    BOOTREPLY, ClientArch, DhcpMessageType, DhcpPacket, HLEN_ETHER, HTYPE_ETHER,
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
                                // Enable IP_ONESBCAST (23) on macOS so sending to the subnet-directed
                                // broadcast (10.77.255.255) routes via en4 AND rewrites the IPv4
                                // destination header on the wire to 255.255.255.255 (RFC 2131 / UEFI PXE).
                                const IP_ONESBCAST: libc::c_int = 23;
                                let one: libc::c_int = 1;
                                libc::setsockopt(
                                    socket.as_raw_fd(),
                                    libc::IPPROTO_IP,
                                    IP_ONESBCAST,
                                    &one as *const _ as *const libc::c_void,
                                    std::mem::size_of_val(&one) as libc::socklen_t,
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

    fn create_tx_socket(port: u16, server_ip: Ipv4Addr) -> std::io::Result<UdpSocket> {
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
        {
            for (if_name, ip) in crate::config::list_network_interfaces() {
                if ip == server_ip {
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
                                const IP_ONESBCAST: libc::c_int = 23;
                                let one: libc::c_int = 1;
                                libc::setsockopt(
                                    socket.as_raw_fd(),
                                    libc::IPPROTO_IP,
                                    IP_ONESBCAST,
                                    &one as *const _ as *const libc::c_void,
                                    std::mem::size_of_val(&one) as libc::socklen_t,
                                );
                            }
                        }
                    }
                    break;
                }
            }
        }
        socket.set_nonblocking(true)?;
        let bind_addr = SocketAddrV4::new(server_ip, port);
        socket.bind(&bind_addr.into())?;
        let std_socket: std::net::UdpSocket = socket.into();
        UdpSocket::from_std(std_socket)
    }

    pub async fn run(self: Arc<Self>) -> Result<()> {
        let dhcp_sock = match Self::create_udp_socket(self.dhcp_port, self.server_ip) {
            Ok(s) => s,
            Err(e) => {
                #[allow(unused_mut)]
                let mut msg = format!("Failed to bind DHCP port {}: {e}", self.dhcp_port);
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

        // Optional secondary socket for ProxyDHCP port (4011)
        let proxy_sock = if self.proxy_port != self.dhcp_port {
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
                        if let Err(e) = this_main
                            .handle_packet_on_if(&dhcp_sock, &buf[..len], src, false, None)
                            .await
                        {
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
                            if let Err(e) = this_proxy
                                .handle_packet_on_if(&sock, &buf[..len], src, true, None)
                                .await
                            {
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

    /// Resolve the active server IP, subnet mask, and mode for the interface that received the packet.
    fn resolve_interface_context(
        &self,
        recv_if: Option<&str>,
    ) -> Option<(Ipv4Addr, Ipv4Addr, ServerMode)> {
        if !self.server_ip.is_loopback()
            && let Some(if_name) = recv_if
        {
            // If we are bound to a specific direct Ethernet cable (e.g. 10.77.0.3 on en4),
            // ignore DHCP broadcasts bleeding in from Wi-Fi (en0) so Wi-Fi devices never steal direct-cable leases.
            if let Some((bound_if, _, _)) = crate::config::list_network_interfaces_detailed()
                .into_iter()
                .find(|(_, ip, _)| *ip == self.server_ip)
                && bound_if != if_name
            {
                return None;
            }

            for (name, ip, mask) in crate::config::list_network_interfaces_detailed() {
                if name == if_name {
                    let mode = if self.mode == ServerMode::Standalone
                        || crate::config::is_direct_ethernet_link(ip)
                    {
                        ServerMode::Standalone
                    } else {
                        ServerMode::Proxy
                    };
                    return Some((ip, mask, mode));
                }
            }
        }
        Some((self.server_ip, self.subnet_mask, self.mode))
    }

    async fn handle_packet_on_if(
        &self,
        sock: &UdpSocket,
        buf: &[u8],
        src: SocketAddr,
        is_proxy_port: bool,
        recv_if: Option<&str>,
    ) -> Result<()> {
        let Some((active_server_ip, active_mask, active_mode)) =
            self.resolve_interface_context(recv_if)
        else {
            return Ok(());
        };

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
        let is_pxe = packet.is_pxe_client();
        let if_tag = recv_if.map(|s| format!(" on {s}")).unwrap_or_default();

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
                // Ignore DHCPDISCOVER on ProxyDHCP port 4011 (Discover is only valid on port 67)
                if is_proxy_port {
                    return Ok(());
                }

                // Ignore non-PXE DHCPDISCOVER broadcasts from the Wi-Fi LAN (e.g. routers, phones, or printers
                // that do not include Option 60/93/94/77/175) so they never steal direct-cable PXE leases (10.77.0.2).
                if !is_pxe && !self.server_ip.is_loopback() {
                    return Ok(());
                }

                let arch_desc = arch.display_name();
                let ipxe_suffix = if is_ipxe { " (iPXE Stage 2)" } else { "" };
                let vc_tag = packet
                    .vendor_class()
                    .map(|v| format!(" vc=\"{v}\""))
                    .unwrap_or_default();
                let mut opt_keys: Vec<u8> = packet.options.keys().copied().collect();
                opt_keys.sort_unstable();
                self.state.log(
                    EventLevel::Info,
                    Protocol::Dhcp,
                    format!(
                        "DHCPDISCOVER from {mac}{if_tag} [{arch_desc}{ipxe_suffix}] xid={:#010x} flags={:#06x}{vc_tag} opts={:?}",
                        packet.xid, packet.flags, opt_keys
                    ),
                );

                self.state.update_client(
                    &mac,
                    observed_ip,
                    Some(arch.display_name()),
                    BootStage::Discovering,
                );

                if let Some(reply) = self.build_offer_with_ctx(
                    &packet,
                    is_ipxe,
                    arch,
                    observed_ip,
                    active_server_ip,
                    active_mask,
                    active_mode,
                )? {
                    self.send_reply(sock, &reply, src, is_proxy_port, recv_if, active_server_ip)
                        .await?;
                }
            }
            DhcpMessageType::Request | DhcpMessageType::Inform => {
                let req_server_opt = packet.options.get(&54).and_then(|s| {
                    if s.len() == 4 {
                        Some(Ipv4Addr::new(s[0], s[1], s[2], s[3]))
                    } else {
                        None
                    }
                });

                // On port 67, NEVER reply to DHCPREQUEST packets containing Option 54 (Server Identifier)
                // pointing to another server (such as the LAN router). Answering those broadcasts causes
                // clients to drop their valid router lease.
                let should_respond = if is_proxy_port {
                    true
                } else {
                    match req_server_opt {
                        Some(req_server) => {
                            req_server == active_server_ip || req_server == self.server_ip
                        }
                        None => {
                            if active_mode == ServerMode::Proxy {
                                is_pxe
                            } else {
                                true
                            }
                        }
                    }
                };

                if should_respond {
                    let arch_desc = arch.display_name();
                    let port_tag = if is_proxy_port {
                        " (ProxyDHCP :4011)"
                    } else {
                        ""
                    };
                    self.state.log(
                        EventLevel::Info,
                        Protocol::Dhcp,
                        format!("DHCPREQUEST from {mac}{if_tag} [{arch_desc}{port_tag}]"),
                    );

                    if let Some(reply) = self.build_ack_with_ctx(
                        &packet,
                        is_ipxe,
                        arch,
                        observed_ip,
                        active_server_ip,
                        active_mask,
                        active_mode,
                    )? {
                        self.send_reply(
                            sock,
                            &reply,
                            src,
                            is_proxy_port,
                            recv_if,
                            active_server_ip,
                        )
                        .await?;
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn populate_boot_fields(
        &self,
        reply: &mut DhcpPacket,
        bootfile: &str,
        server_ip: Ipv4Addr,
        is_ipxe: bool,
    ) {
        // Populate 64-byte server host name (sname) with server IP string
        let sname_str = server_ip.to_string();
        let sname_bytes = sname_str.as_bytes();
        let sname_len = sname_bytes.len().min(63);
        reply.sname[..sname_len].copy_from_slice(&sname_bytes[..sname_len]);

        // Populate 128-byte bootfile field in fixed header
        let bootfile_bytes = bootfile.as_bytes();
        let copy_len = bootfile_bytes.len().min(127);
        reply.file[..copy_len].copy_from_slice(&bootfile_bytes[..copy_len]);

        // Option 66: TFTP Server Name / IP (null-terminated for Stage 1 UEFI/BIOS ROMs; raw for Stage 2 iPXE)
        let mut opt66 = sname_str.into_bytes();
        if !is_ipxe {
            opt66.push(0);
        }
        reply.options.insert(66, opt66);

        // Option 67: Bootfile Name (null-terminated for Stage 1 AMI UEFI/BIOS ROMs; raw for Stage 2 iPXE HTTP URLs)
        let mut opt67 = bootfile.as_bytes().to_vec();
        if !is_ipxe {
            opt67.push(0);
        }
        reply.options.insert(67, opt67);
    }

    fn populate_pxe_vendor_options(
        &self,
        req: &DhcpPacket,
        reply: &mut DhcpPacket,
        arch: ClientArch,
        server_ip: Ipv4Addr,
    ) {
        // Option 60: Vendor Class Identifier = "PXEClient" (Essential for ProxyDHCP & Intel Boot Agent / UEFI PXE)
        reply.options.insert(60, b"PXEClient".to_vec());

        // Option 43: Vendor-Specific Information
        // Legacy BIOS clients (ClientArch::BiosX86) require sub-options 6, 8, 9, and 10 with strict TLV lengths.
        // UEFI PXE clients (EDK2 / AMI Aptio IV) do NOT use BIOS Option 43 menu sub-options.
        if arch == ClientArch::BiosX86 {
            let server_ip_bytes = server_ip.octets();
            let opt43 = vec![
                // Sub-option 6: Discovery Control (value 8 = bypass menu and use bootfile)
                6,
                1,
                8,
                // Sub-option 8: PXE Boot Server IP (type 8, length 7, payload 0,0,1,<server_ip>)
                8,
                7,
                0,
                0,
                1,
                server_ip_bytes[0],
                server_ip_bytes[1],
                server_ip_bytes[2],
                server_ip_bytes[3],
                // Sub-option 9: PXE Boot Menu (Type 0, Length 6, "PXE")
                9,
                6,
                0,
                0,
                3,
                80,
                88,
                69,
                // Sub-option 10: PXE Menu Prompt (Timeout 0, "PXE")
                10,
                4,
                0,
                80,
                88,
                69,
            ];
            reply.options.insert(43, opt43);
        }

        // Echo client machine UUID (Option 97)
        if let Some(uuid) = req.client_uuid() {
            reply.options.insert(97, uuid);
        }
    }

    #[cfg(test)]
    pub(crate) fn build_offer(
        &self,
        req: &DhcpPacket,
        is_ipxe: bool,
        arch: ClientArch,
        observed_ip: Option<Ipv4Addr>,
    ) -> Result<Option<DhcpPacket>> {
        self.build_offer_with_ctx(
            req,
            is_ipxe,
            arch,
            observed_ip,
            self.server_ip,
            self.subnet_mask,
            self.mode,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build_offer_with_ctx(
        &self,
        req: &DhcpPacket,
        is_ipxe: bool,
        arch: ClientArch,
        observed_ip: Option<Ipv4Addr>,
        server_ip: Ipv4Addr,
        subnet_mask: Ipv4Addr,
        mode: ServerMode,
    ) -> Result<Option<DhcpPacket>> {
        let (yiaddr, lease_time): (Ipv4Addr, u32) = match mode {
            ServerMode::Proxy => {
                // ProxyDHCP does NOT allocate IP (client gets IP from router)
                (Ipv4Addr::UNSPECIFIED, 0)
            }
            ServerMode::Standalone => {
                let allocated_ip = self.allocate_ip_for_subnet(&req.mac_string(), server_ip)?;
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
            siaddr: server_ip,
            giaddr: req.giaddr,
            chaddr: req.chaddr,
            sname: [0u8; 64],
            file: [0u8; 128],
            options: HashMap::new(),
        };

        // Determine Bootfile and Next Server
        let bootfile = if is_ipxe {
            format!("http://{}:{}/boot.ipxe", server_ip, self.http_port)
        } else {
            arch.default_bootfile().to_string()
        };

        self.populate_boot_fields(&mut reply, &bootfile, server_ip, is_ipxe);

        // Option 53: DHCP Offer
        reply.options.insert(53, vec![DhcpMessageType::Offer as u8]);

        // Option 54: Server Identifier
        reply.options.insert(54, server_ip.octets().to_vec());

        if mode == ServerMode::Proxy {
            self.populate_pxe_vendor_options(req, &mut reply, arch, server_ip);
        } else {
            // Standalone mode options (strictly isolated from ProxyDHCP)
            reply.options.insert(51, lease_time.to_be_bytes().to_vec());
            reply.options.insert(1, subnet_mask.octets().to_vec());

            let router_ip = if server_ip == self.server_ip {
                self.router.unwrap_or(server_ip)
            } else {
                server_ip
            };
            reply.options.insert(3, router_ip.octets().to_vec());

            let mut dns_bytes = Vec::new();
            if let Some(d) = self.dns {
                dns_bytes.extend_from_slice(&d.octets());
            }
            dns_bytes.extend_from_slice(&Ipv4Addr::new(8, 4, 4, 1).octets());
            reply.options.insert(6, dns_bytes);

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

    #[cfg(test)]
    pub(crate) fn build_ack(
        &self,
        req: &DhcpPacket,
        is_ipxe: bool,
        arch: ClientArch,
        observed_ip: Option<Ipv4Addr>,
    ) -> Result<Option<DhcpPacket>> {
        self.build_ack_with_ctx(
            req,
            is_ipxe,
            arch,
            observed_ip,
            self.server_ip,
            self.subnet_mask,
            self.mode,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build_ack_with_ctx(
        &self,
        req: &DhcpPacket,
        is_ipxe: bool,
        arch: ClientArch,
        observed_ip: Option<Ipv4Addr>,
        server_ip: Ipv4Addr,
        subnet_mask: Ipv4Addr,
        mode: ServerMode,
    ) -> Result<Option<DhcpPacket>> {
        let (yiaddr, lease_time): (Ipv4Addr, u32) = match mode {
            ServerMode::Proxy => {
                // In ProxyDHCP mode, yiaddr MUST be 0.0.0.0 so clients do not overwrite their router lease
                (Ipv4Addr::UNSPECIFIED, 0)
            }
            ServerMode::Standalone => {
                let ip = self.allocate_ip_for_subnet(&req.mac_string(), server_ip)?;
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
            siaddr: server_ip,
            giaddr: req.giaddr,
            chaddr: req.chaddr,
            sname: [0u8; 64],
            file: [0u8; 128],
            options: HashMap::new(),
        };

        let bootfile = if is_ipxe {
            format!("http://{}:{}/boot.ipxe", server_ip, self.http_port)
        } else {
            arch.default_bootfile().to_string()
        };

        self.populate_boot_fields(&mut reply, &bootfile, server_ip, is_ipxe);

        // Option 53: DHCP ACK
        reply.options.insert(53, vec![DhcpMessageType::Ack as u8]);

        // Option 54: Server Identifier
        reply.options.insert(54, server_ip.octets().to_vec());

        if mode == ServerMode::Proxy {
            self.populate_pxe_vendor_options(req, &mut reply, arch, server_ip);
        } else {
            reply.options.insert(51, lease_time.to_be_bytes().to_vec());
            reply.options.insert(1, subnet_mask.octets().to_vec());
            let router_ip = if server_ip == self.server_ip {
                self.router.unwrap_or(server_ip)
            } else {
                server_ip
            };
            reply.options.insert(3, router_ip.octets().to_vec());

            let mut dns_bytes = Vec::new();
            if let Some(d) = self.dns {
                dns_bytes.extend_from_slice(&d.octets());
            }
            dns_bytes.extend_from_slice(&Ipv4Addr::new(8, 4, 4, 1).octets());
            reply.options.insert(6, dns_bytes);

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
        recv_if: Option<&str>,
        active_server_ip: Ipv4Addr,
    ) -> Result<()> {
        let bytes = reply.serialize();

        // Bind outbound interface on macOS to the exact interface that received the request
        #[cfg(target_os = "macos")]
        {
            use std::os::fd::AsRawFd;
            let target_if_name = recv_if.map(|s| s.to_string()).or_else(|| {
                crate::config::list_network_interfaces()
                    .into_iter()
                    .find(|(_, ip)| *ip == active_server_ip)
                    .map(|(name, _)| name)
            });
            if let Some(if_name) = target_if_name
                && let Ok(c_name) = std::ffi::CString::new(if_name)
            {
                unsafe {
                    let idx = libc::if_nametoindex(c_name.as_ptr());
                    if idx != 0 {
                        const IP_BOUND_IF: libc::c_int = 25;
                        libc::setsockopt(
                            sock.as_raw_fd(),
                            libc::IPPROTO_IP,
                            IP_BOUND_IF,
                            &idx as *const _ as *const libc::c_void,
                            std::mem::size_of_val(&idx) as libc::socklen_t,
                        );
                        const IP_ONESBCAST: libc::c_int = 23;
                        let one: libc::c_int = 1;
                        libc::setsockopt(
                            sock.as_raw_fd(),
                            libc::IPPROTO_IP,
                            IP_ONESBCAST,
                            &one as *const _ as *const libc::c_void,
                            std::mem::size_of_val(&one) as libc::socklen_t,
                        );
                    }
                }
            }
        }

        // Destination routing:
        // 1. ProxyDHCP port (4011) or non-standard port simulation: reply directly via unicast to src
        // 2. Standard DHCP port 67: broadcast to 255.255.255.255:68 (per pxe-protocol-engineering Rule 4)
        let target_dest = if is_proxy_port || (self.dhcp_port != 67 && !src.ip().is_unspecified()) {
            src
        } else {
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, 68))
        };

        let subnet_bcast =
            Ipv4Addr::from(u32::from(active_server_ip) | !u32::from(self.subnet_mask));
        let subnet_dest = SocketAddr::V4(SocketAddrV4::new(subnet_bcast, 68));

        // On macOS, a socket bound to 0.0.0.0:67 stamps source IP 192.168.1.172 (en0) on
        // 255.255.255.255 broadcasts even when IP_BOUND_IF is set.
        // To guarantee the IPv4 header on the wire has Src IP = 10.77.0.3 (active_server_ip)
        // and exits en4, we transmit from a dedicated socket explicitly bound to active_server_ip:67.
        let send_res = if !active_server_ip.is_loopback()
            && !active_server_ip.is_unspecified()
            && !is_proxy_port
        {
            if let Ok(tx_sock) = Self::create_tx_socket(self.dhcp_port, active_server_ip) {
                // 1) Send with IP_ONESBCAST=1 to subnet_dest (wire Dst IP = 255.255.255.255, Dst MAC = ff:ff:ff:ff:ff:ff, Src IP = 10.77.0.3 on en4)
                let r1 = tx_sock.send_to(&bytes, subnet_dest).await;
                // 2) Send with IP_ONESBCAST=0 to subnet_dest (wire Dst IP = 10.77.255.255, Dst MAC = ff:ff:ff:ff:ff:ff, Src IP = 10.77.0.3 on en4)
                #[cfg(target_os = "macos")]
                unsafe {
                    use std::os::fd::AsRawFd;
                    const IP_ONESBCAST: libc::c_int = 23;
                    let zero: libc::c_int = 0;
                    libc::setsockopt(
                        tx_sock.as_raw_fd(),
                        libc::IPPROTO_IP,
                        IP_ONESBCAST,
                        &zero as *const _ as *const libc::c_void,
                        std::mem::size_of_val(&zero) as libc::socklen_t,
                    );
                }
                let r2 = tx_sock.send_to(&bytes, subnet_dest).await;
                // 3) If yiaddr is assigned and already in the ARP cache (e.g. iPXE Stage 2 after UEFI Stage 1),
                // also transmit a unicast copy to yiaddr:68 so iPXE SNP filters that only accept unicast
                // frames for their MAC receive the offer immediately.
                if !reply.yiaddr.is_unspecified() {
                    let unicast_dest = SocketAddr::V4(SocketAddrV4::new(reply.yiaddr, 68));
                    let _ = tx_sock.send_to(&bytes, unicast_dest).await;
                }
                let _ = tx_sock.send_to(&bytes, target_dest).await;

                // 4) For Stage 2 iPXE (Option 77/175), when `dhcp` calls ifopen() for the first time,
                // the 2.5GbE PHY takes ~400-800ms to finish RX link training after TX is ready.
                // Spawn a background re-transmit bursts at +200ms, +500ms, and +900ms so the DHCPOFFER
                // arrives right after the client's 2.5GbE RX link locks!
                if reply.options.get(&67).is_some_and(|v| !v.ends_with(&[0])) {
                    let bytes_clone = bytes.clone();
                    let yiaddr_copy = reply.yiaddr;
                    let port_copy = self.dhcp_port;
                    tokio::spawn(async move {
                        for delay_ms in [200_u64, 500, 900] {
                            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                            if let Ok(burst_sock) = Self::create_tx_socket(port_copy, active_server_ip) {
                                let _ = burst_sock.send_to(&bytes_clone, subnet_dest).await;
                                #[cfg(target_os = "macos")]
                                unsafe {
                                    use std::os::fd::AsRawFd;
                                    const IP_ONESBCAST: libc::c_int = 23;
                                    let zero: libc::c_int = 0;
                                    libc::setsockopt(
                                        burst_sock.as_raw_fd(),
                                        libc::IPPROTO_IP,
                                        IP_ONESBCAST,
                                        &zero as *const _ as *const libc::c_void,
                                        std::mem::size_of_val(&zero) as libc::socklen_t,
                                    );
                                }
                                let _ = burst_sock.send_to(&bytes_clone, subnet_dest).await;
                                if !yiaddr_copy.is_unspecified() {
                                    let u_dest = SocketAddr::V4(SocketAddrV4::new(yiaddr_copy, 68));
                                    let _ = burst_sock.send_to(&bytes_clone, u_dest).await;
                                }
                            }
                        }
                    });
                }

                match r1.or(r2) {
                    Ok(n) => Ok((n, SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, 68)))),
                    Err(e) => Err(e),
                }
            } else {
                sock.send_to(&bytes, target_dest)
                    .await
                    .map(|n| (n, target_dest))
            }
        } else {
            match sock.send_to(&bytes, target_dest).await {
                Ok(n) => Ok((n, target_dest)),
                Err(e)
                    if target_dest.ip() == Ipv4Addr::BROADCAST
                        && subnet_bcast != Ipv4Addr::BROADCAST =>
                {
                    sock.send_to(&bytes, subnet_dest)
                        .await
                        .map(|n| (n, subnet_dest))
                        .map_err(|_| e)
                }
                Err(e) => Err(e),
            }
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
                let if_tag = recv_if.map(|s| format!(" ({s})")).unwrap_or_default();
                self.state.log(
                    EventLevel::Success,
                    Protocol::Dhcp,
                    format!("Sent {msg_name} to {mac} via {actual_dest}{if_tag}"),
                );
                Ok(())
            }
            Err(e) => {
                self.state.record_error();
                Err(DhcpError::Socket(e).into())
            }
        }
    }

    fn allocate_ip_for_subnet(&self, mac: &str, server_ip: Ipv4Addr) -> Result<Ipv4Addr> {
        let (start, end) = match self.pool_range {
            Some((s, e)) if server_ip == self.server_ip => (s, e),
            _ => {
                let oct = server_ip.octets();
                // On direct Ethernet link (e.g. 10.77.0.3), start pool at .2 (10.77.0.2)
                (
                    Ipv4Addr::new(oct[0], oct[1], oct[2], 2),
                    Ipv4Addr::new(oct[0], oct[1], oct[2], 50),
                )
            }
        };

        let mut lock = self
            .leases
            .lock()
            .map_err(|_| crate::error::PxeError::Config("Lease lock poisoned".to_string()))?;

        // Check if MAC already has a valid lease
        if let Some((ip, _)) = lock.get(mac) {
            return Ok(*ip);
        }

        // Find first free IP in range (skipping server_ip itself)
        let start_u32 = u32::from(start);
        let end_u32 = u32::from(end);

        let active_ips: Vec<Ipv4Addr> = lock.values().map(|(ip, _)| *ip).collect();

        for candidate_u32 in start_u32..=end_u32 {
            let candidate = Ipv4Addr::from(candidate_u32);
            if candidate != server_ip && !active_ips.contains(&candidate) {
                lock.insert(mac.to_string(), (candidate, Instant::now()));
                return Ok(candidate);
            }
        }

        Err(DhcpError::PoolExhausted.into())
    }
}
