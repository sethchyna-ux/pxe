use crate::error::DhcpError;
use std::collections::HashMap;
use std::net::Ipv4Addr;

pub const MAGIC_COOKIE: [u8; 4] = [99, 130, 83, 99];
pub const BOOTREQUEST: u8 = 1;
pub const BOOTREPLY: u8 = 2;
pub const HTYPE_ETHER: u8 = 1;
pub const HLEN_ETHER: u8 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DhcpMessageType {
    Discover = 1,
    Offer = 2,
    Request = 3,
    Decline = 4,
    Ack = 5,
    Nak = 6,
    Release = 7,
    Inform = 8,
}

impl DhcpMessageType {
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            1 => Some(Self::Discover),
            2 => Some(Self::Offer),
            3 => Some(Self::Request),
            4 => Some(Self::Decline),
            5 => Some(Self::Ack),
            6 => Some(Self::Nak),
            7 => Some(Self::Release),
            8 => Some(Self::Inform),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientArch {
    BiosX86,
    EfiX86,
    EfiX64,
    EfiArm32,
    EfiArm64,
    Unknown(u16),
}

impl ClientArch {
    pub fn from_u16(code: u16) -> Self {
        match code {
            0 => Self::BiosX86,
            6 => Self::EfiX86,
            7 | 9 => Self::EfiX64,
            10 => Self::EfiArm32,
            11 | 12 => Self::EfiArm64,
            other => Self::Unknown(other),
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::BiosX86 => "Legacy BIOS (x86)",
            Self::EfiX86 => "UEFI (x86 32-bit)",
            Self::EfiX64 => "UEFI (x86_64)",
            Self::EfiArm32 => "UEFI (ARM 32-bit)",
            Self::EfiArm64 => "UEFI (ARM64 / AArch64)",
            Self::Unknown(_) => "Unknown Arch",
        }
    }

    pub fn default_bootfile(&self) -> &'static str {
        match self {
            Self::BiosX86 => "undionly.kpxe",
            Self::EfiX64 => "ipxe.efi",
            Self::EfiArm64 => "ipxe-arm64.efi",
            Self::EfiX86 => "ipxe-x86.efi",
            Self::EfiArm32 => "ipxe-arm32.efi",
            Self::Unknown(_) => "ipxe.efi",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DhcpPacket {
    pub op: u8,
    pub htype: u8,
    pub hlen: u8,
    pub hops: u8,
    pub xid: u32,
    pub secs: u16,
    pub flags: u16,
    pub ciaddr: Ipv4Addr,
    pub yiaddr: Ipv4Addr,
    pub siaddr: Ipv4Addr,
    pub giaddr: Ipv4Addr,
    pub chaddr: [u8; 16],
    pub sname: [u8; 64],
    pub file: [u8; 128],
    pub options: HashMap<u8, Vec<u8>>,
}

impl DhcpPacket {
    pub fn parse(buf: &[u8]) -> Result<Self, DhcpError> {
        if buf.len() < 240 {
            return Err(DhcpError::PacketTooShort(buf.len()));
        }

        let cookie: [u8; 4] = [buf[236], buf[237], buf[238], buf[239]];
        if cookie != MAGIC_COOKIE {
            return Err(DhcpError::InvalidMagicCookie(cookie));
        }

        let op = buf[0];
        let htype = buf[1];
        let hlen = buf[2];
        let hops = buf[3];
        let xid = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let secs = u16::from_be_bytes([buf[8], buf[9]]);
        let flags = u16::from_be_bytes([buf[10], buf[11]]);

        let ciaddr = Ipv4Addr::new(buf[12], buf[13], buf[14], buf[15]);
        let yiaddr = Ipv4Addr::new(buf[16], buf[17], buf[18], buf[19]);
        let siaddr = Ipv4Addr::new(buf[20], buf[21], buf[22], buf[23]);
        let giaddr = Ipv4Addr::new(buf[24], buf[25], buf[26], buf[27]);

        let mut chaddr = [0u8; 16];
        chaddr.copy_from_slice(&buf[28..44]);

        let mut sname = [0u8; 64];
        sname.copy_from_slice(&buf[44..108]);

        let mut file = [0u8; 128];
        file.copy_from_slice(&buf[108..236]);

        let mut options = HashMap::new();
        let mut idx = 240;

        while idx < buf.len() {
            let opt_code = buf[idx];
            if opt_code == 0 {
                // Pad option
                idx += 1;
                continue;
            }
            if opt_code == 255 {
                // End option
                break;
            }

            if idx + 1 >= buf.len() {
                break;
            }
            let opt_len = buf[idx + 1] as usize;
            idx += 2;

            if idx + opt_len > buf.len() {
                break;
            }
            let opt_val = buf[idx..idx + opt_len].to_vec();
            options.insert(opt_code, opt_val);
            idx += opt_len;
        }

        Ok(Self {
            op,
            htype,
            hlen,
            hops,
            xid,
            secs,
            flags,
            ciaddr,
            yiaddr,
            siaddr,
            giaddr,
            chaddr,
            sname,
            file,
            options,
        })
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(512);
        buf.push(self.op);
        buf.push(self.htype);
        buf.push(self.hlen);
        buf.push(self.hops);
        buf.extend_from_slice(&self.xid.to_be_bytes());
        buf.extend_from_slice(&self.secs.to_be_bytes());
        buf.extend_from_slice(&self.flags.to_be_bytes());
        buf.extend_from_slice(&self.ciaddr.octets());
        buf.extend_from_slice(&self.yiaddr.octets());
        buf.extend_from_slice(&self.siaddr.octets());
        buf.extend_from_slice(&self.giaddr.octets());
        buf.extend_from_slice(&self.chaddr);
        buf.extend_from_slice(&self.sname);
        buf.extend_from_slice(&self.file);
        buf.extend_from_slice(&MAGIC_COOKIE);

        // Sort options deterministically for testing and RFC conformity
        let mut keys: Vec<&u8> = self.options.keys().collect();
        keys.sort();

        for &key in keys {
            if let Some(val) = self.options.get(&key) {
                buf.push(key);
                buf.push(val.len() as u8);
                buf.extend_from_slice(val);
            }
        }

        // End option (255)
        buf.push(255);

        // Pad to minimum standard BOOTP length if needed (at least 300 bytes)
        while buf.len() < 300 {
            buf.push(0);
        }

        buf
    }

    pub fn mac_string(&self) -> String {
        let len = (self.hlen as usize).min(6);
        self.chaddr[..len]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":")
    }

    pub fn message_type(&self) -> Option<DhcpMessageType> {
        self.options
            .get(&53)
            .and_then(|v| v.first().copied())
            .and_then(DhcpMessageType::from_u8)
    }

    pub fn client_arch(&self) -> ClientArch {
        // Option 93 (Client System Architecture)
        if let Some(val) = self.options.get(&93).filter(|v| v.len() >= 2) {
            let code = u16::from_be_bytes([val[0], val[1]]);
            return ClientArch::from_u16(code);
        }
        ClientArch::BiosX86
    }

    pub fn is_ipxe(&self) -> bool {
        // Check Option 77 (User-Class)
        if self
            .options
            .get(&77)
            .is_some_and(|val| std::str::from_utf8(val).is_ok_and(|s| s.contains("iPXE")))
        {
            return true;
        }
        // Check Option 175 (iPXE Encapsulated Options)
        self.options.contains_key(&175)
    }

    pub fn client_uuid(&self) -> Option<Vec<u8>> {
        // Option 97 (Client Machine Identifier)
        self.options.get(&97).cloned()
    }
}
