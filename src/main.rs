use clap::Parser;
use pxe::config::{Cli, Commands, detect_primary_ipv4};
use pxe::dhcp::DhcpServer;
use pxe::http::HttpServer;
use pxe::init::initialize_workspace;
use pxe::state::{EventLevel, Protocol, SharedState};
use pxe::tftp::TftpServer;
use pxe::ui::UiApp;
use std::io::IsTerminal;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match &cli.command {
        Some(Commands::Init { target_dir }) => {
            println!(
                "Initializing PXE netboot directories in '{}'...",
                target_dir.display()
            );
            initialize_workspace(target_dir)?;
            println!(" Successfully initialized:");
            println!(
                "  ├── {}/ (Bootloaders: ipxe.efi, undionly.kpxe)",
                target_dir.join("tftpboot").display()
            );
            println!(
                "  └── {}/ (ISOs, kernels, boot.ipxe menu)",
                target_dir.join("httpboot").display()
            );
            println!("\nNext steps:");
            println!("  1. Run 'pxe serve' to start your PXE server");
            println!("  2. See 'tftpboot/README.md' to download official iPXE binaries");
            return Ok(());
        }
        Some(Commands::Status) => {
            println!("=== PXE Server Network Diagnostic ===");
            if let Some(ip) = detect_primary_ipv4() {
                println!("Primary LAN IPv4 detected: {ip}");
            } else {
                println!("Could not automatically determine primary LAN IPv4.");
            }

            // Check if UDP port 67 is occupied
            let port_67_occupied = std::net::UdpSocket::bind("0.0.0.0:67").is_err();
            if port_67_occupied {
                println!("\n⚠️  UDP Port 67 is currently IN USE by another process.");
                #[cfg(target_os = "macos")]
                {
                    println!("   On macOS, this is typically Apple's built-in 'bootpd' service.");
                    println!("   To free port 67 for PXE boot, run:");
                    println!("     sudo launchctl bootout system/com.apple.bootpd\n");
                }
            } else {
                println!("✓ UDP Port 67 is available.");
            }

            if let Some(ip) = detect_primary_ipv4() {
                println!("Recommended launch command:");
                println!("  sudo pxe serve --bind-ip {ip} --mode proxy");
            }
            return Ok(());
        }
        Some(Commands::Serve) | None => {
            // Proceed to run server below
        }
    }

    // Resolve server IP address
    let server_ip = match cli.bind_ip.or_else(detect_primary_ipv4) {
        Some(ip) => ip,
        None => {
            eprintln!("Error: Could not auto-detect primary LAN IPv4 address.");
            eprintln!(
                "Please specify your machine's IP address explicitly using '--bind-ip <IP>'."
            );
            std::process::exit(1);
        }
    };

    // Ensure root directories exist
    if !cli.tftp_dir.exists() {
        let _ = std::fs::create_dir_all(&cli.tftp_dir);
    }
    if !cli.http_dir.exists() {
        let _ = std::fs::create_dir_all(&cli.http_dir);
    }

    // Check privileged ports notice
    let is_privileged = cli.dhcp_port < 1024 || cli.tftp_port < 1024;
    #[cfg(unix)]
    if is_privileged && unsafe { libc::geteuid() } != 0 {
        eprintln!("============================================================");
        eprintln!(
            " WARNING: Binding DHCP ({}) and TFTP ({}) requires root privileges.",
            cli.dhcp_port, cli.tftp_port
        );
        eprintln!(" If binding fails, re-run with: 'sudo pxe ...'");
        eprintln!(
            " Alternatively, for testing use custom ports: '--dhcp-port 6767 --tftp-port 6969'"
        );
        eprintln!("============================================================\n");
    }

    let state = Arc::new(SharedState::new(2000));
    let (shutdown_tx, shutdown_rx) = broadcast::channel::<()>(16);

    state.log(
        EventLevel::Success,
        Protocol::System,
        format!("Starting PXE-RS server on {} [{}]", server_ip, cli.mode),
    );

    // Initialize DHCP Engine
    let dhcp_config = pxe::dhcp::DhcpConfig {
        server_ip,
        mode: cli.mode,
        dhcp_port: cli.dhcp_port,
        proxy_port: cli.proxy_port,
        http_port: cli.http_port,
        subnet_mask: cli.dhcp_mask,
        router: cli.dhcp_router,
        dns: cli.dhcp_dns,
        pool_range: cli.dhcp_range,
    };
    let dhcp_engine = Arc::new(DhcpServer::new(Arc::clone(&state), dhcp_config));

    // Initialize TFTP Engine
    let tftp_engine = Arc::new(TftpServer::new(
        Arc::clone(&state),
        cli.tftp_dir.clone(),
        cli.tftp_port,
    ));

    // Initialize HTTP Engine
    let http_engine = Arc::new(HttpServer::new(
        Arc::clone(&state),
        server_ip,
        cli.http_port,
        cli.http_dir.clone(),
    ));

    // Spawn network services
    let dhcp_task = tokio::spawn(async move {
        if let Err(e) = dhcp_engine.run().await {
            eprintln!("DHCP server terminated: {e}");
        }
    });

    let tftp_task = tokio::spawn(async move {
        if let Err(e) = tftp_engine.run().await {
            eprintln!("TFTP server terminated: {e}");
        }
    });

    let http_task = tokio::spawn(async move {
        if let Err(e) = http_engine.run().await {
            eprintln!("HTTP server terminated: {e}");
        }
    });

    let is_tty = std::io::stdout().is_terminal() && !cli.no_tui;

    if is_tty {
        // Run Ratatui Dashboard in synchronous block
        let ui_state = Arc::clone(&state);
        let shutdown_tx_ui = shutdown_tx.clone();

        let ui_handle = std::thread::spawn(move || {
            let app = UiApp::new(
                ui_state,
                server_ip,
                cli.mode,
                cli.dhcp_port,
                cli.tftp_port,
                cli.http_port,
            );
            let res = app.run(shutdown_rx);
            let _ = shutdown_tx_ui.send(());
            res
        });

        // Listen for OS interrupt in background
        let shutdown_tx_sig = shutdown_tx.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            let _ = shutdown_tx_sig.send(());
        });

        let mut shutdown_sub = shutdown_tx.subscribe();
        tokio::select! {
            _ = shutdown_sub.recv() => {},
            _ = dhcp_task => {},
            _ = tftp_task => {},
            _ = http_task => {},
        }

        let _ = shutdown_tx.send(());
        let _ = ui_handle.join();
    } else {
        println!("Running in headless daemon mode (Ctrl+C to stop)...");
        let mut shutdown_sub = shutdown_tx.subscribe();

        // Print streaming logs to stdout
        let log_state = Arc::clone(&state);
        let logger_task = tokio::spawn(async move {
            let mut last_idx = 0;
            loop {
                let events = log_state.get_events();
                if events.len() > last_idx {
                    for ev in &events[last_idx..] {
                        let proto = match ev.protocol {
                            Protocol::Dhcp => "\x1b[34m[DHCP]\x1b[0m",
                            Protocol::Tftp => "\x1b[33m[TFTP]\x1b[0m",
                            Protocol::Http => "\x1b[35m[HTTP]\x1b[0m",
                            Protocol::System => "\x1b[36m[SYS ]\x1b[0m",
                        };
                        println!("{} {} {}", ev.timestamp, proto, ev.message);
                    }
                    last_idx = events.len();
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\nShutdown signal received. Stopping servers...");
            }
            _ = shutdown_sub.recv() => {}
        }

        logger_task.abort();
    }

    Ok(())
}
