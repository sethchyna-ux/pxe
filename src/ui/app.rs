use crate::config::ServerMode;
use crate::state::{BootStage, EventLevel, Protocol, PxeEvent, SharedState};
use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Paragraph, Row, Table,
};
use ratatui::Terminal;
use std::io::stdout;
use std::net::Ipv4Addr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

pub struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut out = stdout();
        let _ = execute!(out, LeaveAlternateScreen, DisableMouseCapture, Show);
    }
}

pub struct UiApp {
    state: Arc<SharedState>,
    server_ip: Ipv4Addr,
    mode: ServerMode,
    dhcp_port: u16,
    tftp_port: u16,
    http_port: u16,
    scroll_offset: usize,
    auto_scroll: bool,
    filter_protocol: Option<Protocol>,
}

impl UiApp {
    pub fn new(
        state: Arc<SharedState>,
        server_ip: Ipv4Addr,
        mode: ServerMode,
        dhcp_port: u16,
        tftp_port: u16,
        http_port: u16,
    ) -> Self {
        Self {
            state,
            server_ip,
            mode,
            dhcp_port,
            tftp_port,
            http_port,
            scroll_offset: 0,
            auto_scroll: true,
            filter_protocol: None,
        }
    }

    pub fn run(
        mut self,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> std::io::Result<()> {
        enable_raw_mode()?;
        let mut out = stdout();
        execute!(out, EnterAlternateScreen, EnableMouseCapture, Hide)?;
        let _guard = TerminalGuard;

        let backend = CrosstermBackend::new(out);
        let mut terminal = Terminal::new(backend)?;

        loop {
            // Check for shutdown signal from other tasks
            if shutdown_rx.try_recv().is_ok() {
                break;
            }

            terminal.draw(|f| self.render(f))?;

            // Poll keyboard events with low latency
            if !event::poll(Duration::from_millis(50))? {
                continue;
            }

            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('c') => {
                        self.state.clear_events();
                        self.scroll_offset = 0;
                    }
                    KeyCode::Char('p') => {
                        self.auto_scroll = !self.auto_scroll;
                    }
                    KeyCode::Char('d') => {
                        self.filter_protocol = match self.filter_protocol {
                            Some(Protocol::Dhcp) => None,
                            _ => Some(Protocol::Dhcp),
                        };
                    }
                    KeyCode::Char('t') => {
                        self.filter_protocol = match self.filter_protocol {
                            Some(Protocol::Tftp) => None,
                            _ => Some(Protocol::Tftp),
                        };
                    }
                    KeyCode::Char('h') => {
                        self.filter_protocol = match self.filter_protocol {
                            Some(Protocol::Http) => None,
                            _ => Some(Protocol::Http),
                        };
                    }
                    KeyCode::Up => {
                        self.auto_scroll = false;
                        if self.scroll_offset > 0 {
                            self.scroll_offset -= 1;
                        }
                    }
                    KeyCode::Down => {
                        self.scroll_offset += 1;
                    }
                    KeyCode::PageUp => {
                        self.auto_scroll = false;
                        self.scroll_offset = self.scroll_offset.saturating_sub(10);
                    }
                    KeyCode::PageDown => {
                        self.scroll_offset += 10;
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }

    fn render(&mut self, frame: &mut ratatui::Frame) {
        let area = frame.area();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Top Banner
                Constraint::Min(8),    // Split: Events & Clients
                Constraint::Length(4), // Metrics / Telemetry
                Constraint::Length(1), // Footer Help
            ])
            .split(area);

        self.render_header(frame, chunks[0]);

        // Main horizontal split
        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(62), // Event Stream
                Constraint::Percentage(38), // Client Tracker
            ])
            .split(chunks[1]);

        self.render_events(frame, main_chunks[0]);
        self.render_clients(frame, main_chunks[1]);
        self.render_metrics(frame, chunks[2]);
        self.render_footer(frame, chunks[3]);
    }

    fn render_header(&self, frame: &mut ratatui::Frame, area: Rect) {
        let mode_color = match self.mode {
            ServerMode::Proxy => Color::Rgb(255, 180, 50),
            ServerMode::Standalone => Color::Rgb(220, 100, 255),
        };

        let title_line = Line::from(vec![
            Span::styled(" ⚡ PXE-RS ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("│ "),
            Span::styled("IP: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}", self.server_ip), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw(" │ "),
            Span::styled("Mode: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}", self.mode), Style::default().fg(mode_color).add_modifier(Modifier::BOLD)),
            Span::raw(" │ "),
            Span::styled(format!("DHCP:{} ", self.dhcp_port), Style::default().fg(Color::LightBlue)),
            Span::styled(format!("TFTP:{} ", self.tftp_port), Style::default().fg(Color::LightYellow)),
            Span::styled(format!("HTTP:{}", self.http_port), Style::default().fg(Color::LightMagenta)),
        ]);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan));

        let paragraph = Paragraph::new(title_line)
            .block(block)
            .alignment(Alignment::Left);

        frame.render_widget(paragraph, area);
    }

    fn render_events(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let events = self.state.get_events();
        let filtered: Vec<&PxeEvent> = if let Some(proto) = self.filter_protocol {
            events.iter().filter(|e| e.protocol == proto).collect()
        } else {
            events.iter().collect()
        };

        let total_lines = filtered.len();
        let visible_height = area.height.saturating_sub(2) as usize;

        if self.auto_scroll || self.scroll_offset + visible_height > total_lines {
            self.scroll_offset = total_lines.saturating_sub(visible_height);
        }

        let slice = if total_lines > 0 {
            let end = (self.scroll_offset + visible_height).min(total_lines);
            &filtered[self.scroll_offset..end]
        } else {
            &[]
        };

        let mut lines = Vec::new();
        for ev in slice {
            let (proto_str, proto_color) = match ev.protocol {
                Protocol::Dhcp => ("DHCP", Color::LightBlue),
                Protocol::Tftp => ("TFTP", Color::Yellow),
                Protocol::Http => ("HTTP", Color::Magenta),
                Protocol::System => ("SYS ", Color::White),
            };

            let level_style = match ev.level {
                EventLevel::Info => Style::default().fg(Color::White),
                EventLevel::Success => Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                EventLevel::Warn => Style::default().fg(Color::LightYellow),
                EventLevel::Error => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            };

            let line = Line::from(vec![
                Span::styled(format!("{} ", ev.timestamp), Style::default().fg(Color::DarkGray)),
                Span::styled(format!("[{proto_str}] "), Style::default().fg(proto_color).add_modifier(Modifier::BOLD)),
                Span::styled(&ev.message, level_style),
            ]);
            lines.push(line);
        }

        let scroll_badge = if self.auto_scroll {
            "LIVE"
        } else {
            "PAUSED"
        };
        let filter_badge = match self.filter_protocol {
            Some(Protocol::Dhcp) => " [Filter: DHCP]",
            Some(Protocol::Tftp) => " [Filter: TFTP]",
            Some(Protocol::Http) => " [Filter: HTTP]",
            _ => "",
        };

        let block = Block::default()
            .title(format!(" Activity Stream [{scroll_badge}]{filter_badge} "))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::White));

        let paragraph = Paragraph::new(lines).block(block);
        frame.render_widget(paragraph, area);
    }

    fn render_clients(&self, frame: &mut ratatui::Frame, area: Rect) {
        let clients = self.state.get_clients();

        let header = Row::new(vec!["Client MAC", "Arch", "Stage / State"])
            .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
            .bottom_margin(1);

        let rows: Vec<Row> = clients
            .iter()
            .take(12)
            .map(|c| {
                let stage_desc = match &c.stage {
                    BootStage::Discovering => Span::styled("DHCP Discover", Style::default().fg(Color::LightBlue)),
                    BootStage::Offered(ip) => {
                        let ip_str = if ip.is_unspecified() {
                            "Proxy".to_string()
                        } else {
                            ip.to_string()
                        };
                        Span::styled(
                            format!("Offered ({ip_str})"),
                            Style::default().fg(Color::Cyan),
                        )
                    }
                    BootStage::TftpTransfer { file, percent } => Span::styled(
                        format!("TFTP {file} {percent}%"),
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    ),
                    BootStage::HttpBoot(path) => Span::styled(
                        format!("HTTP {path}"),
                        Style::default().fg(Color::Green),
                    ),
                    BootStage::Booted => Span::styled("Booted", Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD)),
                };

                Row::new(vec![
                    Cell::from(Span::styled(&c.mac, Style::default().fg(Color::White))),
                    Cell::from(Span::styled(&c.arch, Style::default().fg(Color::LightCyan))),
                    Cell::from(stage_desc),
                ])
            })
            .collect();

        let widths = [
            Constraint::Length(18),
            Constraint::Length(16),
            Constraint::Min(20),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(
                Block::default()
                    .title(format!(" Active Clients ({}) ", clients.len()))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::LightBlue)),
            );

        frame.render_widget(table, area);
    }

    fn render_metrics(&self, frame: &mut ratatui::Frame, area: Rect) {
        let dhcp_in = self.state.metrics.dhcp_in.load(Ordering::Relaxed);
        let dhcp_out = self.state.metrics.dhcp_out.load(Ordering::Relaxed);
        let tftp_bytes = self.state.metrics.tftp_bytes.load(Ordering::Relaxed);
        let tftp_files = self.state.metrics.tftp_files.load(Ordering::Relaxed);
        let http_reqs = self.state.metrics.http_requests.load(Ordering::Relaxed);
        let errors = self.state.metrics.errors.load(Ordering::Relaxed);

        let tftp_mb = (tftp_bytes as f64) / (1024.0 * 1024.0);

        let stats_line = Line::from(vec![
            Span::styled(" DHCP: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{dhcp_in} in / {dhcp_out} out"), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::raw("  │  "),
            Span::styled("TFTP: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{tftp_mb:.2} MB ({tftp_files} files)"), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw("  │  "),
            Span::styled("HTTP: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{http_reqs} requests"), Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
            Span::raw("  │  "),
            Span::styled("Errors: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{errors}"),
                if errors > 0 {
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Green)
                },
            ),
        ]);

        let block = Block::default()
            .title(" Telemetry & Throughput ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray));

        let paragraph = Paragraph::new(stats_line).block(block);
        frame.render_widget(paragraph, area);
    }

    fn render_footer(&self, frame: &mut ratatui::Frame, area: Rect) {
        let help_spans = vec![
            Span::styled("[Q/Esc]", Style::default().fg(Color::Yellow)),
            Span::raw(" Quit  "),
            Span::styled("[P]", Style::default().fg(Color::Yellow)),
            Span::raw(" Toggle Scroll  "),
            Span::styled("[C]", Style::default().fg(Color::Yellow)),
            Span::raw(" Clear Logs  "),
            Span::styled("[D/T/H]", Style::default().fg(Color::Yellow)),
            Span::raw(" Filter DHCP/TFTP/HTTP  "),
            Span::styled("[↑/↓]", Style::default().fg(Color::Yellow)),
            Span::raw(" Scroll"),
        ];

        let line = Line::from(help_spans);
        let paragraph = Paragraph::new(line).alignment(Alignment::Center);
        frame.render_widget(paragraph, area);
    }
}
