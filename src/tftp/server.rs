use crate::error::{Result, TftpError};
use crate::state::{BootStage, EventLevel, Protocol, SharedState};
use crate::tftp::packet::{
    TftpPacket, ERR_ACCESS_VIOLATION, ERR_ILLEGAL_OP, ERR_NOT_FOUND,
};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;

pub struct TftpServer {
    state: Arc<SharedState>,
    tftp_dir: PathBuf,
    port: u16,
}

impl TftpServer {
    pub fn new(state: Arc<SharedState>, tftp_dir: PathBuf, port: u16) -> Self {
        Self {
            state,
            tftp_dir,
            port,
        }
    }

    pub async fn run(self: Arc<Self>) -> Result<()> {
        let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, self.port);
        let socket = match UdpSocket::bind(bind_addr).await {
            Ok(s) => s,
            Err(e) => {
                let msg = format!(
                    "Failed to bind TFTP port {}: {e} (run with sudo for port 69 or use --tftp-port)",
                    self.port
                );
                self.state.log(EventLevel::Error, Protocol::Tftp, &msg);
                return Err(TftpError::Io(e).into());
            }
        };

        self.state.log(
            EventLevel::Success,
            Protocol::Tftp,
            format!(
                "TFTP engine listening on UDP {} (root: {})",
                self.port,
                self.tftp_dir.display()
            ),
        );

        let mut buf = [0u8; 2048];
        loop {
            match socket.recv_from(&mut buf).await {
                Ok((len, client_addr)) => {
                    let packet_data = buf[..len].to_vec();
                    let this = Arc::clone(&self);
                    tokio::spawn(async move {
                        if let Err(e) = this.handle_initial_packet(&packet_data, client_addr).await {
                            this.state.record_error();
                            this.state.log(
                                EventLevel::Warn,
                                Protocol::Tftp,
                                format!("TFTP error for {client_addr}: {e}"),
                            );
                        }
                    });
                }
                Err(e) => {
                    self.state.record_error();
                    self.state.log(
                        EventLevel::Error,
                        Protocol::Tftp,
                        format!("TFTP socket recv error: {e}"),
                    );
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
    }

    async fn handle_initial_packet(
        &self,
        buf: &[u8],
        client_addr: SocketAddr,
    ) -> Result<()> {
        let packet = match TftpPacket::parse(buf) {
            Ok(p) => p,
            Err(e) => return Err(e.into()),
        };

        match packet {
            TftpPacket::Rrq {
                filename,
                options,
                ..
            } => {
                self.handle_rrq(filename, options, client_addr).await
            }
            TftpPacket::Wrq { .. } => {
                let err_pkt = TftpPacket::build_error(
                    ERR_ILLEGAL_OP,
                    "Write operations are not supported by PXE server",
                );
                let ephemeral_sock = UdpSocket::bind("0.0.0.0:0").await?;
                ephemeral_sock
                    .send_to(&err_pkt.serialize(), client_addr)
                    .await?;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn handle_rrq(
        &self,
        filename: String,
        options: HashMap<String, String>,
        client_addr: SocketAddr,
    ) -> Result<()> {
        let ephemeral_sock = UdpSocket::bind("0.0.0.0:0").await?;

        // Sanitize path and prevent directory traversal
        let file_path = match self.resolve_safe_path(&filename) {
            Ok(p) => p,
            Err(err) => {
                let (code, msg) = match &err {
                    TftpError::FileNotFound(m) => (ERR_NOT_FOUND, m.clone()),
                    _ => (ERR_ACCESS_VIOLATION, "Access violation".to_string()),
                };
                let err_pkt = TftpPacket::build_error(code, &msg);
                ephemeral_sock
                    .send_to(&err_pkt.serialize(), client_addr)
                    .await?;
                return Err(err.into());
            }
        };

        let file_bytes = match tokio::fs::read(&file_path).await {
            Ok(b) => b,
            Err(e) => {
                let err_pkt = TftpPacket::build_error(ERR_NOT_FOUND, "File read error");
                ephemeral_sock
                    .send_to(&err_pkt.serialize(), client_addr)
                    .await?;
                return Err(TftpError::Io(e).into());
            }
        };

        let file_len = file_bytes.len();
        self.state.log(
            EventLevel::Info,
            Protocol::Tftp,
            format!(
                "RRQ '{filename}' ({file_len} bytes) requested by {client_addr}"
            ),
        );

        // Negotiate options (RFC 2347 / RFC 2348 / RFC 2349)
        let mut negotiated_options = HashMap::new();
        let mut blksize: usize = 512;
        let mut timeout_secs = 2;

        if let Some(val) = options.get("blksize").and_then(|s| s.parse::<usize>().ok()) {
            // Limit blksize to standard Ethernet MTU without fragmentation (1468)
            blksize = val.clamp(512, 1468);
            negotiated_options.insert("blksize".to_string(), blksize.to_string());
        }

        if options.contains_key("tsize") {
            negotiated_options.insert("tsize".to_string(), file_len.to_string());
        }

        if let Some(val) = options.get("timeout").and_then(|s| s.parse::<u64>().ok()) {
            timeout_secs = val.clamp(1, 10);
            negotiated_options.insert("timeout".to_string(), timeout_secs.to_string());
        }

        let timeout_dur = Duration::from_secs(timeout_secs);
        let client_ip_str = match client_addr.ip() {
            std::net::IpAddr::V4(v4) => v4.to_string(),
            _ => client_addr.to_string(),
        };

        // If options were negotiated, send OACK (block 0) and wait for ACK 0
        if !negotiated_options.is_empty() {
            let oack = TftpPacket::Oack {
                options: negotiated_options,
            };
            ephemeral_sock.send_to(&oack.serialize(), client_addr).await?;

            let mut ack_buf = [0u8; 512];
            let mut retries = 0;
            loop {
                match tokio::time::timeout(timeout_dur, ephemeral_sock.recv_from(&mut ack_buf)).await {
                    Ok(Ok((len, _from))) => {
                        if let Ok(TftpPacket::Ack { block: 0 }) = TftpPacket::parse(&ack_buf[..len]) {
                            break;
                        }
                    }
                    _ => {
                        retries += 1;
                        if retries >= 3 {
                            return Err(TftpError::Timeout(0).into());
                        }
                        ephemeral_sock.send_to(&oack.serialize(), client_addr).await?;
                    }
                }
            }
        }

        // Stream DATA blocks
        let total_blocks = if file_len == 0 {
            1
        } else {
            file_len.div_ceil(blksize)
        };

        let mut block_num: u16 = 1;
        let mut offset = 0;
        let mut sent_bytes = 0u64;

        while offset < file_len || (file_len == 0 && block_num == 1) {
            let chunk_end = (offset + blksize).min(file_len);
            let chunk = file_bytes[offset..chunk_end].to_vec();
            let chunk_len = chunk.len();
            let data_pkt = TftpPacket::Data {
                block: block_num,
                data: chunk,
            };
            let serialized = data_pkt.serialize();

            let mut retries = 0;
            let mut acknowledged = false;
            let mut ack_buf = [0u8; 512];

            while !acknowledged {
                ephemeral_sock.send_to(&serialized, client_addr).await?;

                match tokio::time::timeout(timeout_dur, ephemeral_sock.recv_from(&mut ack_buf)).await {
                    Ok(Ok((len, _from))) => {
                        if let Ok(TftpPacket::Ack { block }) = TftpPacket::parse(&ack_buf[..len]) {
                            if block == block_num {
                                acknowledged = true;
                            }
                        } else if let Ok(TftpPacket::Error { code, msg }) = TftpPacket::parse(&ack_buf[..len]) {
                            return Err(TftpError::ClientError { code, msg }.into());
                        }
                    }
                    _ => {
                        retries += 1;
                        if retries >= 4 {
                            return Err(TftpError::Timeout(block_num).into());
                        }
                    }
                }
            }

            offset = chunk_end;
            sent_bytes += chunk_len as u64;
            self.state.record_tftp_bytes(chunk_len as u64);

            let percent = if file_len > 0 {
                ((sent_bytes * 100) / file_len as u64) as u8
            } else {
                100
            };

            self.state.update_client(
                &client_ip_str,
                match client_addr.ip() {
                    std::net::IpAddr::V4(v4) => Some(v4),
                    _ => None,
                },
                None,
                BootStage::TftpTransfer {
                    file: filename.clone(),
                    percent,
                },
            );

            block_num = block_num.wrapping_add(1);
            if chunk_len < blksize {
                break;
            }
        }

        self.state.record_tftp_file();
        self.state.log(
            EventLevel::Success,
            Protocol::Tftp,
            format!(
                "Transfer complete: '{filename}' -> {client_addr} ({sent_bytes} bytes in {total_blocks} blocks)"
            ),
        );

        Ok(())
    }

    fn resolve_safe_path(&self, requested: &str) -> std::result::Result<PathBuf, TftpError> {
        // Strip leading slashes and Windows-style paths
        let clean_name = requested
            .trim_start_matches('/')
            .trim_start_matches('\\')
            .replace('\\', "/");

        // Prevent traversal tricks
        if clean_name.contains("..") {
            return Err(TftpError::AccessViolation(
                "Path traversal sequences forbidden".into(),
            ));
        }

        let full_path = self.tftp_dir.join(&clean_name);
        if !full_path.exists() {
            return Err(TftpError::FileNotFound(clean_name));
        }

        let canonical_root = self
            .tftp_dir
            .canonicalize()
            .unwrap_or_else(|_| self.tftp_dir.clone());
        let canonical_target = full_path
            .canonicalize()
            .map_err(|_| TftpError::FileNotFound(clean_name.clone()))?;

        if !canonical_target.starts_with(&canonical_root) {
            return Err(TftpError::AccessViolation(
                "Path escapes TFTP root directory".into(),
            ));
        }

        Ok(canonical_target)
    }
}
