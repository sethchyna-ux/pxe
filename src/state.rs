use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum EventLevel {
    Info,
    Success,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Protocol {
    Dhcp,
    Tftp,
    Http,
    System,
}

#[derive(Debug, Clone, Serialize)]
pub struct PxeEvent {
    pub timestamp: String,
    pub level: EventLevel,
    pub protocol: Protocol,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum BootStage {
    Discovering,
    Offered(Ipv4Addr),
    TftpTransfer { file: String, percent: u8 },
    HttpBoot(String),
    Booted,
}

#[derive(Debug, Clone)]
pub struct ClientSession {
    pub mac: String,
    pub ip: Option<Ipv4Addr>,
    pub arch: String,
    pub stage: BootStage,
    pub last_seen: Instant,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientSnapshot {
    pub mac: String,
    pub ip: Option<String>,
    pub arch: String,
    pub stage: BootStage,
    pub stage_text: String,
    pub last_seen_secs_ago: u64,
}

#[derive(Default)]
pub struct Metrics {
    pub dhcp_in: AtomicU64,
    pub dhcp_out: AtomicU64,
    pub tftp_bytes: AtomicU64,
    pub tftp_files: AtomicU64,
    pub http_requests: AtomicU64,
    pub errors: AtomicU64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricsSnapshot {
    pub dhcp_in: u64,
    pub dhcp_out: u64,
    pub tftp_bytes: u64,
    pub tftp_files: u64,
    pub http_requests: u64,
    pub errors: u64,
}

pub struct SharedState {
    pub metrics: Metrics,
    events: Mutex<Vec<PxeEvent>>,
    clients: Mutex<HashMap<String, ClientSession>>,
    max_events: usize,
}

impl SharedState {
    pub fn new(max_events: usize) -> Self {
        Self {
            metrics: Metrics::default(),
            events: Mutex::new(Vec::with_capacity(max_events)),
            clients: Mutex::new(HashMap::new()),
            max_events,
        }
    }

    pub fn log(&self, level: EventLevel, protocol: Protocol, message: impl Into<String>) {
        let msg = message.into();
        let now = chrono_or_simple_time();
        let event = PxeEvent {
            timestamp: now,
            level,
            protocol,
            message: msg,
        };

        if let Ok(mut lock) = self.events.lock() {
            if lock.len() >= self.max_events {
                lock.remove(0);
            }
            lock.push(event);
        }
    }

    pub fn get_events(&self) -> Vec<PxeEvent> {
        self.events.lock().map(|l| l.clone()).unwrap_or_default()
    }

    pub fn clear_events(&self) {
        if let Ok(mut l) = self.events.lock() {
            l.clear();
        }
    }

    pub fn update_client(
        &self,
        mac: &str,
        ip: Option<Ipv4Addr>,
        arch: Option<&str>,
        stage: BootStage,
    ) {
        if let Ok(mut lock) = self.clients.lock() {
            let entry = lock.entry(mac.to_string()).or_insert_with(|| ClientSession {
                mac: mac.to_string(),
                ip,
                arch: arch.unwrap_or("Unknown").to_string(),
                stage: stage.clone(),
                last_seen: Instant::now(),
            });

            if let Some(ip) = ip {
                entry.ip = Some(ip);
            }
            if let Some(arch) = arch {
                entry.arch = arch.to_string();
            }
            entry.stage = stage;
            entry.last_seen = Instant::now();
        }
    }

    pub fn get_clients(&self) -> Vec<ClientSession> {
        if let Ok(lock) = self.clients.lock() {
            let mut list: Vec<ClientSession> = lock.values().cloned().collect();
            list.sort_by_key(|a| std::cmp::Reverse(a.last_seen));
            list
        } else {
            Vec::new()
        }
    }

    pub fn record_dhcp_in(&self) {
        self.metrics.dhcp_in.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_dhcp_out(&self) {
        self.metrics.dhcp_out.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_tftp_bytes(&self, bytes: u64) {
        self.metrics.tftp_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn record_tftp_file(&self) {
        self.metrics.tftp_files.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_http_request(&self) {
        self.metrics.http_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_error(&self) {
        self.metrics.errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn metrics_snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            dhcp_in: self.metrics.dhcp_in.load(Ordering::Relaxed),
            dhcp_out: self.metrics.dhcp_out.load(Ordering::Relaxed),
            tftp_bytes: self.metrics.tftp_bytes.load(Ordering::Relaxed),
            tftp_files: self.metrics.tftp_files.load(Ordering::Relaxed),
            http_requests: self.metrics.http_requests.load(Ordering::Relaxed),
            errors: self.metrics.errors.load(Ordering::Relaxed),
        }
    }

    pub fn client_snapshots(&self) -> Vec<ClientSnapshot> {
        let clients = self.get_clients();
        let now = Instant::now();
        clients
            .into_iter()
            .map(|c| {
                let stage_text = match &c.stage {
                    BootStage::Discovering => "Discovering (DHCP)".to_string(),
                    BootStage::Offered(ip) => format!("Offered ({ip})"),
                    BootStage::TftpTransfer { file, percent } => format!("TFTP {file} ({percent}%)"),
                    BootStage::HttpBoot(path) => format!("HTTP {path}"),
                    BootStage::Booted => "Booted".to_string(),
                };
                ClientSnapshot {
                    mac: c.mac,
                    ip: c.ip.map(|i| i.to_string()),
                    arch: c.arch,
                    stage: c.stage,
                    stage_text,
                    last_seen_secs_ago: now.duration_since(c.last_seen).as_secs(),
                }
            })
            .collect()
    }
}

fn chrono_or_simple_time() -> String {
    // Generate simple HH:MM:SS from system time without extra dependencies
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(dur) => {
            let secs = dur.as_secs();
            let hours = (secs / 3600) % 24;
            let mins = (secs / 60) % 60;
            let sec = secs % 60;
            format!("{:02}:{:02}:{:02}", hours, mins, sec)
        }
        Err(_) => "00:00:00".to_string(),
    }
}
