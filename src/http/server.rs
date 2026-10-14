use crate::error::{HttpError, Result};
use crate::state::{BootStage, EventLevel, Protocol, SharedState};
use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path as AxumPath, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::StreamExt;
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tower_http::services::ServeDir;

use crate::state::{ClientSnapshot, MetricsSnapshot, PxeEvent};
use std::time::Instant;

const DASHBOARD_HTML: &str = include_str!("dashboard.html");

#[derive(RustEmbed)]
#[folder = "web/out/"]
struct DashboardAssets;

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
            .route("/dashboard/", get(handle_dashboard))
            .route("/_next/{*path}", get(handle_dashboard_asset))
            .route("/api/status", get(handle_api_status))
            .route("/api/clients/clear", post(handle_api_clients_clear))
            .route(
                "/api/clients/disconnect",
                post(handle_api_clients_disconnect),
            )
            .route("/api/images", get(handle_api_images))
            .route("/api/images/presets", get(handle_api_presets))
            .route("/api/images/download", post(handle_api_download_image))
            .route(
                "/api/images/download-preset",
                post(handle_api_download_preset),
            )
            .route(
                "/api/images/upload",
                post(handle_api_upload_image).layer(DefaultBodyLimit::disable()),
            )
            .route("/boot.ipxe", get(handle_boot_ipxe))
            .route("/autoexec.ipxe", get(handle_autoexec_ipxe))
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

fn embedded_response(path: &str, cache_control: &'static str) -> Option<Response> {
    let asset = DashboardAssets::get(path)?;
    let content_type = match path.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, cache_control)
        .body(Body::from(asset.data.into_owned()))
        .ok()
}

async fn handle_dashboard_asset(AxumPath(path): AxumPath<String>) -> Response {
    embedded_response(
        &format!("_next/{path}"),
        "public, max-age=31536000, immutable",
    )
    .unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
}

async fn request_logger_middleware(
    State(ctx): State<HttpServerContext>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();

    // Dashboard polling and its static assets are control-plane traffic. Keeping
    // them out of PXE metrics prevents the console from creating phantom clients
    // and inflating the HTTP boot request count.
    let is_control_plane = path == "/"
        || path == "/dashboard"
        || path == "/dashboard/"
        || path.starts_with("/_next/")
        || path.starts_with("/api/");
    if is_control_plane {
        return next.run(req).await;
    }

    ctx.state.record_http_request();

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
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-cache, no-store, must-revalidate"),
            ),
        ],
        script_content,
    )
}

async fn handle_autoexec_ipxe(State(ctx): State<HttpServerContext>) -> impl IntoResponse {
    let custom_file = ctx.http_dir.join("autoexec.ipxe");
    let script_content = if custom_file.exists() {
        tokio::fs::read_to_string(&custom_file)
            .await
            .unwrap_or_else(|_| generate_default_autoexec_script(ctx.server_ip, ctx.http_port))
            .replace("${server_ip}", &ctx.server_ip.to_string())
            .replace("${http_port}", &ctx.http_port.to_string())
            .replace("${next-server}", &ctx.server_ip.to_string())
    } else {
        generate_default_autoexec_script(ctx.server_ip, ctx.http_port)
    };

    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            ),
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
    embedded_response("dashboard/index.html", "no-cache, no-store")
        .unwrap_or_else(|| legacy_dashboard_response())
}

fn legacy_dashboard_response() -> Response {
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-cache, no-store"),
            ),
        ],
        DASHBOARD_HTML,
    )
        .into_response()
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
        return embedded_response("index.html", "no-cache, no-store")
            .unwrap_or_else(legacy_dashboard_response);
    }

    let index_file = ctx.http_dir.join("index.html");
    if index_file.exists() {
        match tokio::fs::read_to_string(&index_file).await {
            Ok(content) => (
                StatusCode::OK,
                [(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/html; charset=utf-8"),
                )],
                content,
            )
                .into_response(),
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    } else {
        embedded_response("index.html", "no-cache, no-store")
            .unwrap_or_else(legacy_dashboard_response)
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
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                ),
                (
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("no-cache, no-store"),
                ),
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

#[derive(Deserialize)]
struct DisconnectRequest {
    id: String,
}

async fn handle_api_clients_clear(State(ctx): State<HttpServerContext>) -> impl IntoResponse {
    ctx.state.clear_clients();
    ctx.state.log(
        EventLevel::Info,
        Protocol::Http,
        "Cleared all active boot client sessions via dashboard",
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"status":"cleared"}"#,
    )
}

async fn handle_api_clients_disconnect(
    State(ctx): State<HttpServerContext>,
    axum::Json(request): axum::Json<DisconnectRequest>,
) -> Response {
    if ctx.state.remove_client(&request.id) {
        ctx.state.log(
            EventLevel::Info,
            Protocol::Http,
            format!("Removed boot client session: {}", request.id),
        );
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"status":"disconnected"}"#,
        )
            .into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"status":"not_found"}"#,
        )
            .into_response()
    }
}

#[derive(Serialize, Clone)]
struct BootImageItem {
    name: String,
    rel_path: String,
    category: String,
    size_bytes: u64,
    size_formatted: String,
    is_iso: bool,
    is_kernel: bool,
    is_initrd: bool,
}

#[derive(Serialize, Clone)]
struct ImagePreset {
    id: String,
    name: String,
    category: String,
    description: String,
    size_estimate: String,
    files: Vec<PresetFile>,
}

#[derive(Serialize, Clone)]
struct PresetFile {
    url: String,
    filename: String,
}

#[derive(Deserialize)]
struct DownloadImageRequest {
    category: String,
    filename: String,
    url: String,
}

#[derive(Deserialize)]
struct PresetDownloadRequest {
    id: String,
}

#[derive(Deserialize)]
struct UploadQueryParams {
    category: Option<String>,
    filename: String,
}

fn safe_path_component(value: &str) -> Option<&str> {
    let clean = value.trim();
    if clean.is_empty()
        || clean == "."
        || clean == ".."
        || clean.contains("..")
        || clean.contains('/')
        || clean.contains('\\')
        || clean.contains('\0')
    {
        None
    } else {
        Some(clean)
    }
}

fn valid_download_url(value: &str) -> bool {
    (value.starts_with("https://") || value.starts_with("http://"))
        && !value.chars().any(char::is_control)
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn classify_image(name: &str) -> (bool, bool, bool) {
    let lower = name.to_lowercase();
    (
        lower.ends_with(".iso") || lower.ends_with(".img"),
        lower.contains("vmlinuz") || lower == "linux" || lower.contains("kernel"),
        lower.contains("initrd") || lower.contains("initramfs"),
    )
}

async fn scan_boot_images(http_dir: &std::path::Path) -> Vec<BootImageItem> {
    let mut images = Vec::new();
    let Ok(mut root) = tokio::fs::read_dir(http_dir).await else {
        return images;
    };

    while let Ok(Some(entry)) = root.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name.contains(".tmp") {
            continue;
        }
        let Ok(file_type) = entry.file_type().await else {
            continue;
        };

        if file_type.is_dir() || file_type.is_symlink() {
            if let Ok(mut category_dir) = tokio::fs::read_dir(entry.path()).await {
                while let Ok(Some(file)) = category_dir.next_entry().await {
                    let file_name = file.file_name().to_string_lossy().to_string();
                    if file_name.starts_with('.') || file_name.contains(".tmp") {
                        continue;
                    }
                    let Ok(metadata) = file.metadata().await else {
                        continue;
                    };
                    if !metadata.is_file() {
                        continue;
                    }
                    let (is_iso, is_kernel, is_initrd) = classify_image(&file_name);
                    images.push(BootImageItem {
                        name: file_name.clone(),
                        rel_path: format!("{name}/{file_name}"),
                        category: name.clone(),
                        size_bytes: metadata.len(),
                        size_formatted: format_bytes(metadata.len()),
                        is_iso,
                        is_kernel,
                        is_initrd,
                    });
                }
            }
        } else if file_type.is_file() {
            let size = entry
                .metadata()
                .await
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            let (is_iso, is_kernel, is_initrd) = classify_image(&name);
            images.push(BootImageItem {
                name: name.clone(),
                rel_path: name.clone(),
                category: "root".to_string(),
                size_bytes: size,
                size_formatted: format_bytes(size),
                is_iso,
                is_kernel,
                is_initrd,
            });
        }
    }

    images.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| left.name.cmp(&right.name))
    });
    images
}

fn official_presets() -> Vec<ImagePreset> {
    vec![
        ImagePreset {
            id: "alpine".into(),
            name: "Alpine Linux 3.20".into(),
            category: "alpine".into(),
            description: "Small RAM-boot environment for provisioning and recovery.".into(),
            size_estimate: "~22 MB".into(),
            files: vec![
                PresetFile { url: "https://dl-cdn.alpinelinux.org/alpine/v3.20/releases/x86_64/netboot/vmlinuz-virt".into(), filename: "vmlinuz-virt".into() },
                PresetFile { url: "https://dl-cdn.alpinelinux.org/alpine/v3.20/releases/x86_64/netboot/initramfs-virt".into(), filename: "initramfs-virt".into() },
                PresetFile { url: "https://dl-cdn.alpinelinux.org/alpine/v3.20/releases/x86_64/netboot/modloop-virt".into(), filename: "modloop-virt".into() },
            ],
        },
        ImagePreset {
            id: "debian".into(),
            name: "Debian 12 Bookworm".into(),
            category: "debian".into(),
            description: "Official Debian network installer kernel and ramdisk.".into(),
            size_estimate: "~45 MB".into(),
            files: vec![
                PresetFile { url: "https://deb.debian.org/debian/dists/bookworm/main/installer-amd64/current/images/netboot/debian-installer/amd64/linux".into(), filename: "linux".into() },
                PresetFile { url: "https://deb.debian.org/debian/dists/bookworm/main/installer-amd64/current/images/netboot/debian-installer/amd64/initrd.gz".into(), filename: "initrd.gz".into() },
            ],
        },
        ImagePreset {
            id: "memtest".into(),
            name: "Memtest86+".into(),
            category: "tools".into(),
            description: "Hardware memory diagnostics for UEFI and BIOS systems.".into(),
            size_estimate: "~2 MB".into(),
            files: vec![PresetFile { url: "https://memtest.org/download/v7.00/memtest86+-7.00.bin".into(), filename: "memtest.bin".into() }],
        },
        ImagePreset {
            id: "netboot_xyz".into(),
            name: "netboot.xyz".into(),
            category: "tools".into(),
            description: "Chainload a maintained catalog of operating system installers.".into(),
            size_estimate: "~2.5 MB".into(),
            files: vec![
                PresetFile { url: "https://boot.netboot.xyz/ipxe/netboot.xyz.efi".into(), filename: "netboot.xyz.efi".into() },
                PresetFile { url: "https://boot.netboot.xyz/ipxe/netboot.xyz.kpxe".into(), filename: "netboot.xyz.kpxe".into() },
            ],
        },
    ]
}

async fn handle_api_images(State(ctx): State<HttpServerContext>) -> impl IntoResponse {
    axum::Json(scan_boot_images(&ctx.http_dir).await)
}

async fn handle_api_presets() -> impl IntoResponse {
    axum::Json(official_presets())
}

async fn download_file(
    url: String,
    target: PathBuf,
    label: String,
    state: Arc<SharedState>,
) -> bool {
    let temp = target.with_extension(format!("tmp.{}", std::process::id()));
    let output = tokio::process::Command::new("curl")
        .args([
            "-L",
            "-s",
            "-S",
            "-f",
            "--connect-timeout",
            "15",
            "--retry",
            "2",
            "-o",
        ])
        .arg(&temp)
        .arg(&url)
        .output()
        .await;

    match output {
        Ok(output) if output.status.success() => {
            if tokio::fs::rename(&temp, &target).await.is_ok() {
                let size = tokio::fs::metadata(&target)
                    .await
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                state.log(
                    EventLevel::Success,
                    Protocol::Http,
                    format!("Downloaded {label} ({})", format_bytes(size)),
                );
                true
            } else {
                state.log(
                    EventLevel::Error,
                    Protocol::Http,
                    format!("Could not finalize download: {label}"),
                );
                false
            }
        }
        Ok(output) => {
            let _ = tokio::fs::remove_file(&temp).await;
            state.log(
                EventLevel::Error,
                Protocol::Http,
                format!(
                    "Download failed for {label}: {}",
                    String::from_utf8_lossy(&output.stderr)
                ),
            );
            false
        }
        Err(error) => {
            let _ = tokio::fs::remove_file(&temp).await;
            state.log(
                EventLevel::Error,
                Protocol::Http,
                format!("Could not start download for {label}: {error}"),
            );
            false
        }
    }
}

async fn handle_api_download_image(
    State(ctx): State<HttpServerContext>,
    axum::Json(request): axum::Json<DownloadImageRequest>,
) -> Response {
    let (Some(category), Some(filename)) = (
        safe_path_component(&request.category),
        safe_path_component(&request.filename),
    ) else {
        return (StatusCode::BAD_REQUEST, "Invalid category or filename").into_response();
    };
    if !valid_download_url(&request.url) {
        return (
            StatusCode::BAD_REQUEST,
            "Download URL must use HTTP or HTTPS",
        )
            .into_response();
    }

    let target_dir = ctx.http_dir.join(category);
    if let Err(error) = tokio::fs::create_dir_all(&target_dir).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not create category: {error}"),
        )
            .into_response();
    }
    let target = target_dir.join(filename);
    let label = format!("{category}/{filename}");
    let state = Arc::clone(&ctx.state);
    let url = request.url;
    state.log(
        EventLevel::Info,
        Protocol::Http,
        format!("Scheduled download: {label}"),
    );
    tokio::spawn(async move {
        download_file(url, target, label, state).await;
    });
    (
        StatusCode::ACCEPTED,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"status":"download_started"}"#,
    )
        .into_response()
}

async fn handle_api_download_preset(
    State(ctx): State<HttpServerContext>,
    axum::Json(request): axum::Json<PresetDownloadRequest>,
) -> Response {
    let Some(preset) = official_presets()
        .into_iter()
        .find(|preset| preset.id == request.id)
    else {
        return (StatusCode::NOT_FOUND, "Preset not found").into_response();
    };
    let target_dir = ctx.http_dir.join(&preset.category);
    if let Err(error) = tokio::fs::create_dir_all(&target_dir).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not create category: {error}"),
        )
            .into_response();
    }

    let state = Arc::clone(&ctx.state);
    state.log(
        EventLevel::Info,
        Protocol::Http,
        format!("Scheduled preset installation: {}", preset.name),
    );
    tokio::spawn(async move {
        let mut succeeded = true;
        for file in preset.files {
            let target = target_dir.join(&file.filename);
            let label = format!("{}/{}", preset.category, file.filename);
            succeeded &= download_file(file.url, target, label, Arc::clone(&state)).await;
        }
        state.log(
            if succeeded {
                EventLevel::Success
            } else {
                EventLevel::Warn
            },
            Protocol::Http,
            if succeeded {
                format!("Preset installation complete: {}", preset.name)
            } else {
                format!("Preset installation finished with errors: {}", preset.name)
            },
        );
    });
    (
        StatusCode::ACCEPTED,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"status":"download_started"}"#,
    )
        .into_response()
}

async fn handle_api_upload_image(
    State(ctx): State<HttpServerContext>,
    Query(params): Query<UploadQueryParams>,
    body: Body,
) -> Response {
    let category_value = params.category.as_deref().unwrap_or("custom");
    let (Some(category), Some(filename)) = (
        safe_path_component(category_value),
        safe_path_component(&params.filename),
    ) else {
        return (StatusCode::BAD_REQUEST, "Invalid category or filename").into_response();
    };

    let target_dir = ctx.http_dir.join(category);
    if let Err(error) = tokio::fs::create_dir_all(&target_dir).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not create category: {error}"),
        )
            .into_response();
    }
    let target = target_dir.join(filename);
    let temp = target.with_extension(format!("upload.{}.tmp", std::process::id()));
    let mut file = match tokio::fs::File::create(&temp).await {
        Ok(file) => file,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not create upload: {error}"),
            )
                .into_response();
        }
    };
    let mut stream = body.into_data_stream();
    let mut size = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                let _ = tokio::fs::remove_file(&temp).await;
                return (
                    StatusCode::BAD_REQUEST,
                    format!("Upload stream failed: {error}"),
                )
                    .into_response();
            }
        };
        size += chunk.len() as u64;
        if let Err(error) = file.write_all(&chunk).await {
            let _ = tokio::fs::remove_file(&temp).await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not write upload: {error}"),
            )
                .into_response();
        }
    }
    if let Err(error) = file.flush().await {
        let _ = tokio::fs::remove_file(&temp).await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not finish upload: {error}"),
        )
            .into_response();
    }
    drop(file);
    if let Err(error) = tokio::fs::rename(&temp, &target).await {
        let _ = tokio::fs::remove_file(&temp).await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not install upload: {error}"),
        )
            .into_response();
    }
    ctx.state.log(
        EventLevel::Success,
        Protocol::Http,
        format!("Uploaded {category}/{filename} ({})", format_bytes(size)),
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        format!(r#"{{"status":"uploaded","size":{size}}}"#),
    )
        .into_response()
}

fn generate_default_autoexec_script(server_ip: Ipv4Addr, http_port: u16) -> String {
    format!("#!ipxe\nchain --autofree http://{server_ip}:{http_port}/boot.ipxe || shell\n")
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
