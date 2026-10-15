pub mod config;
pub mod dhcp;
pub mod error;
pub mod http;
pub mod init;
pub mod state;
pub mod tftp;
pub mod ui;

#[cfg(test)]
mod tests {
    use super::*;
    use dhcp::packet::{ClientArch, DhcpMessageType, DhcpPacket};
    use std::collections::HashMap;
    use std::net::Ipv4Addr;
    use tftp::packet::TftpPacket;

    #[test]
    fn test_dhcp_packet_serialization_roundtrip() {
        let mut options = HashMap::new();
        options.insert(53, vec![DhcpMessageType::Discover as u8]);
        options.insert(55, vec![1, 3, 6, 15]);
        // Architecture 7 = x64 EFI
        options.insert(93, vec![0x00, 0x07]);

        let packet = DhcpPacket {
            op: 1,
            htype: 1,
            hlen: 6,
            hops: 0,
            xid: 0x12345678,
            secs: 10,
            flags: 0x8000,
            ciaddr: Ipv4Addr::new(0, 0, 0, 0),
            yiaddr: Ipv4Addr::new(0, 0, 0, 0),
            siaddr: Ipv4Addr::new(0, 0, 0, 0),
            giaddr: Ipv4Addr::new(0, 0, 0, 0),
            chaddr: [0x52, 0x54, 0x00, 0x12, 0x34, 0x56, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            sname: [0; 64],
            file: [0; 128],
            options,
        };

        let raw = packet.serialize();
        assert!(raw.len() >= 300);

        let parsed = DhcpPacket::parse(&raw).expect("DHCP parsing must succeed");
        assert_eq!(parsed.xid, 0x12345678);
        assert_eq!(parsed.message_type(), Some(DhcpMessageType::Discover));
        assert_eq!(parsed.client_arch(), ClientArch::EfiX64);
        assert_eq!(parsed.mac_string(), "52:54:00:12:34:56");
    }

    #[test]
    fn test_dhcp_bios_arch_detection() {
        let mut options = HashMap::new();
        options.insert(53, vec![DhcpMessageType::Discover as u8]);
        options.insert(93, vec![0x00, 0x00]); // BIOS

        let packet = DhcpPacket {
            op: 1,
            htype: 1,
            hlen: 6,
            hops: 0,
            xid: 1,
            secs: 0,
            flags: 0,
            ciaddr: Ipv4Addr::UNSPECIFIED,
            yiaddr: Ipv4Addr::UNSPECIFIED,
            siaddr: Ipv4Addr::UNSPECIFIED,
            giaddr: Ipv4Addr::UNSPECIFIED,
            chaddr: [0; 16],
            sname: [0; 64],
            file: [0; 128],
            options,
        };

        assert_eq!(packet.client_arch(), ClientArch::BiosX86);
        assert_eq!(packet.client_arch().default_bootfile(), "undionly.kpxe");
    }

    #[test]
    fn test_dhcp_arm64_arch_detection() {
        let mut options = HashMap::new();
        options.insert(93, vec![0x00, 0x0b]); // ARM64 EFI (11)

        let packet = DhcpPacket {
            op: 1,
            htype: 1,
            hlen: 6,
            hops: 0,
            xid: 2,
            secs: 0,
            flags: 0,
            ciaddr: Ipv4Addr::UNSPECIFIED,
            yiaddr: Ipv4Addr::UNSPECIFIED,
            siaddr: Ipv4Addr::UNSPECIFIED,
            giaddr: Ipv4Addr::UNSPECIFIED,
            chaddr: [0; 16],
            sname: [0; 64],
            file: [0; 128],
            options,
        };

        assert_eq!(packet.client_arch(), ClientArch::EfiArm64);
        assert_eq!(packet.client_arch().default_bootfile(), "ipxe-arm64.efi");
    }

    #[test]
    fn test_tftp_rrq_parse_with_options() {
        let mut rrq_raw = vec![0x00, 0x01]; // OP_RRQ
        rrq_raw.extend_from_slice(b"ipxe.efi\0");
        rrq_raw.extend_from_slice(b"octet\0");
        rrq_raw.extend_from_slice(b"blksize\x001468\x00");
        rrq_raw.extend_from_slice(b"tsize\x000\x00");

        let pkt = TftpPacket::parse(&rrq_raw).expect("RRQ should parse successfully");
        if let TftpPacket::Rrq {
            filename,
            mode,
            options,
        } = pkt
        {
            assert_eq!(filename, "ipxe.efi");
            assert_eq!(mode, "octet");
            assert_eq!(options.get("blksize").unwrap(), "1468");
            assert_eq!(options.get("tsize").unwrap(), "0");
        } else {
            panic!("Expected RRQ packet");
        }
    }

    #[test]
    fn test_tftp_data_serialize_parse() {
        let data_pkt = TftpPacket::Data {
            block: 42,
            data: b"hello world".to_vec(),
        };
        let raw = data_pkt.serialize();
        let parsed = TftpPacket::parse(&raw).expect("DATA should parse");
        assert_eq!(
            parsed,
            TftpPacket::Data {
                block: 42,
                data: b"hello world".to_vec()
            }
        );
    }

    #[test]
    fn test_dhcp_user_class_ipxe_detection() {
        let mut options = HashMap::new();
        options.insert(77, b"iPXE".to_vec());

        let packet = DhcpPacket {
            op: 1,
            htype: 1,
            hlen: 6,
            hops: 0,
            xid: 10,
            secs: 0,
            flags: 0,
            ciaddr: Ipv4Addr::UNSPECIFIED,
            yiaddr: Ipv4Addr::UNSPECIFIED,
            siaddr: Ipv4Addr::UNSPECIFIED,
            giaddr: Ipv4Addr::UNSPECIFIED,
            chaddr: [0; 16],
            sname: [0; 64],
            file: [0; 128],
            options,
        };

        assert!(packet.is_ipxe());
    }

    #[test]
    fn test_ipxe_template_generation() {
        let script = http::generate_default_ipxe_script(Ipv4Addr::new(192, 168, 1, 100), 8080);
        assert!(script.starts_with("#!ipxe"));
        assert!(script.contains("set server_ip 192.168.1.100"));
        assert!(script.contains("set http_port 8080"));
        assert!(script.contains("item alpine"));
        assert!(script.contains("item ubuntu"));
        assert!(script.contains("item netboot_xyz"));
    }

    #[test]
    fn test_proxy_dhcp_bios_option43_tlv_and_isolation() {
        let state = std::sync::Arc::new(state::SharedState::new(100));
        let server_ip = Ipv4Addr::new(192, 168, 1, 50);
        let config = dhcp::DhcpConfig {
            server_ip,
            mode: config::ServerMode::Proxy,
            dhcp_port: 67,
            proxy_port: 4011,
            http_port: 8080,
            subnet_mask: Ipv4Addr::new(255, 255, 255, 0),
            router: Some(Ipv4Addr::new(192, 168, 1, 1)),
            dns: Some(Ipv4Addr::new(1, 1, 1, 1)),
            pool_range: None,
        };
        let server = dhcp::DhcpServer::new(state, config);

        let mut req_options = HashMap::new();
        req_options.insert(53, vec![DhcpMessageType::Discover as u8]);
        req_options.insert(93, vec![0, 0]); // BiosX86

        let req = DhcpPacket {
            op: 1,
            htype: 1,
            hlen: 6,
            hops: 0,
            xid: 0x99,
            secs: 0,
            flags: 0,
            ciaddr: Ipv4Addr::UNSPECIFIED,
            yiaddr: Ipv4Addr::UNSPECIFIED,
            siaddr: Ipv4Addr::UNSPECIFIED,
            giaddr: Ipv4Addr::UNSPECIFIED,
            chaddr: [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            sname: [0; 64],
            file: [0; 128],
            options: req_options,
        };

        // Test DHCPOFFER
        let offer = server.build_offer(&req, false, ClientArch::BiosX86).unwrap().unwrap();
        assert_eq!(offer.yiaddr, Ipv4Addr::UNSPECIFIED); // yiaddr MUST be 0.0.0.0
        assert_eq!(offer.siaddr, server_ip);
        assert_eq!(offer.options.get(&60).unwrap(), b"PXEClient");

        // Verify Option 43 is present and TLVs are exact
        let opt43 = offer.options.get(&43).expect("Option 43 must be present for BIOS in Proxy mode");
        let expected_opt43 = vec![
            6, 1, 8,
            8, 7, 0, 0, 1, 192, 168, 1, 50,
            9, 6, 0, 0, 3, 80, 88, 69,
            10, 4, 0, 80, 88, 69,
        ];
        assert_eq!(opt43, &expected_opt43);

        // Verify IP configuration options (1, 3, 51) are strictly absent in Proxy mode
        assert!(!offer.options.contains_key(&1));
        assert!(!offer.options.contains_key(&3));
        assert!(!offer.options.contains_key(&51));

        // Test DHCPACK
        let ack = server.build_ack(&req, false, ClientArch::BiosX86).unwrap().unwrap();
        assert_eq!(ack.yiaddr, Ipv4Addr::UNSPECIFIED); // yiaddr MUST be 0.0.0.0
        assert_eq!(ack.options.get(&60).unwrap(), b"PXEClient");
        assert_eq!(ack.options.get(&43).unwrap(), &expected_opt43);
        assert!(!ack.options.contains_key(&1));
        assert!(!ack.options.contains_key(&3));
        assert!(!ack.options.contains_key(&51));
    }

    #[test]
    fn test_standalone_dhcp_dns_fallback() {
        let state = std::sync::Arc::new(state::SharedState::new(100));
        let server_ip = Ipv4Addr::new(192, 168, 1, 1);
        let config = dhcp::DhcpConfig {
            server_ip,
            mode: config::ServerMode::Standalone,
            dhcp_port: 67,
            proxy_port: 4011,
            http_port: 8080,
            subnet_mask: Ipv4Addr::new(255, 255, 255, 0),
            router: Some(Ipv4Addr::new(192, 168, 1, 1)),
            dns: Some(Ipv4Addr::new(1, 1, 1, 1)),
            pool_range: Some((Ipv4Addr::new(192, 168, 1, 100), Ipv4Addr::new(192, 168, 1, 200))),
        };
        let server = dhcp::DhcpServer::new(state, config);

        let mut req_options = HashMap::new();
        req_options.insert(53, vec![DhcpMessageType::Discover as u8]);

        let req = DhcpPacket {
            op: 1,
            htype: 1,
            hlen: 6,
            hops: 0,
            xid: 0x42,
            secs: 0,
            flags: 0,
            ciaddr: Ipv4Addr::UNSPECIFIED,
            yiaddr: Ipv4Addr::UNSPECIFIED,
            siaddr: Ipv4Addr::UNSPECIFIED,
            giaddr: Ipv4Addr::UNSPECIFIED,
            chaddr: [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            sname: [0; 64],
            file: [0; 128],
            options: req_options,
        };

        let offer = server.build_offer(&req, false, ClientArch::EfiX64).unwrap().unwrap();
        assert_eq!(offer.yiaddr, Ipv4Addr::new(192, 168, 1, 100));
        // Verify DNS option 6 contains 1.1.1.1 + fallback 8.4.4.1
        let dns = offer.options.get(&6).unwrap();
        assert_eq!(dns, &[1, 1, 1, 1, 8, 4, 4, 1]);
    }

    #[test]
    fn test_dhcp_option60_arch_detection() {
        let mut options = HashMap::new();
        options.insert(60, b"PXEClient:Arch:00007:UNDI:003010".to_vec());

        let packet = DhcpPacket {
            op: 1,
            htype: 1,
            hlen: 6,
            hops: 0,
            xid: 1,
            secs: 0,
            flags: 0,
            ciaddr: Ipv4Addr::UNSPECIFIED,
            yiaddr: Ipv4Addr::UNSPECIFIED,
            siaddr: Ipv4Addr::UNSPECIFIED,
            giaddr: Ipv4Addr::UNSPECIFIED,
            chaddr: [0; 16],
            sname: [0; 64],
            file: [0; 128],
            options,
        };

        assert_eq!(packet.client_arch(), ClientArch::EfiX64);
    }
}
