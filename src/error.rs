use std::net::Ipv4Addr;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PxeError {
    #[error("DHCP error: {0}")]
    Dhcp(#[from] DhcpError),

    #[error("TFTP error: {0}")]
    Tftp(#[from] TftpError),

    #[error("HTTP error: {0}")]
    Http(#[from] HttpError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Network error: {0}")]
    Network(String),
}

#[derive(Error, Debug)]
pub enum DhcpError {
    #[error("Packet too short: received {0} bytes, minimum is 240 bytes")]
    PacketTooShort(usize),

    #[error("Invalid magic cookie: expected [99, 130, 83, 99], found {0:?}")]
    InvalidMagicCookie([u8; 4]),

    #[error("Missing required DHCP option: {0}")]
    MissingOption(&'static str),

    #[error("Invalid option format for option {0}")]
    InvalidOption(u8),

    #[error("No IP address available in pool")]
    PoolExhausted,

    #[error("Socket error on interface: {0}")]
    Socket(#[from] std::io::Error),

    #[error("Invalid address: {0}")]
    InvalidAddress(Ipv4Addr),
}

#[derive(Error, Debug)]
pub enum TftpError {
    #[error("Malformed packet: {0}")]
    MalformedPacket(String),

    #[error("Illegal TFTP operation: opcode {0}")]
    IllegalOperation(u16),

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Access violation or directory traversal attempted: {0}")]
    AccessViolation(String),

    #[error("Transfer timed out waiting for ACK block {0}")]
    Timeout(u16),

    #[error("I/O error during file read: {0}")]
    Io(#[from] std::io::Error),

    #[error("Client sent error code {code}: {msg}")]
    ClientError { code: u16, msg: String },
}

#[derive(Error, Debug)]
pub enum HttpError {
    #[error("Server startup error: {0}")]
    Startup(String),

    #[error("Template rendering error: {0}")]
    Template(String),
}

pub type Result<T> = std::result::Result<T, PxeError>;
