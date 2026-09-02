use crate::error::TftpError;
use std::collections::HashMap;

pub const OP_RRQ: u16 = 1;
pub const OP_WRQ: u16 = 2;
pub const OP_DATA: u16 = 3;
pub const OP_ACK: u16 = 4;
pub const OP_ERROR: u16 = 5;
pub const OP_OACK: u16 = 6;

pub const ERR_NOT_FOUND: u16 = 1;
pub const ERR_ACCESS_VIOLATION: u16 = 2;
pub const ERR_ILLEGAL_OP: u16 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TftpPacket {
    Rrq {
        filename: String,
        mode: String,
        options: HashMap<String, String>,
    },
    Wrq {
        filename: String,
        mode: String,
    },
    Data {
        block: u16,
        data: Vec<u8>,
    },
    Ack {
        block: u16,
    },
    Error {
        code: u16,
        msg: String,
    },
    Oack {
        options: HashMap<String, String>,
    },
}

impl TftpPacket {
    pub fn parse(buf: &[u8]) -> Result<Self, TftpError> {
        if buf.len() < 2 {
            return Err(TftpError::MalformedPacket("Buffer too small".into()));
        }

        let opcode = u16::from_be_bytes([buf[0], buf[1]]);
        match opcode {
            OP_RRQ | OP_WRQ => {
                let rest = &buf[2..];
                let mut tokens = Vec::new();
                let mut start = 0;
                for (i, &b) in rest.iter().enumerate() {
                    if b == 0 {
                        let token = String::from_utf8_lossy(&rest[start..i]).to_string();
                        tokens.push(token);
                        start = i + 1;
                    }
                }

                if tokens.len() < 2 {
                    return Err(TftpError::MalformedPacket(
                        "RRQ/WRQ requires filename and mode".into(),
                    ));
                }

                let raw_filename = tokens[0].clone();
                let filename = raw_filename
                    .trim_matches(|c: char| !c.is_ascii_graphic())
                    .to_string();
                let mode = tokens[1].to_lowercase();

                if opcode == OP_WRQ {
                    return Ok(Self::Wrq { filename, mode });
                }

                let mut options = HashMap::new();
                let mut idx = 2;
                while idx + 1 < tokens.len() {
                    let opt_name = tokens[idx].to_lowercase();
                    let opt_val = tokens[idx + 1].clone();
                    options.insert(opt_name, opt_val);
                    idx += 2;
                }

                Ok(Self::Rrq {
                    filename,
                    mode,
                    options,
                })
            }
            OP_DATA => {
                if buf.len() < 4 {
                    return Err(TftpError::MalformedPacket("DATA packet too short".into()));
                }
                let block = u16::from_be_bytes([buf[2], buf[3]]);
                let data = buf[4..].to_vec();
                Ok(Self::Data { block, data })
            }
            OP_ACK => {
                if buf.len() < 4 {
                    return Err(TftpError::MalformedPacket("ACK packet too short".into()));
                }
                let block = u16::from_be_bytes([buf[2], buf[3]]);
                Ok(Self::Ack { block })
            }
            OP_ERROR => {
                if buf.len() < 4 {
                    return Err(TftpError::MalformedPacket("ERROR packet too short".into()));
                }
                let code = u16::from_be_bytes([buf[2], buf[3]]);
                let msg_slice = if let Some(null_pos) = buf[4..].iter().position(|&b| b == 0) {
                    &buf[4..4 + null_pos]
                } else {
                    &buf[4..]
                };
                let msg = String::from_utf8_lossy(msg_slice).to_string();
                Ok(Self::Error { code, msg })
            }
            OP_OACK => {
                let rest = &buf[2..];
                let mut tokens = Vec::new();
                let mut start = 0;
                for (i, &b) in rest.iter().enumerate() {
                    if b == 0 {
                        let token = String::from_utf8_lossy(&rest[start..i]).to_string();
                        tokens.push(token);
                        start = i + 1;
                    }
                }
                let mut options = HashMap::new();
                let mut idx = 0;
                while idx + 1 < tokens.len() {
                    options.insert(tokens[idx].to_lowercase(), tokens[idx + 1].clone());
                    idx += 2;
                }
                Ok(Self::Oack { options })
            }
            other => Err(TftpError::IllegalOperation(other)),
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(516);
        match self {
            Self::Rrq {
                filename,
                mode,
                options,
            } => {
                buf.extend_from_slice(&OP_RRQ.to_be_bytes());
                buf.extend_from_slice(filename.as_bytes());
                buf.push(0);
                buf.extend_from_slice(mode.as_bytes());
                buf.push(0);
                for (k, v) in options {
                    buf.extend_from_slice(k.as_bytes());
                    buf.push(0);
                    buf.extend_from_slice(v.as_bytes());
                    buf.push(0);
                }
            }
            Self::Wrq { filename, mode } => {
                buf.extend_from_slice(&OP_WRQ.to_be_bytes());
                buf.extend_from_slice(filename.as_bytes());
                buf.push(0);
                buf.extend_from_slice(mode.as_bytes());
                buf.push(0);
            }
            Self::Data { block, data } => {
                buf.extend_from_slice(&OP_DATA.to_be_bytes());
                buf.extend_from_slice(&block.to_be_bytes());
                buf.extend_from_slice(data);
            }
            Self::Ack { block } => {
                buf.extend_from_slice(&OP_ACK.to_be_bytes());
                buf.extend_from_slice(&block.to_be_bytes());
            }
            Self::Error { code, msg } => {
                buf.extend_from_slice(&OP_ERROR.to_be_bytes());
                buf.extend_from_slice(&code.to_be_bytes());
                buf.extend_from_slice(msg.as_bytes());
                buf.push(0);
            }
            Self::Oack { options } => {
                buf.extend_from_slice(&OP_OACK.to_be_bytes());
                for (k, v) in options {
                    buf.extend_from_slice(k.as_bytes());
                    buf.push(0);
                    buf.extend_from_slice(v.as_bytes());
                    buf.push(0);
                }
            }
        }
        buf
    }

    pub fn build_error(code: u16, msg: &str) -> Self {
        Self::Error {
            code,
            msg: msg.to_string(),
        }
    }
}
