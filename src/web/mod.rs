//! Web UI server backed by actix-web.
//!
//! Spawned as a background std::thread with its own tokio runtime.
//! Exposes:
//!   - Static HTML/CSS/JS embedded via rust-embed from the `web/` directory
//!   - `/auth/login` and `/auth/logout` (no auth required)
//!   - `/api/*` routes protected by JWT Bearer auth

pub mod auth;
pub mod routes;
pub mod sse;

use crate::app::AppCommand;
use crate::config::Config;
use actix_web::{get, web, App, HttpRequest, HttpResponse, HttpServer};
use rust_embed::RustEmbed;
use serde::Serialize;
use std::sync::{mpsc, Arc, RwLock};

pub use sse::SseBroadcaster;

// ---------------------------------------------------------------------------
// Embedded static assets
// ---------------------------------------------------------------------------

#[derive(RustEmbed)]
#[folder = "web/"]
struct WebAssets;

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

/// A single file entry in the tracks listing.
#[derive(Debug, Clone, Serialize)]
pub struct TrackEntry {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub duration_secs: Option<f32>,
    pub channels: Option<u8>,
    /// Unix timestamp of the file's last-modified time.
    pub modified_secs: Option<u64>,
}

/// Status snapshot shared between App loop and web server.
#[derive(Default, Clone, Serialize)]
pub struct SharedStatus {
    pub playing: bool,
    pub recording: bool,
    pub monitoring: bool,
    pub current_track: Option<String>,
    pub position_secs: Option<f32>,
    pub duration_secs: Option<f32>,
    pub volume: u8,
    pub input_levels: Vec<f32>,
    pub output_levels: Vec<f32>,
    pub recording_path: Option<String>,
    pub recording_size_bytes: u64,
    pub recording_duration_secs: f32,
    pub files_written: u32,
    pub tracks_dir: String,
    pub track_list: Vec<TrackEntry>,
}

// ---------------------------------------------------------------------------
// Embedded asset handler
// ---------------------------------------------------------------------------

fn serve_embedded(path: &str) -> Option<HttpResponse> {
    let asset = WebAssets::get(path)?;
    let mime = mime_guess::from_path(path)
        .first_raw()
        .unwrap_or("application/octet-stream");
    Some(
        HttpResponse::Ok()
            .content_type(mime)
            .body(asset.data.into_owned()),
    )
}

/// Serve `web/index.html` at GET /
#[get("/")]
async fn index() -> HttpResponse {
    serve_embedded("index.html").unwrap_or_else(|| HttpResponse::NotFound().finish())
}

/// Serve static assets under `/assets/*`
async fn static_assets(req: HttpRequest) -> HttpResponse {
    let path = req.match_info().query("tail");
    let asset_path = format!("assets/{}", path);
    serve_embedded(&asset_path).unwrap_or_else(|| HttpResponse::NotFound().finish())
}

/// Serve any named HTML page (dashboard, files, recording, settings)
async fn serve_page(req: HttpRequest) -> HttpResponse {
    let page = req.match_info().query("page");
    let asset_path = format!("{}.html", page);
    serve_embedded(&asset_path).unwrap_or_else(|| HttpResponse::NotFound().finish())
}

// ---------------------------------------------------------------------------
// spawn()
// ---------------------------------------------------------------------------

/// Start the actix-web server in a background thread.
///
/// Returns `None` if `config.web.enabled` is false.
pub fn spawn(
    config: Arc<RwLock<Config>>,
    status: Arc<RwLock<SharedStatus>>,
    cmd_tx: mpsc::SyncSender<AppCommand>,
    broadcaster: SseBroadcaster,
    controlling_tty: Option<String>,
) -> Option<std::thread::JoinHandle<()>> {
    let enabled = { config.read().unwrap().web.enabled };
    if !enabled {
        return None;
    }

    let port = { config.read().unwrap().web.port };
    let session_timeout_hours = { config.read().unwrap().web.session_timeout_hours };

    let handle = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async move {
            let jwt_cfg = auth::JwtConfig::new(session_timeout_hours);
            let jwt_cfg = web::Data::new(jwt_cfg);

            let app_state = web::Data::new(routes::AppState {
                config: config.clone(),
                status: status.clone(),
                cmd_tx: cmd_tx.clone(),
                broadcaster: broadcaster.clone(),
                peaks_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(2)),
                peaks_progress: std::sync::Arc::new(routes::PeaksProgress::default()),
                controlling_tty: controlling_tty.clone(),
            });

            // Pre-compute missing peaks in the background at startup.
            {
                let tracks_dir = app_state.status.read().unwrap().tracks_dir.clone();
                let sem = app_state.peaks_semaphore.clone();
                let progress = app_state.peaks_progress.clone();
                routes::spawn_precompute(tracks_dir, sem, progress);
            }

            let server = HttpServer::new(move || {
                App::new()
                    .app_data(jwt_cfg.clone())
                    .app_data(app_state.clone())
                    // Auth routes (no auth required)
                    .service(routes::login)
                    .service(routes::logout)
                    // Root and static assets (no auth required)
                    .service(index)
                    .route("/assets/{tail:.*}", web::get().to(static_assets))
                    // Page routes (no auth, but the pages themselves redirect to login if needed)
                    .route("/{page}", web::get().to(serve_page))
                    // API routes (auth via BearerAuth extractor in each handler)
                    .service(routes::get_status)
                    .service(routes::sse_stream)
                    .service(routes::list_files)
                    // peaks must be registered before download to avoid ambiguity
                    .service(routes::get_peaks)
                    .service(routes::download_file)
                    .service(routes::delete_file)
                    .service(routes::transport_play)
                    .service(routes::transport_stop)
                    .service(routes::transport_prev)
                    .service(routes::transport_next)
                    .service(routes::transport_seek)
                    .service(routes::transport_load)
                    .service(routes::recording_start)
                    .service(routes::recording_stop)
                    .service(routes::get_config)
                    .service(routes::patch_config)
                    .service(routes::network_scan)
                    .service(routes::network_connect)
                    .service(routes::network_ap_password)
                    .service(routes::devices_playback)
                    .service(routes::devices_capture)
                    .service(routes::peaks_status)
                    .service(routes::peaks_precompute)
                    .service(routes::system_restart)
                    .service(routes::system_reboot)
                    .service(routes::system_shutdown)
                    .service(routes::system_info)
                    .service(routes::file_info)
                    .service(routes::file_detail)
            })
            .bind(format!("0.0.0.0:{}", port))
            .expect("bind failed")
            .run();

            let _ = server.await;
        });
    });

    Some(handle)
}
