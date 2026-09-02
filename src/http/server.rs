use crate::error::{HttpError, Result};
use crate::state::{BootStage, EventLevel, Protocol, SharedState};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::services::ServeDir;

use std::time::Instant;
use serde::Serialize;
use crate::state::{ClientSnapshot, MetricsSnapshot, PxeEvent};

const DASHBOARD_HTML: &str = include_str!("dashboard.html");

#[derive(Clone)]
pub struct HttpServerContext {
    pub state: Arc<SharedState>,
    pub server_ip: Ipv4Addr,
    pub http_port: u16,
    pub http_dir: PathBuf,
    pub start_time: Instant,
}

pub struct HttpServer {
    ctx: HttpServerContext,
}

impl HttpServer {
    pub fn new(
        state: Arc<SharedState>,
        server_ip: Ipv4Addr,
        http_port: u16,
        http_dir: PathBuf,
    ) -> Self {
        Self {
            ctx: HttpServerContext {
                state,
                server_ip,
                http_port,
                http_dir,
                start_time: Instant::now(),
            },
        }
    }

    pub async fn run(self: Arc<Self>) -> Result<()> {
        let addr = SocketAddr::from(([0, 0, 0, 0], self.ctx.http_port));
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                let msg = format!("Failed to bind HTTP port {}: {e}", self.ctx.http_port);
                self.ctx.state.log(EventLevel::Error, Protocol::Http, &msg);
                return Err(HttpError::Startup(msg).into());
            }
        };

        self.ctx.state.log(
            EventLevel::Success,
            Protocol::Http,
            format!(
                "HTTP engine active on TCP {} (root: {})",
                self.ctx.http_port,
                self.ctx.http_dir.display()
            ),
        );

        let ctx = self.ctx.clone();
        let app = Router::new()
            .route("/", get(handle_root_or_dashboard))
            .route("/dashboard", get(handle_dashboard))
            .route("/api/status", get(handle_api_status))
            .route("/boot.ipxe", get(handle_boot_ipxe))
            .fallback_service(ServeDir::new(&self.ctx.http_dir))
            .layer(middleware::from_fn_with_state(
                ctx.clone(),
                request_logger_middleware,
            ))
            .with_state(ctx);

        if let Err(e) = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        {
            return Err(HttpError::Startup(e.to_string()).into());
        }

        Ok(())
    }
}

async fn request_logger_middleware(
    State(ctx): State<HttpServerContext>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    ctx.state.record_http_request();
    let path = req.uri().path().to_string();
    let method = req.method().clone();

    let client_ip_str = match addr.ip() {
        std::net::IpAddr::V4(v4) => v4.to_string(),
        _ => addr.to_string(),
    };

    ctx.state.update_client(
        &client_ip_str,
        match addr.ip() {
            std::net::IpAddr::V4(v4) => Some(v4),
            _ => None,
        },
        None,
        BootStage::HttpBoot(path.clone()),
    );

    ctx.state.log(
        EventLevel::Info,
        Protocol::Http,
        format!("{method} {path} from {addr}"),
    );

    next.run(req).await
}

async fn handle_boot_ipxe(State(ctx): State<HttpServerContext>) -> impl IntoResponse {
    let custom_file = ctx.http_dir.join("boot.ipxe");
    let script_content = if custom_file.exists() {
        match tokio::fs::read_to_string(&custom_file).await {
            Ok(content) => content
                .replace("${server_ip}", &ctx.server_ip.to_string())
                .replace("${http_port}", &ctx.http_port.to_string()),
            Err(_) => generate_default_ipxe_script(ctx.server_ip, ctx.http_port),
        }
    } else {
        generate_default_ipxe_script(ctx.server_ip, ctx.http_port)
    };

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8")),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-cache, no-store, must-revalidate"),
            ),
        ],
        script_content,
    )
}

#[derive(Serialize)]
struct ApiStatusResponse {
    server_ip: String,
    http_port: u16,
    uptime_secs: u64,
    metrics: MetricsSnapshot,
    clients: Vec<ClientSnapshot>,
    events: Vec<PxeEvent>,
}

async fn handle_dashboard() -> impl IntoResponse {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8")),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache, no-store")),
        ],
        DASHBOARD_HTML,
    )
}

async fn handle_root_or_dashboard(
    State(ctx): State<HttpServerContext>,
    req: Request,
) -> impl IntoResponse {
    let accept = req
        .headers()
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if accept.contains("text/html") {
        return (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8")),
                (header::CACHE_CONTROL, HeaderValue::from_static("no-cache, no-store")),
            ],
            DASHBOARD_HTML,
        )
            .into_response();
    }

    let index_file = ctx.http_dir.join("index.html");
    if index_file.exists() {
        match tokio::fs::read_to_string(&index_file).await {
            Ok(content) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"))],
                content,
            )
                .into_response(),
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    } else {
        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8")),
                (header::CACHE_CONTROL, HeaderValue::from_static("no-cache, no-store")),
            ],
            DASHBOARD_HTML,
        )
            .into_response()
    }
}

async fn handle_api_status(State(ctx): State<HttpServerContext>) -> impl IntoResponse {
    let uptime_secs = ctx.start_time.elapsed().as_secs();
    let metrics = ctx.state.metrics_snapshot();
    let clients = ctx.state.client_snapshots();
    let events = ctx.state.get_events();

    let resp = ApiStatusResponse {
        server_ip: ctx.server_ip.to_string(),
        http_port: ctx.http_port,
        uptime_secs,
        metrics,
        clients,
        events,
    };

    match serde_json::to_string(&resp) {
        Ok(json) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, HeaderValue::from_static("application/json")),
                (header::CACHE_CONTROL, HeaderValue::from_static("no-cache, no-store")),
            ],
            json,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Serialization error: {e}"),
        )
            .into_response(),
    }
}

pub fn generate_default_ipxe_script(server_ip: Ipv4Addr, http_port: u16) -> String {
    format!(
        r#"#!ipxe
# Auto-generated by pxe-rs server
set server_ip {server_ip}
set http_port {http_port}
set base_url http://${{server_ip}}:${{http_port}}

# Setup console resolution and theme
console --picture ${{base_url}}/splash.png --x 1024 --y 768 || true

:start
menu PXE Netboot Menu [Host: ${{mac}}]
item --gap --             ---------------- Available Installers ----------------
item alpine               Alpine Linux 3.20 (Fast RAM Boot / MicroOS)
item ubuntu               Ubuntu 24.04 LTS Noble Numbat Live Server
item debian               Debian 12 Bookworm Network Installer
item --gap --             ---------------- System Diagnostics ------------------
item memtest              Memtest86+ Memory Diagnostics
item netboot_xyz          Chainload netboot.xyz (Public Multi-OS Index)
item --gap --             ---------------- Bootloader Control ------------------
item ipxe_shell           Drop to interactive iPXE shell
item reboot               Reboot system
item exit                 Exit and boot local drive
choose --timeout 15000 --default alpine target && goto ${{target}}

:alpine
echo Booting Alpine Linux from ${{base_url}}...
kernel ${{base_url}}/alpine/vmlinuz-virt ip=dhcp alpine_repo=https://dl-cdn.alpinelinux.org/alpine/v3.20/main/ modloop=${{base_url}}/alpine/modloop-virt quiet
initrd ${{base_url}}/alpine/initramfs-virt
boot || goto failed

:ubuntu
echo Booting Ubuntu 24.04 LTS Installer...
kernel ${{base_url}}/ubuntu/vmlinuz ip=dhcp url=${{base_url}}/ubuntu/ubuntu-24.04-live-server-amd64.iso autoinstall ds=nocloud-net;s=${{base_url}}/ubuntu/
initrd ${{base_url}}/ubuntu/initrd
boot || goto failed

:debian
echo Booting Debian 12 Installer...
kernel ${{base_url}}/debian/linux vga=788 --- quiet
initrd ${{base_url}}/debian/initrd.gz
boot || goto failed

:memtest
echo Loading Memtest86+...
kernel ${{base_url}}/tools/memtest.bin
boot || goto failed

:netboot_xyz
echo Loading upstream netboot.xyz catalog...
chain --autofree https://boot.netboot.xyz || goto failed

:ipxe_shell
echo Type 'exit' to return to menu, or run iPXE commands directly.
shell
goto start

:reboot
reboot

:exit
exit

:failed
echo Boot execution failed. Press any key to return to menu...
prompt
goto start
"#
    )
}
