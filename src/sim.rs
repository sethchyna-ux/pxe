use crate::config::ServerMode;
use crate::dhcp::packet::{
    ClientArch, DhcpMessageType, DhcpPacket, BOOTREPLY, BOOTREQUEST, HLEN_ETHER, HTYPE_ETHER,
};
use crate::dhcp::{DhcpConfig, DhcpServer};
use crate::http::HttpServer;
use crate::state::SharedState;
use crate::tftp::packet::{TftpPacket, OP_ERROR};
use crate::tftp::TftpServer;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

/// Hardware profile of the ASUS Z97-A Motherboard (Intel I218-V Gigabit LAN)
pub const Z97A_MAC: [u8; 6] = [0xf0, 0x79, 0x59, 0x70, 0xda, 0x7f];
pub const Z97A_MAC_STR: &str = "f0:79:59:70:da:7f";
pub const Z97A_UUID: [u8; 17] = [
    0x00, 0x90, 0x2b, 0x34, 0x59, 0x79, 0xf0, 0x11, 0xe4, 0x8a, 0x00, 0xf0, 0x79, 0x59, 0x70, 0xda,
    0x7f,
];

#[derive(Debug, Clone)]
pub struct SimulationStepResult {
    pub step: String,
    pub passed: bool,
    pub detail: String,
    pub duration_ms: u128,
}

#[derive(Debug, Clone)]
pub struct SimulationReport {
    pub scenario: String,
    pub steps: Vec<SimulationStepResult>,
    pub passed: bool,
}

fn find_free_udp_port() -> std::io::Result<u16> {
    let sock = std::net::UdpSocket::bind("127.0.0.1:0")?;
    Ok(sock.local_addr()?.port())
}

fn find_free_tcp_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn build_z97a_chaddr() -> [u8; 16] {
    let mut chaddr = [0u8; 16];
    chaddr[..6].copy_from_slice(&Z97A_MAC);
    chaddr
}

async fn tftp_get_file_size_probe(
    server_addr: SocketAddr,
    filename: &str,
    blksize: usize,
) -> Result<usize, String> {
    let sock = UdpSocket::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("bind failed: {e}"))?;

    let mut options = HashMap::new();
    options.insert("tsize".to_string(), "0".to_string());
    options.insert("blksize".to_string(), blksize.to_string());

    let rrq = TftpPacket::Rrq {
        filename: filename.to_string(),
        mode: "octet".to_string(),
        options,
    };

    sock.send_to(&rrq.serialize(), server_addr)
        .await
        .map_err(|e| format!("send RRQ failed: {e}"))?;

    let mut buf = [0u8; 1024];
    let (len, tid_addr) = tokio::time::timeout(Duration::from_secs(2), sock.recv_from(&mut buf))
        .await
        .map_err(|_| "timeout waiting for OACK".to_string())?
        .map_err(|e| format!("recv OACK failed: {e}"))?;

    let pkt = TftpPacket::parse(&buf[..len]).map_err(|e| format!("parse OACK failed: {e}"))?;
    let tsize = match pkt {
        TftpPacket::Oack { options } => options
            .get("tsize")
            .ok_or_else(|| "OACK missing tsize".to_string())?
            .parse::<usize>()
            .map_err(|e| format!("invalid tsize: {e}"))?,
        other => return Err(format!("expected OACK, got {other:?}")),
    };

    // Emulate AMI Aptio IV UEFI MTFTP GetFileSize abort: send ERROR after reading tsize
    let mut err_cancel = Vec::new();
    err_cancel.extend_from_slice(&OP_ERROR.to_be_bytes());
    err_cancel.extend_from_slice(&0u16.to_be_bytes());
    err_cancel.extend_from_slice(b"User aborted\0");
    sock.send_to(&err_cancel, tid_addr)
        .await
        .map_err(|e| format!("send abort ERROR failed: {e}"))?;

    // Yield briefly so server task processes the ERROR packet cleanly
    tokio::time::sleep(Duration::from_millis(15)).await;
    Ok(tsize)
}

async fn tftp_download_file(
    server_addr: SocketAddr,
    filename: &str,
    blksize: usize,
) -> Result<Vec<u8>, String> {
    let sock = UdpSocket::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("bind failed: {e}"))?;

    let mut options = HashMap::new();
    options.insert("blksize".to_string(), blksize.to_string());

    let rrq = TftpPacket::Rrq {
        filename: filename.to_string(),
        mode: "octet".to_string(),
        options,
    };

    sock.send_to(&rrq.serialize(), server_addr)
        .await
        .map_err(|e| format!("send RRQ failed: {e}"))?;

    let mut buf = [0u8; 2048];
    let (len, tid_addr) = tokio::time::timeout(Duration::from_secs(2), sock.recv_from(&mut buf))
        .await
        .map_err(|_| "timeout waiting for OACK".to_string())?
        .map_err(|e| format!("recv OACK failed: {e}"))?;

    match TftpPacket::parse(&buf[..len]).map_err(|e| format!("parse OACK failed: {e}"))? {
        TftpPacket::Oack { .. } => {
            let ack0 = TftpPacket::Ack { block: 0 };
            sock.send_to(&ack0.serialize(), tid_addr)
                .await
                .map_err(|e| format!("send ACK 0 failed: {e}"))?;
        }
        other => return Err(format!("expected OACK, got {other:?}")),
    }

    let mut downloaded = Vec::new();
    let mut expected_block: u16 = 1;

    loop {
        let (len, srv_tid) =
            tokio::time::timeout(Duration::from_secs(2), sock.recv_from(&mut buf))
                .await
                .map_err(|_| format!("timeout waiting for DATA block {expected_block}"))?
                .map_err(|e| format!("recv DATA failed: {e}"))?;

        match TftpPacket::parse(&buf[..len]).map_err(|e| format!("parse DATA failed: {e}"))? {
            TftpPacket::Data { block, data } if block == expected_block => {
                let chunk_len = data.len();
                downloaded.extend_from_slice(&data);
                let ack = TftpPacket::Ack { block };
                sock.send_to(&ack.serialize(), srv_tid)
                    .await
                    .map_err(|e| format!("send ACK {block} failed: {e}"))?;
                if chunk_len < blksize {
                    break;
                }
                expected_block = expected_block.wrapping_add(1);
            }
            other => return Err(format!("unexpected TFTP packet: {other:?}")),
        }
    }

    Ok(downloaded)
}

async fn http_get_text(addr: SocketAddr, path: &str) -> Result<(u16, String), String> {
    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(|e| format!("TCP connect failed: {e}"))?;
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {}\r\nUser-Agent: iPXE/1.21.1+\r\nConnection: close\r\n\r\n",
        addr
    );
    stream
        .write_all(req.as_bytes())
        .await
        .map_err(|e| format!("HTTP write failed: {e}"))?;

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .await
        .map_err(|e| format!("HTTP read failed: {e}"))?;

    let text = String::from_utf8_lossy(&raw).to_string();
    let (headers, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| "malformed HTTP response".to_string())?;
    let status_line = headers.lines().next().unwrap_or("");
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    Ok((status_code, body.to_string()))
}

/// Run the full ASUS Z97-A simulation suite (UEFI ProxyDHCP, Legacy BIOS ProxyDHCP, and Standalone DHCP)
pub async fn run_z97a_simulation_suite(
    tftp_dir: &Path,
    http_dir: &Path,
) -> Result<Vec<SimulationReport>, String> {
    let mut reports = Vec::new();
    reports.push(simulate_z97a_uefi_proxydhcp(tftp_dir, http_dir).await?);
    reports.push(simulate_z97a_bios_proxydhcp(tftp_dir, http_dir).await?);
    reports.push(simulate_z97a_standalone_dhcp(tftp_dir, http_dir).await?);
    Ok(reports)
}

async fn simulate_z97a_uefi_proxydhcp(
    tftp_dir: &Path,
    http_dir: &Path,
) -> Result<SimulationReport, String> {
    let server_ip = Ipv4Addr::new(127, 0, 0, 1);
    let dhcp_port = find_free_udp_port().map_err(|e| e.to_string())?;
    let proxy_port = find_free_udp_port().map_err(|e| e.to_string())?;
    let tftp_port = find_free_udp_port().map_err(|e| e.to_string())?;
    let http_port = find_free_tcp_port().map_err(|e| e.to_string())?;

    let state = Arc::new(SharedState::new(500));
    let dhcp_srv = Arc::new(DhcpServer::new(
        Arc::clone(&state),
        DhcpConfig {
            server_ip,
            mode: ServerMode::Proxy,
            dhcp_port,
            proxy_port,
            http_port,
            subnet_mask: Ipv4Addr::new(255, 255, 255, 0),
            router: None,
            dns: None,
            pool_range: None,
        },
    ));
    let tftp_srv = Arc::new(
        TftpServer::new(Arc::clone(&state), tftp_dir.to_path_buf(), tftp_port)
            .with_http_context(server_ip, http_port),
    );
    let http_srv = Arc::new(HttpServer::new(
        Arc::clone(&state),
        server_ip,
        http_port,
        http_dir.to_path_buf(),
    ));

    let d_handle = tokio::spawn(async move {
        let _ = dhcp_srv.run().await;
    });
    let t_handle = tokio::spawn(async move {
        let _ = tftp_srv.run().await;
    });
    let h_handle = tokio::spawn(async move {
        let _ = http_srv.run().await;
    });

    tokio::time::sleep(Duration::from_millis(40)).await;

    let mut steps = Vec::new();
    let cli_sock = UdpSocket::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;

    // Step 1: ASUS Z97-A AMI Aptio IV UEFI PXE DHCPDISCOVER (Arch 0x0007, UNDI 3.16)
    let t0 = Instant::now();
    let mut disc_opts = HashMap::new();
    disc_opts.insert(53, vec![DhcpMessageType::Discover as u8]);
    disc_opts.insert(55, vec![1, 3, 6, 43, 60, 66, 67, 97]);
    disc_opts.insert(60, b"PXEClient:Arch:00007:UNDI:003016".to_vec());
    disc_opts.insert(93, vec![0x00, 0x07]); // EFI x86_64
    disc_opts.insert(94, vec![1, 3, 16]); // UNDI 3.16
    disc_opts.insert(97, Z97A_UUID.to_vec());

    let discover = DhcpPacket {
        op: BOOTREQUEST,
        htype: HTYPE_ETHER,
        hlen: HLEN_ETHER,
        hops: 0,
        xid: 0xa505_297a,
        secs: 0,
        flags: 0x0000,
        ciaddr: Ipv4Addr::UNSPECIFIED,
        yiaddr: Ipv4Addr::UNSPECIFIED,
        siaddr: Ipv4Addr::UNSPECIFIED,
        giaddr: Ipv4Addr::UNSPECIFIED,
        chaddr: build_z97a_chaddr(),
        sname: [0; 64],
        file: [0; 128],
        options: disc_opts,
    };

    cli_sock
        .send_to(
            &discover.serialize(),
            SocketAddr::V4(SocketAddrV4::new(server_ip, dhcp_port)),
        )
        .await
        .map_err(|e| e.to_string())?;

    let mut buf = [0u8; 2048];
    let (len, _) = tokio::time::timeout(Duration::from_secs(2), cli_sock.recv_from(&mut buf))
        .await
        .map_err(|_| "Timeout waiting for UEFI ProxyDHCP DHCPOFFER".to_string())?
        .map_err(|e| e.to_string())?;
    let offer = DhcpPacket::parse(&buf[..len]).map_err(|e| e.to_string())?;

    let offer_ok = offer.op == BOOTREPLY
        && offer.message_type() == Some(DhcpMessageType::Offer)
        && offer.yiaddr == Ipv4Addr::UNSPECIFIED
        && offer.siaddr == server_ip
        && offer.options.get(&60).map(|v| v.as_slice()) == Some(b"PXEClient")
        && offer.options.get(&43).map(|v| v.as_slice()) == Some(&[6, 1, 8, 255])
        && offer.options.get(&67).map(|v| v.as_slice()) == Some(b"ipxe.efi")
        && offer.options.get(&97).map(|v| v.as_slice()) == Some(&Z97A_UUID);

    steps.push(SimulationStepResult {
        step: "Stage 1a: UEFI DHCPDISCOVER -> ProxyDHCP DHCPOFFER".to_string(),
        passed: offer_ok,
        detail: format!(
            "siaddr={}, opt60=PXEClient, opt43={:?}, opt67=ipxe.efi",
            offer.siaddr,
            offer.options.get(&43)
        ),
        duration_ms: t0.elapsed().as_millis(),
    });

    // Step 2: ASUS Z97-A Unicast ProxyDHCP DHCPREQUEST to UDP port 4011 (with Option 60 only, no Option 93)
    let t1 = Instant::now();
    let mut req_opts = HashMap::new();
    req_opts.insert(53, vec![DhcpMessageType::Request as u8]);
    req_opts.insert(54, server_ip.octets().to_vec());
    req_opts.insert(60, b"PXEClient:Arch:00007:UNDI:003016".to_vec());
    req_opts.insert(97, Z97A_UUID.to_vec());

    let proxy_req = DhcpPacket {
        op: BOOTREQUEST,
        htype: HTYPE_ETHER,
        hlen: HLEN_ETHER,
        hops: 0,
        xid: 0xa505_297b,
        secs: 0,
        flags: 0x0000, // Unicast to port 4011
        ciaddr: Ipv4Addr::new(192, 168, 1, 12),
        yiaddr: Ipv4Addr::UNSPECIFIED,
        siaddr: server_ip,
        giaddr: Ipv4Addr::UNSPECIFIED,
        chaddr: build_z97a_chaddr(),
        sname: [0; 64],
        file: [0; 128],
        options: req_opts,
    };

    cli_sock
        .send_to(
            &proxy_req.serialize(),
            SocketAddr::V4(SocketAddrV4::new(server_ip, proxy_port)),
        )
        .await
        .map_err(|e| e.to_string())?;

    let (len, _) = tokio::time::timeout(Duration::from_secs(2), cli_sock.recv_from(&mut buf))
        .await
        .map_err(|_| "Timeout waiting for ProxyDHCP :4011 DHCPACK".to_string())?
        .map_err(|e| e.to_string())?;
    let ack = DhcpPacket::parse(&buf[..len]).map_err(|e| e.to_string())?;
    let ack_ok = ack.message_type() == Some(DhcpMessageType::Ack)
        && ack.options.get(&67).map(|v| v.as_slice()) == Some(b"ipxe.efi")
        && ack.options.get(&43).map(|v| v.as_slice()) == Some(&[6, 1, 8, 255]);

    steps.push(SimulationStepResult {
        step: "Stage 1b: Unicast ProxyDHCP (:4011) DHCPREQUEST -> DHCPACK".to_string(),
        passed: ack_ok,
        detail: "Port 4011 unicast reply verified; Option 60 Arch:00007 fallback -> ipxe.efi"
            .to_string(),
        duration_ms: t1.elapsed().as_millis(),
    });

    // Step 3: ASUS Z97-A UEFI MTFTP GetFileSize Probe (RRQ tsize=0 + client ERROR abort)
    let t2 = Instant::now();
    let tftp_addr = SocketAddr::V4(SocketAddrV4::new(server_ip, tftp_port));
    let probed_size = tftp_get_file_size_probe(tftp_addr, "ipxe.efi", 1468).await?;
    let errors_after_probe = state.metrics_snapshot().errors;
    let probe_ok = probed_size > 100_000 && errors_after_probe == 0;

    steps.push(SimulationStepResult {
        step: "Stage 1c: UEFI MTFTP GetFileSize Probe (OACK tsize + ERROR abort)".to_string(),
        passed: probe_ok,
        detail: format!(
            "Probed ipxe.efi tsize={probed_size} bytes, server error count={errors_after_probe}"
        ),
        duration_ms: t2.elapsed().as_millis(),
    });

    // Step 4: ASUS Z97-A UEFI TFTP Download of ipxe.efi (case-insensitive "IPXE.EFI" test)
    let t3 = Instant::now();
    let efi_bytes = tftp_download_file(tftp_addr, "IPXE.EFI", 1468).await?;
    let has_mz_header = efi_bytes.len() == probed_size && efi_bytes.starts_with(b"MZ");

    steps.push(SimulationStepResult {
        step: "Stage 1d: TFTP Download NBP (IPXE.EFI, blksize=1468)".to_string(),
        passed: has_mz_header,
        detail: format!(
            "Downloaded {} bytes (PE32+ 'MZ' signature verified)",
            efi_bytes.len()
        ),
        duration_ms: t3.elapsed().as_millis(),
    });

    // Step 5: iPXE Stage 2 DHCP (Option 77 = "iPXE" + Option 175) -> HTTP boot.ipxe URL
    let t4 = Instant::now();
    let mut ipxe_opts = HashMap::new();
    ipxe_opts.insert(53, vec![DhcpMessageType::Request as u8]);
    ipxe_opts.insert(60, b"PXEClient:Arch:00007:UNDI:003016".to_vec());
    ipxe_opts.insert(77, b"iPXE".to_vec());
    ipxe_opts.insert(175, vec![177, 1, 1]);
    ipxe_opts.insert(93, vec![0x00, 0x07]);

    let ipxe_req = DhcpPacket {
        op: BOOTREQUEST,
        htype: HTYPE_ETHER,
        hlen: HLEN_ETHER,
        hops: 0,
        xid: 0xa505_297c,
        secs: 0,
        flags: 0x0000,
        ciaddr: Ipv4Addr::new(192, 168, 1, 12),
        yiaddr: Ipv4Addr::UNSPECIFIED,
        siaddr: server_ip,
        giaddr: Ipv4Addr::UNSPECIFIED,
        chaddr: build_z97a_chaddr(),
        sname: [0; 64],
        file: [0; 128],
        options: ipxe_opts,
    };

    cli_sock
        .send_to(
            &ipxe_req.serialize(),
            SocketAddr::V4(SocketAddrV4::new(server_ip, dhcp_port)),
        )
        .await
        .map_err(|e| e.to_string())?;

    let (len, _) = tokio::time::timeout(Duration::from_secs(2), cli_sock.recv_from(&mut buf))
        .await
        .map_err(|_| "Timeout waiting for iPXE Stage 2 DHCPACK".to_string())?
        .map_err(|e| e.to_string())?;
    let ipxe_ack = DhcpPacket::parse(&buf[..len]).map_err(|e| e.to_string())?;
    let expected_url = format!("http://{server_ip}:{http_port}/boot.ipxe");
    let got_url = ipxe_ack
        .options
        .get(&67)
        .map(|v| String::from_utf8_lossy(v).to_string())
        .unwrap_or_default();
    let stage2_dhcp_ok = got_url == expected_url && !got_url.contains('\0');

    steps.push(SimulationStepResult {
        step: "Stage 2a: iPXE Stage 2 DHCP (Option 77/175) -> HTTP Boot URL".to_string(),
        passed: stage2_dhcp_ok,
        detail: format!("Option 67 = '{got_url}' (no trailing null byte)"),
        duration_ms: t4.elapsed().as_millis(),
    });

    // Step 6: TFTP autoexec.ipxe & HTTP /boot.ipxe dynamic interpolation + unified client state
    let t5 = Instant::now();
    let autoexec_bytes = tftp_download_file(tftp_addr, "autoexec.ipxe", 512).await?;
    let autoexec_str = String::from_utf8_lossy(&autoexec_bytes);
    let http_addr = SocketAddr::V4(SocketAddrV4::new(server_ip, http_port));
    let (status_code, boot_ipxe_body) = http_get_text(http_addr, "/boot.ipxe").await?;

    let clients = state.client_snapshots();
    let metrics = state.metrics_snapshot();
    let http_ok = status_code == 200
        && autoexec_str.contains(&expected_url)
        && boot_ipxe_body.starts_with("#!ipxe")
        && boot_ipxe_body.contains(&format!("http://{server_ip}:{http_port}"))
        && clients.len() == 1
        && clients[0].mac == Z97A_MAC_STR
        && metrics.errors == 0;

    steps.push(SimulationStepResult {
        step: "Stage 2b: TFTP autoexec.ipxe + HTTP /boot.ipxe Menu & Session Tracking".to_string(),
        passed: http_ok,
        detail: format!(
            "HTTP {status_code} ({} bytes), active_clients={} ({}), errors={}",
            boot_ipxe_body.len(),
            clients.len(),
            clients.first().map(|c| c.mac.as_str()).unwrap_or("none"),
            metrics.errors
        ),
        duration_ms: t5.elapsed().as_millis(),
    });

    d_handle.abort();
    t_handle.abort();
    h_handle.abort();

    let passed = steps.iter().all(|s| s.passed);
    Ok(SimulationReport {
        scenario: "ASUS Z97-A UEFI PXE Boot (Intel I218-V UNDI + AMI Aptio IV, ProxyDHCP)".to_string(),
        steps,
        passed,
    })
}

async fn simulate_z97a_bios_proxydhcp(
    tftp_dir: &Path,
    http_dir: &Path,
) -> Result<SimulationReport, String> {
    let server_ip = Ipv4Addr::new(127, 0, 0, 1);
    let dhcp_port = find_free_udp_port().map_err(|e| e.to_string())?;
    let proxy_port = find_free_udp_port().map_err(|e| e.to_string())?;
    let tftp_port = find_free_udp_port().map_err(|e| e.to_string())?;
    let http_port = find_free_tcp_port().map_err(|e| e.to_string())?;

    let state = Arc::new(SharedState::new(500));
    let dhcp_srv = Arc::new(DhcpServer::new(
        Arc::clone(&state),
        DhcpConfig {
            server_ip,
            mode: ServerMode::Proxy,
            dhcp_port,
            proxy_port,
            http_port,
            subnet_mask: Ipv4Addr::new(255, 255, 255, 0),
            router: None,
            dns: None,
            pool_range: None,
        },
    ));
    let tftp_srv = Arc::new(
        TftpServer::new(Arc::clone(&state), tftp_dir.to_path_buf(), tftp_port)
            .with_http_context(server_ip, http_port),
    );
    let http_srv = Arc::new(HttpServer::new(
        Arc::clone(&state),
        server_ip,
        http_port,
        http_dir.to_path_buf(),
    ));

    let d_handle = tokio::spawn(async move {
        let _ = dhcp_srv.run().await;
    });
    let t_handle = tokio::spawn(async move {
        let _ = tftp_srv.run().await;
    });
    let h_handle = tokio::spawn(async move {
        let _ = http_srv.run().await;
    });

    tokio::time::sleep(Duration::from_millis(40)).await;

    let mut steps = Vec::new();
    let cli_sock = UdpSocket::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;

    // Step 1: Intel Boot Agent GE v1.5.50 Legacy BIOS DHCPDISCOVER (Arch 0x0000, UNDI 2.1)
    let t0 = Instant::now();
    let mut disc_opts = HashMap::new();
    disc_opts.insert(53, vec![DhcpMessageType::Discover as u8]);
    disc_opts.insert(60, b"PXEClient:Arch:00000:UNDI:002001".to_vec());
    disc_opts.insert(93, vec![0x00, 0x00]); // Legacy BIOS x86
    disc_opts.insert(97, Z97A_UUID.to_vec());

    let discover = DhcpPacket {
        op: BOOTREQUEST,
        htype: HTYPE_ETHER,
        hlen: HLEN_ETHER,
        hops: 0,
        xid: 0xb105_0001,
        secs: 0,
        flags: 0x0000,
        ciaddr: Ipv4Addr::UNSPECIFIED,
        yiaddr: Ipv4Addr::UNSPECIFIED,
        siaddr: Ipv4Addr::UNSPECIFIED,
        giaddr: Ipv4Addr::UNSPECIFIED,
        chaddr: build_z97a_chaddr(),
        sname: [0; 64],
        file: [0; 128],
        options: disc_opts,
    };

    cli_sock
        .send_to(
            &discover.serialize(),
            SocketAddr::V4(SocketAddrV4::new(server_ip, dhcp_port)),
        )
        .await
        .map_err(|e| e.to_string())?;

    let mut buf = [0u8; 2048];
    let (len, _) = tokio::time::timeout(Duration::from_secs(2), cli_sock.recv_from(&mut buf))
        .await
        .map_err(|_| "Timeout waiting for BIOS DHCPOFFER".to_string())?
        .map_err(|e| e.to_string())?;
    let offer = DhcpPacket::parse(&buf[..len]).map_err(|e| e.to_string())?;

    let offer_ok = offer.message_type() == Some(DhcpMessageType::Offer)
        && offer.options.get(&67).map(|v| v.as_slice()) == Some(b"undionly.kpxe")
        && offer.options.get(&43).map(|v| v.as_slice()) == Some(&[6, 1, 8, 255]);

    steps.push(SimulationStepResult {
        step: "Stage 1a: Intel Boot Agent GE v1.5.50 (CSM/BIOS) DHCPDISCOVER".to_string(),
        passed: offer_ok,
        detail: "Option 43 PXE_DISCOVERY_CONTROL=[6,1,8,255] + Option 67=undionly.kpxe".to_string(),
        duration_ms: t0.elapsed().as_millis(),
    });

    // Step 2: TFTP download undionly.kpxe
    let t1 = Instant::now();
    let tftp_addr = SocketAddr::V4(SocketAddrV4::new(server_ip, tftp_port));
    let kpxe_bytes = tftp_download_file(tftp_addr, "undionly.kpxe", 1456).await?;
    let kpxe_ok = kpxe_bytes.len() > 50_000;

    steps.push(SimulationStepResult {
        step: "Stage 1b: TFTP Download undionly.kpxe (blksize=1456)".to_string(),
        passed: kpxe_ok,
        detail: format!("Downloaded {} bytes cleanly", kpxe_bytes.len()),
        duration_ms: t1.elapsed().as_millis(),
    });

    d_handle.abort();
    t_handle.abort();
    h_handle.abort();

    let passed = steps.iter().all(|s| s.passed);
    Ok(SimulationReport {
        scenario: "ASUS Z97-A Legacy BIOS / CSM PXE Boot (Intel Boot Agent GE v1.5.50)".to_string(),
        steps,
        passed,
    })
}

async fn simulate_z97a_standalone_dhcp(
    tftp_dir: &Path,
    http_dir: &Path,
) -> Result<SimulationReport, String> {
    let server_ip = Ipv4Addr::new(127, 0, 0, 1);
    let dhcp_port = find_free_udp_port().map_err(|e| e.to_string())?;
    let proxy_port = find_free_udp_port().map_err(|e| e.to_string())?;
    let tftp_port = find_free_udp_port().map_err(|e| e.to_string())?;
    let http_port = find_free_tcp_port().map_err(|e| e.to_string())?;

    let pool_start = Ipv4Addr::new(192, 168, 1, 200);
    let pool_end = Ipv4Addr::new(192, 168, 1, 250);
    let router_ip = Ipv4Addr::new(192, 168, 1, 1);

    let state = Arc::new(SharedState::new(500));
    let dhcp_srv = Arc::new(DhcpServer::new(
        Arc::clone(&state),
        DhcpConfig {
            server_ip,
            mode: ServerMode::Standalone,
            dhcp_port,
            proxy_port,
            http_port,
            subnet_mask: Ipv4Addr::new(255, 255, 255, 0),
            router: Some(router_ip),
            dns: Some(Ipv4Addr::new(1, 1, 1, 1)),
            pool_range: Some((pool_start, pool_end)),
        },
    ));
    let tftp_srv = Arc::new(
        TftpServer::new(Arc::clone(&state), tftp_dir.to_path_buf(), tftp_port)
            .with_http_context(server_ip, http_port),
    );
    let http_srv = Arc::new(HttpServer::new(
        Arc::clone(&state),
        server_ip,
        http_port,
        http_dir.to_path_buf(),
    ));

    let d_handle = tokio::spawn(async move {
        let _ = dhcp_srv.run().await;
    });
    let t_handle = tokio::spawn(async move {
        let _ = tftp_srv.run().await;
    });
    let h_handle = tokio::spawn(async move {
        let _ = http_srv.run().await;
    });

    tokio::time::sleep(Duration::from_millis(40)).await;

    let mut steps = Vec::new();
    let cli_sock = UdpSocket::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;

    let t0 = Instant::now();
    let mut disc_opts = HashMap::new();
    disc_opts.insert(53, vec![DhcpMessageType::Discover as u8]);
    disc_opts.insert(93, vec![0x00, 0x07]);
    disc_opts.insert(97, Z97A_UUID.to_vec());

    let discover = DhcpPacket {
        op: BOOTREQUEST,
        htype: HTYPE_ETHER,
        hlen: HLEN_ETHER,
        hops: 0,
        xid: 0x57a0_0001,
        secs: 0,
        flags: 0x0000,
        ciaddr: Ipv4Addr::UNSPECIFIED,
        yiaddr: Ipv4Addr::UNSPECIFIED,
        siaddr: Ipv4Addr::UNSPECIFIED,
        giaddr: Ipv4Addr::UNSPECIFIED,
        chaddr: build_z97a_chaddr(),
        sname: [0; 64],
        file: [0; 128],
        options: disc_opts,
    };

    cli_sock
        .send_to(
            &discover.serialize(),
            SocketAddr::V4(SocketAddrV4::new(server_ip, dhcp_port)),
        )
        .await
        .map_err(|e| e.to_string())?;

    let mut buf = [0u8; 2048];
    let (len, _) = tokio::time::timeout(Duration::from_secs(2), cli_sock.recv_from(&mut buf))
        .await
        .map_err(|_| "Timeout waiting for Standalone DHCPOFFER".to_string())?
        .map_err(|e| e.to_string())?;
    let offer = DhcpPacket::parse(&buf[..len]).map_err(|e| e.to_string())?;
    let lease_ok = offer.yiaddr == pool_start
        && offer.options.get(&1).map(|v| v.as_slice()) == Some(&[255, 255, 255, 0])
        && offer.options.get(&3).map(|v| v.as_slice()) == Some(&[192, 168, 1, 1])
        && offer.options.get(&67).map(|v| v.as_slice()) == Some(b"ipxe.efi")
        && offer.client_arch() == ClientArch::BiosX86; // Reply has no opt 93

    steps.push(SimulationStepResult {
        step: "Standalone Authoritative DHCP Lease & Options Allocation".to_string(),
        passed: lease_ok,
        detail: format!(
            "Assigned yiaddr={}, mask=255.255.255.0, router={}, bootfile=ipxe.efi",
            offer.yiaddr, router_ip
        ),
        duration_ms: t0.elapsed().as_millis(),
    });

    d_handle.abort();
    t_handle.abort();
    h_handle.abort();

    let passed = steps.iter().all(|s| s.passed);
    Ok(SimulationReport {
        scenario: "ASUS Z97-A Standalone Authoritative DHCP Boot (Isolated Switch Mode)".to_string(),
        steps,
        passed,
    })
}
