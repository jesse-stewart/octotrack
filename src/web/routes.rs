//! All route handlers for the octotrack web API.

use crate::app::AppCommand;
use crate::config::Config;
use crate::web::auth::{BearerAuth, JwtConfig};
use crate::web::sse::SseBroadcaster;
use crate::web::SharedStatus;
use actix_files::NamedFile;
use actix_web::{delete, get, patch, post, web, HttpRequest, HttpResponse, Responder};
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::{Path, PathBuf};
extern crate libc;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, RwLock};
use tokio::sync::Semaphore;

// ---------------------------------------------------------------------------
// App data types
// ---------------------------------------------------------------------------

/// Shared counters for the peaks precomputation task.
#[derive(Default)]
pub struct PeaksProgress {
    pub total: AtomicU32,
    pub done: AtomicU32,
    pub failed: AtomicU32,
}

pub struct AppState {
    pub config: Arc<RwLock<Config>>,
    pub status: Arc<RwLock<SharedStatus>>,
    pub cmd_tx: mpsc::SyncSender<AppCommand>,
    pub broadcaster: SseBroadcaster,
    /// Limits concurrent ffmpeg peak-computation jobs to avoid bogging the server.
    pub peaks_semaphore: Arc<Semaphore>,
    /// Progress counters for the background precompute task.
    pub peaks_progress: Arc<PeaksProgress>,
    /// Controlling terminal at startup (e.g. /dev/tty1), if any.
    pub controlling_tty: Option<String>,
}

// ---------------------------------------------------------------------------
// Auth routes
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct LoginBody {
    password: String,
}

#[derive(Serialize)]
struct TokenResponse {
    token: String,
}

#[post("/auth/login")]
pub async fn login(
    body: web::Json<LoginBody>,
    jwt_cfg: web::Data<JwtConfig>,
    state: web::Data<AppState>,
) -> impl Responder {
    let password_hash = {
        let cfg = state.config.read().unwrap();
        cfg.web.password_hash.clone()
    };

    if password_hash.is_empty() {
        return HttpResponse::Unauthorized().finish();
    }

    let parsed = match PasswordHash::new(&password_hash) {
        Ok(h) => h,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };

    let ok = Argon2::default()
        .verify_password(body.password.as_bytes(), &parsed)
        .is_ok();

    if !ok {
        return HttpResponse::Unauthorized().finish();
    }

    let token = match jwt_cfg.issue_token() {
        Ok(t) => t,
        Err(_) => return HttpResponse::InternalServerError().finish(),
    };

    // Set a session cookie in addition to returning the token in the body.
    let cookie = actix_web::cookie::Cookie::build("session", token.clone())
        .http_only(true)
        .same_site(actix_web::cookie::SameSite::Strict)
        .path("/")
        .finish();

    HttpResponse::Ok()
        .cookie(cookie)
        .json(TokenResponse { token })
}

#[post("/auth/logout")]
pub async fn logout() -> impl Responder {
    // Invalidate cookie by setting it to expire immediately.
    let cookie = actix_web::cookie::Cookie::build("session", "")
        .http_only(true)
        .same_site(actix_web::cookie::SameSite::Strict)
        .path("/")
        .max_age(actix_web::cookie::time::Duration::seconds(0))
        .finish();
    HttpResponse::Ok().cookie(cookie).finish()
}

// ---------------------------------------------------------------------------
// /api/status
// ---------------------------------------------------------------------------

#[get("/api/status")]
pub async fn get_status(_auth: BearerAuth, state: web::Data<AppState>) -> impl Responder {
    let status = state.status.read().unwrap().clone();
    HttpResponse::Ok().json(status)
}

// ---------------------------------------------------------------------------
// /api/events (SSE)
// ---------------------------------------------------------------------------

#[get("/api/events")]
pub async fn sse_stream(_auth: BearerAuth, state: web::Data<AppState>) -> impl Responder {
    let rx = state.broadcaster.subscribe();
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(|msg| {
        std::future::ready(match msg {
            Ok(json) => {
                let payload = format!("data: {}\n\n", json);
                Some(Ok::<_, io::Error>(actix_web::web::Bytes::from(payload)))
            }
            Err(_) => None,
        })
    });

    HttpResponse::Ok()
        .content_type("text/event-stream")
        .insert_header(("Cache-Control", "no-cache"))
        .insert_header(("X-Accel-Buffering", "no"))
        .streaming(stream)
}

// ---------------------------------------------------------------------------
// /api/files
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct FilesQuery {
    #[serde(default = "default_page")]
    page: usize,
    #[serde(default = "default_per_page")]
    per_page: usize,
}
fn default_page() -> usize {
    1
}
fn default_per_page() -> usize {
    20
}

#[get("/api/files")]
pub async fn list_files(
    _auth: BearerAuth,
    state: web::Data<AppState>,
    query: web::Query<FilesQuery>,
) -> impl Responder {
    let status = state.status.read().unwrap();
    // Sort by modified_secs descending (most recent first).
    let mut sorted = status.track_list.clone();
    sorted.sort_by(|a, b| b.modified_secs.cmp(&a.modified_secs));
    let total = sorted.len();
    let per_page = query.per_page.max(1);
    let pages = total.div_ceil(per_page);
    let page = query.page.clamp(1, pages.max(1));
    let start = (page - 1) * per_page;
    let items = sorted[start.min(total)..((start + per_page).min(total))].to_vec();
    HttpResponse::Ok().json(serde_json::json!({
        "items": items,
        "total": total,
        "page": page,
        "per_page": per_page,
        "pages": pages,
    }))
}

#[get("/api/files/{path:.*}/info")]
pub async fn file_info(
    _auth: BearerAuth,
    state: web::Data<AppState>,
    path: web::Path<String>,
) -> impl Responder {
    let name = path.into_inner();
    let status = state.status.read().unwrap();
    match status.track_list.iter().find(|e| e.name == name) {
        Some(entry) => HttpResponse::Ok().json(entry),
        None => HttpResponse::NotFound().finish(),
    }
}

#[get("/api/files/{path:.*}/detail")]
pub async fn file_detail(
    _auth: BearerAuth,
    state: web::Data<AppState>,
    path: web::Path<String>,
) -> impl Responder {
    let name = path.into_inner();
    let (tracks_dir, entry) = {
        let s = state.status.read().unwrap();
        let entry = s.track_list.iter().find(|e| e.name == name).cloned();
        (s.tracks_dir.clone(), entry)
    };
    let entry = match entry {
        Some(e) => e,
        None => return HttpResponse::NotFound().finish(),
    };
    let file_path = std::path::Path::new(&tracks_dir).join(&entry.name);

    // Run ffprobe -show_streams -show_format -of json
    let probe_out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(&file_path)
        .output();

    let (codec, sample_rate, bits_per_sample, bit_rate, format_name, format_long) =
        match probe_out.ok().and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).into_owned();
            serde_json::from_str::<Value>(&s).ok()
        }) {
            Some(j) => {
                let stream = j.get("streams").and_then(|s| s.as_array()).and_then(|a| {
                    a.iter()
                        .find(|s| s.get("codec_type").and_then(|t| t.as_str()) == Some("audio"))
                });
                let fmt = j.get("format");
                (
                    stream
                        .and_then(|s| s.get("codec_name"))
                        .and_then(|v| v.as_str())
                        .map(str::to_owned),
                    stream
                        .and_then(|s| s.get("sample_rate"))
                        .and_then(|v| v.as_str())
                        .and_then(|v| v.parse::<u32>().ok()),
                    stream
                        .and_then(|s| s.get("bits_per_sample"))
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32)
                        .or_else(|| {
                            stream
                                .and_then(|s| s.get("bits_per_raw_sample"))
                                .and_then(|v| v.as_str())
                                .and_then(|v| v.parse().ok())
                        }),
                    fmt.and_then(|f| f.get("bit_rate"))
                        .and_then(|v| v.as_str())
                        .and_then(|v| v.parse::<u64>().ok()),
                    fmt.and_then(|f| f.get("format_name"))
                        .and_then(|v| v.as_str())
                        .map(str::to_owned),
                    fmt.and_then(|f| f.get("format_long_name"))
                        .and_then(|v| v.as_str())
                        .map(str::to_owned),
                )
            }
            None => (None, None, None, None, None, None),
        };

    HttpResponse::Ok().json(serde_json::json!({
        "name":             entry.name,
        "path":             entry.path,
        "size_bytes":       entry.size_bytes,
        "duration_secs":    entry.duration_secs,
        "channels":         entry.channels,
        "modified_secs":    entry.modified_secs,
        "codec":            codec,
        "sample_rate":      sample_rate,
        "bits_per_sample":  bits_per_sample,
        "bit_rate_bps":     bit_rate,
        "format_name":      format_name,
        "format_long_name": format_long,
    }))
}

#[get("/api/files/{path:.*}/peaks")]
pub async fn get_peaks(
    _auth: BearerAuth,
    state: web::Data<AppState>,
    path: web::Path<String>,
) -> impl Responder {
    let tracks_dir = {
        let s = state.status.read().unwrap();
        s.tracks_dir.clone()
    };

    let file_path = resolve_path(&tracks_dir, &path.into_inner());
    let file_path = match file_path {
        Some(p) => p,
        None => return HttpResponse::BadRequest().body("invalid path"),
    };

    if !file_path.exists() {
        return HttpResponse::NotFound().finish();
    }

    // Check if currently being recorded
    {
        let s = state.status.read().unwrap();
        if let Some(ref rp) = s.recording_path {
            if file_path == std::path::Path::new(rp) {
                return HttpResponse::Conflict().finish();
            }
        }
    }

    // Serve from cache if valid.
    if let Some(peaks) = read_peaks_cache(&file_path) {
        return HttpResponse::Ok().json(peaks);
    }

    // Try to acquire a slot — if all slots are busy return 202 so the
    // client knows to retry rather than blocking a worker thread.
    let permit = match state.peaks_semaphore.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return HttpResponse::Accepted().body("pending"),
    };

    let output = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        compute_and_cache_peaks(&file_path)
    })
    .await;

    match output {
        Ok(Some(peaks)) => HttpResponse::Ok().json(peaks),
        _ => HttpResponse::InternalServerError().body("ffmpeg error"),
    }
}

#[get("/api/files/{path:.*}")]
pub async fn download_file(
    _auth: BearerAuth,
    state: web::Data<AppState>,
    path: web::Path<String>,
    req: HttpRequest,
) -> actix_web::Result<HttpResponse> {
    let tracks_dir = {
        let s = state.status.read().unwrap();
        s.tracks_dir.clone()
    };

    let file_path = resolve_path(&tracks_dir, &path.into_inner());
    let file_path = match file_path {
        Some(p) => p,
        None => return Err(actix_web::error::ErrorBadRequest("invalid path")),
    };

    if !file_path.exists() {
        return Err(actix_web::error::ErrorNotFound("not found"));
    }

    // Check if currently being recorded
    {
        let s = state.status.read().unwrap();
        if let Some(ref rp) = s.recording_path {
            if file_path == std::path::Path::new(rp) {
                return Err(actix_web::error::ErrorConflict("file is being recorded"));
            }
        }
    }

    let named = NamedFile::open(file_path).map_err(actix_web::error::ErrorInternalServerError)?;
    Ok(named.into_response(&req))
}

#[delete("/api/files/{path:.*}")]
pub async fn delete_file(
    _auth: BearerAuth,
    state: web::Data<AppState>,
    path: web::Path<String>,
) -> impl Responder {
    let tracks_dir = {
        let s = state.status.read().unwrap();
        s.tracks_dir.clone()
    };

    let file_path = resolve_path(&tracks_dir, &path.into_inner());
    let file_path = match file_path {
        Some(p) => p,
        None => return HttpResponse::BadRequest().finish(),
    };

    // Check if currently being recorded
    {
        let s = state.status.read().unwrap();
        if let Some(ref rp) = s.recording_path {
            if file_path == std::path::Path::new(rp) {
                return HttpResponse::Conflict().finish();
            }
        }
    }

    if std::fs::remove_file(&file_path).is_ok() {
        // Remove peaks cache if present.
        let stem = file_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let cache = file_path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join(format!("{}.peaks.json", stem));
        let _ = std::fs::remove_file(cache);

        // Tell the app loop to remove this track so SharedStatus stays consistent.
        let _ = state.cmd_tx.send(AppCommand::RemoveTrack(file_path));

        HttpResponse::Ok().finish()
    } else {
        HttpResponse::NotFound().finish()
    }
}

// ---------------------------------------------------------------------------
// /api/transport
// ---------------------------------------------------------------------------

#[post("/api/transport/play")]
pub async fn transport_play(_auth: BearerAuth, state: web::Data<AppState>) -> impl Responder {
    let _ = state.cmd_tx.send(AppCommand::Play);
    HttpResponse::Ok().finish()
}

#[post("/api/transport/stop")]
pub async fn transport_stop(_auth: BearerAuth, state: web::Data<AppState>) -> impl Responder {
    let _ = state.cmd_tx.send(AppCommand::Stop);
    HttpResponse::Ok().finish()
}

#[post("/api/transport/prev")]
pub async fn transport_prev(_auth: BearerAuth, state: web::Data<AppState>) -> impl Responder {
    let _ = state.cmd_tx.send(AppCommand::Prev);
    HttpResponse::Ok().finish()
}

#[post("/api/transport/next")]
pub async fn transport_next(_auth: BearerAuth, state: web::Data<AppState>) -> impl Responder {
    let _ = state.cmd_tx.send(AppCommand::Next);
    HttpResponse::Ok().finish()
}

#[derive(Deserialize)]
pub struct LoadBody {
    name: String,
}

#[post("/api/transport/load")]
pub async fn transport_load(
    _auth: BearerAuth,
    state: web::Data<AppState>,
    body: web::Json<LoadBody>,
) -> impl Responder {
    let idx = {
        let s = state.status.read().unwrap();
        s.track_list.iter().position(|e| e.name == body.name)
    };
    match idx {
        Some(i) => {
            let _ = state.cmd_tx.send(AppCommand::JumpToTrack(i));
            let _ = state.cmd_tx.send(AppCommand::Play);
            HttpResponse::Ok().finish()
        }
        None => HttpResponse::NotFound().body("track not found"),
    }
}

#[derive(Deserialize)]
pub struct SeekBody {
    position_secs: f32,
}

#[post("/api/transport/seek")]
pub async fn transport_seek(
    _auth: BearerAuth,
    state: web::Data<AppState>,
    body: web::Json<SeekBody>,
) -> impl Responder {
    let _ = state.cmd_tx.send(AppCommand::Seek(body.position_secs));
    HttpResponse::Ok().finish()
}

// ---------------------------------------------------------------------------
// /api/recording
// ---------------------------------------------------------------------------

#[post("/api/recording/start")]
pub async fn recording_start(_auth: BearerAuth, state: web::Data<AppState>) -> impl Responder {
    let _ = state.cmd_tx.send(AppCommand::StartRecording);
    HttpResponse::Ok().finish()
}

#[post("/api/recording/stop")]
pub async fn recording_stop(_auth: BearerAuth, state: web::Data<AppState>) -> impl Responder {
    let _ = state.cmd_tx.send(AppCommand::StopRecording);
    HttpResponse::Ok().finish()
}

// ---------------------------------------------------------------------------
// Peaks precomputation
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct PeaksStatusResponse {
    total: u32,
    done: u32,
    failed: u32,
    pending: u32,
}

#[get("/api/peaks/status")]
pub async fn peaks_status(_auth: BearerAuth, state: web::Data<AppState>) -> impl Responder {
    let p = &state.peaks_progress;
    let total = p.total.load(Ordering::Relaxed);
    let done = p.done.load(Ordering::Relaxed);
    let failed = p.failed.load(Ordering::Relaxed);
    HttpResponse::Ok().json(PeaksStatusResponse {
        total,
        done,
        failed,
        pending: total.saturating_sub(done + failed),
    })
}

#[post("/api/peaks/precompute")]
pub async fn peaks_precompute(_auth: BearerAuth, state: web::Data<AppState>) -> impl Responder {
    let tracks_dir = state.status.read().unwrap().tracks_dir.clone();
    let semaphore = state.peaks_semaphore.clone();
    let progress = state.peaks_progress.clone();
    spawn_precompute(tracks_dir, semaphore, progress);
    HttpResponse::Accepted().body("queued")
}

// ---------------------------------------------------------------------------
// /api/config
// ---------------------------------------------------------------------------

#[get("/api/config")]
pub async fn get_config(_auth: BearerAuth, state: web::Data<AppState>) -> impl Responder {
    let mut cfg = {
        let c = state.config.read().unwrap();
        c.clone()
    };
    // Redact password fields
    cfg.web.password_hash = String::new();
    cfg.network.ap.password = String::new();
    HttpResponse::Ok().json(cfg)
}

#[patch("/api/config")]
pub async fn patch_config(
    _auth: BearerAuth,
    state: web::Data<AppState>,
    body: web::Json<Value>,
) -> impl Responder {
    let mut patch = body.into_inner();

    // Strip sensitive fields from patch
    if let Value::Object(ref mut map) = patch {
        if let Some(Value::Object(ref mut web_map)) = map.get_mut("web") {
            web_map.remove("password_hash");
        }
        if let Some(Value::Object(ref mut net_map)) = map.get_mut("network") {
            if let Some(Value::Object(ref mut ap_map)) = net_map.get_mut("ap") {
                ap_map.remove("password");
            }
        }
    }

    let current = {
        let c = state.config.read().unwrap();
        serde_json::to_value(c.clone()).unwrap_or(Value::Null)
    };

    let merged = json_merge(current, patch);

    let new_cfg: Config = match serde_json::from_value(merged) {
        Ok(c) => c,
        Err(e) => return HttpResponse::BadRequest().body(format!("invalid config: {}", e)),
    };

    {
        let mut cfg = state.config.write().unwrap();
        *cfg = new_cfg.clone();
    }

    if let Err(e) = new_cfg.save() {
        return HttpResponse::InternalServerError().body(format!("save error: {}", e));
    }

    HttpResponse::Ok().finish()
}

fn json_merge(base: Value, patch: Value) -> Value {
    match (base, patch) {
        (Value::Object(mut a), Value::Object(b)) => {
            for (k, v) in b {
                let merged = json_merge(a.remove(&k).unwrap_or(Value::Null), v);
                a.insert(k, merged);
            }
            Value::Object(a)
        }
        (_, patch) => patch,
    }
}

// ---------------------------------------------------------------------------
// /api/network
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct WifiNetwork {
    ssid: String,
    signal: i32,
}

#[get("/api/network/scan")]
pub async fn network_scan(_auth: BearerAuth) -> impl Responder {
    let output = Command::new("nmcli")
        .arg("-t")
        .arg("-f")
        .arg("SSID,SIGNAL")
        .arg("dev")
        .arg("wifi")
        .arg("list")
        .output();

    let output = match output {
        Ok(o) => o,
        Err(_) => {
            let empty: Vec<WifiNetwork> = vec![];
            return HttpResponse::Ok().json(empty);
        }
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let mut networks: Vec<WifiNetwork> = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, ':');
            let ssid = parts.next()?.trim().to_owned();
            let signal: i32 = parts.next()?.trim().parse().ok()?;
            if ssid.is_empty() {
                return None;
            }
            Some(WifiNetwork { ssid, signal })
        })
        .collect();

    // Deduplicate by ssid, keeping highest signal
    networks.sort_by(|a, b| b.signal.cmp(&a.signal));
    networks.dedup_by(|a, b| {
        if a.ssid == b.ssid {
            // b is kept (higher signal), a is removed
            true
        } else {
            false
        }
    });

    HttpResponse::Ok().json(networks)
}

#[derive(Deserialize)]
pub struct ConnectBody {
    ssid: String,
    psk: String,
}

#[post("/api/network/connect")]
pub async fn network_connect(_auth: BearerAuth, body: web::Json<ConnectBody>) -> impl Responder {
    let status = Command::new("nmcli")
        .arg("dev")
        .arg("wifi")
        .arg("connect")
        .arg(&body.ssid)
        .arg("password")
        .arg(&body.psk)
        .status();

    match status {
        Ok(s) if s.success() => HttpResponse::Ok().finish(),
        _ => HttpResponse::InternalServerError().body("connect failed"),
    }
}

#[derive(Deserialize)]
pub struct ApPasswordBody {
    password: String,
}

#[post("/api/network/ap/password")]
pub async fn network_ap_password(
    _auth: BearerAuth,
    state: web::Data<AppState>,
    body: web::Json<ApPasswordBody>,
) -> impl Responder {
    {
        let mut cfg = state.config.write().unwrap();
        cfg.network.ap.password = body.password.clone();
    }
    let cfg = state.config.read().unwrap().clone();
    if let Err(e) = cfg.save() {
        return HttpResponse::InternalServerError().body(format!("save error: {}", e));
    }
    HttpResponse::Ok().finish()
}

// ---------------------------------------------------------------------------
// /api/devices
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct AlsaDevice {
    name: String,
    id: String,
}

fn parse_alsa_devices(output: &str) -> Vec<AlsaDevice> {
    let mut devices = Vec::new();
    for line in output.lines() {
        // Lines like: card 0: DAC8x [snd_rpi_hifiberry_dac8x], device 0: ...
        if !line.starts_with("card ") {
            continue;
        }
        // Parse card number and name
        let card_num: u32 = line
            .trim_start_matches("card ")
            .split(':')
            .next()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);

        // Find device number
        let dev_num: u32 = if let Some(dev_part) = line.split("device ").nth(1) {
            dev_part
                .split(':')
                .next()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0)
        } else {
            0
        };

        // Extract name from brackets [...]
        let name = line
            .split('[')
            .nth(1)
            .and_then(|s| s.split(']').next())
            .unwrap_or("")
            .to_string();

        if name.is_empty() {
            continue;
        }

        devices.push(AlsaDevice {
            name,
            id: format!("hw:{},{}", card_num, dev_num),
        });
    }
    devices
}

#[get("/api/devices/playback")]
pub async fn devices_playback(_auth: BearerAuth) -> impl Responder {
    let output = Command::new("aplay").arg("-l").output();
    let text = output
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    HttpResponse::Ok().json(parse_alsa_devices(&text))
}

#[get("/api/devices/capture")]
pub async fn devices_capture(_auth: BearerAuth) -> impl Responder {
    let output = Command::new("arecord").arg("-l").output();
    let text = output
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    HttpResponse::Ok().json(parse_alsa_devices(&text))
}

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

#[post("/api/system/restart")]
pub async fn system_restart(_auth: BearerAuth, state: web::Data<AppState>) -> impl Responder {
    // Signal the app to stop playback/recording so ALSA devices are released
    // before exit. The app loop processes these within one tick (~250ms).
    let _ = state.cmd_tx.send(AppCommand::Stop);
    let _ = state.cmd_tx.send(AppCommand::StopRecording);

    // Spawn a thread that re-execs the current binary after a delay so
    // the HTTP response can be delivered and stop commands can be processed.
    let tty = state.controlling_tty.clone();
    std::thread::spawn(move || {
        // 600ms: enough for the HTTP response (~100ms) and two app ticks (~500ms)
        // to process the Stop/StopRecording commands before we exit.
        std::thread::sleep(std::time::Duration::from_millis(600));
        let exe = std::env::current_exe().expect("current_exe");
        let args: Vec<String> = std::env::args().skip(1).collect();
        // If we know the original controlling terminal, redirect the new process
        // to it so the TUI/display comes back up on the same screen.
        let cmd = if let Some(ref tty_path) = tty {
            format!(
                "setsid {} {} <{} >{} 2>{} &",
                exe.display(),
                args.join(" "),
                tty_path,
                tty_path,
                tty_path,
            )
        } else {
            format!(
                "nohup {} {} </dev/null >/tmp/octotrack_restart.log 2>&1 &",
                exe.display(),
                args.join(" ")
            )
        };
        eprintln!("restart: {}", cmd);
        let _ = std::process::Command::new("sh").arg("-c").arg(&cmd).spawn();
        std::process::exit(0);
    });
    HttpResponse::Ok().body("restarting")
}

#[post("/api/system/reboot")]
pub async fn system_reboot(_auth: BearerAuth) -> impl Responder {
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let _ = std::process::Command::new("sudo").arg("reboot").spawn();
    });
    HttpResponse::Ok().body("rebooting")
}

#[post("/api/system/shutdown")]
pub async fn system_shutdown(_auth: BearerAuth) -> impl Responder {
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let _ = std::process::Command::new("sudo")
            .arg("shutdown")
            .arg("-h")
            .arg("now")
            .spawn();
    });
    HttpResponse::Ok().body("shutting down")
}

#[get("/api/system/info")]
pub async fn system_info(_auth: BearerAuth, state: web::Data<AppState>) -> impl Responder {
    use std::fs;

    let hostname = fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let uptime_secs = fs::read_to_string("/proc/uptime")
        .map(|s| {
            s.split_whitespace()
                .next()
                .unwrap_or("0")
                .parse::<f64>()
                .unwrap_or(0.0)
        })
        .unwrap_or(0.0);

    let load_avg = fs::read_to_string("/proc/loadavg")
        .map(|s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
        .unwrap_or_default();

    // Parse /proc/meminfo for MemTotal and MemAvailable (in kB)
    let (mem_total_kb, mem_available_kb) = fs::read_to_string("/proc/meminfo")
        .map(|s| {
            let mut total = 0u64;
            let mut avail = 0u64;
            for line in s.lines() {
                if line.starts_with("MemTotal:") {
                    total = line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                } else if line.starts_with("MemAvailable:") {
                    avail = line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                }
            }
            (total, avail)
        })
        .unwrap_or((0, 0));

    // Disk usage for tracks dir
    let tracks_dir = state.status.read().unwrap().tracks_dir.clone();
    let (disk_used_bytes, disk_total_bytes) = {
        use std::ffi::CString;
        let path = CString::new(tracks_dir.as_bytes()).unwrap_or_default();
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } == 0 {
            let total = stat.f_blocks * stat.f_frsize;
            let free = stat.f_bavail * stat.f_frsize;
            (total - free, total)
        } else {
            (0, 0)
        }
    };

    HttpResponse::Ok().json(serde_json::json!({
        "hostname": hostname,
        "uptime_secs": uptime_secs,
        "load_avg": load_avg,
        "mem_total_mb": mem_total_kb / 1024,
        "mem_available_mb": mem_available_kb / 1024,
        "disk_used_bytes": disk_used_bytes,
        "disk_total_bytes": disk_total_bytes,
    }))
}

// ---------------------------------------------------------------------------
// Static file / page helpers
// ---------------------------------------------------------------------------

/// Derive the `.peaks.json` sidecar path for an audio file.
pub fn peaks_cache_path(file_path: &Path) -> PathBuf {
    let stem = file_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut p = file_path.to_path_buf();
    p.set_file_name(format!("{}.peaks.json", stem));
    p
}

/// Read the cached per-channel peaks for `file_path` if the cache exists and
/// mtime matches.  Returns `None` if missing, stale, or in the old mono format.
pub fn read_peaks_cache(file_path: &PathBuf) -> Option<Vec<Vec<f32>>> {
    let cache_path = peaks_cache_path(file_path);
    let file_mtime = std::fs::metadata(file_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    let cached = std::fs::read_to_string(&cache_path).ok()?;
    let v: Value = serde_json::from_str(&cached).ok()?;
    if v["mtime"].as_u64() != file_mtime {
        return None;
    }
    // "channels" key is the new per-channel format; old "peaks" key is discarded.
    serde_json::from_value(v["channels"].clone()).ok()
}

/// Run ffmpeg to compute per-channel peaks for `file_path`, write the cache,
/// and return the normalised peaks as one array per channel.
/// Blocking — call from `spawn_blocking`.
pub fn compute_and_cache_peaks(file_path: &PathBuf) -> Option<Vec<Vec<f32>>> {
    // Determine channel count via ffprobe so we can de-interleave the raw PCM.
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=channels",
            "-of",
            "csv=p=0",
        ])
        .arg(file_path)
        .output()
        .ok()?;
    let n_channels: usize = String::from_utf8_lossy(&probe.stdout)
        .trim()
        .parse()
        .unwrap_or(1)
        .max(1);

    // Decode to raw interleaved s16le at 441 Hz (1/100th of 44100).
    let out = Command::new("ffmpeg")
        .arg("-i")
        .arg(file_path)
        .arg("-filter:a")
        .arg("aresample=441")
        .arg("-map")
        .arg("0:a")
        .arg("-c:a")
        .arg("pcm_s16le")
        .arg("-f")
        .arg("data")
        .arg("pipe:1")
        .output()
        .ok()?;

    let interleaved: Vec<i16> = out
        .stdout
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();

    if interleaved.is_empty() {
        return None;
    }

    // De-interleave: channel[c] = interleaved[c], interleaved[c + n_channels], ...
    let frames = interleaved.len() / n_channels;
    let bucket_size = (frames / 1000).max(1);

    let mut channels: Vec<Vec<f32>> = (0..n_channels)
        .map(|c| {
            // Extract this channel's samples.
            let ch_samples: Vec<i16> = (0..frames)
                .map(|f| interleaved[f * n_channels + c])
                .collect();

            // Compute RMS in buckets of `bucket_size` frames, scaled to 0.0–1.0
            // relative to i16::MAX (absolute scale — no normalization so that
            // silent files appear flat rather than being boosted to full height).
            ch_samples
                .chunks(bucket_size)
                .map(|chunk| {
                    let rms = (chunk.iter().map(|&s| (s as f64).powi(2)).sum::<f64>()
                        / chunk.len() as f64)
                        .sqrt();
                    (rms / i16::MAX as f64) as f32
                })
                .collect()
        })
        .collect();

    // Trim to exactly 1000 buckets so all channels are the same length.
    for ch in &mut channels {
        ch.truncate(1000);
    }

    let file_mtime = std::fs::metadata(file_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());

    let cache = serde_json::json!({ "mtime": file_mtime, "channels": &channels });
    let _ = std::fs::write(peaks_cache_path(file_path), cache.to_string());

    Some(channels)
}

/// Spawn a background tokio task that pre-computes peaks for every audio file
/// in `tracks_dir` that doesn't already have a valid cache.
/// Uses `semaphore` to limit concurrent ffmpeg jobs and updates `progress`.
pub fn spawn_precompute(
    tracks_dir: String,
    semaphore: Arc<Semaphore>,
    progress: Arc<PeaksProgress>,
) {
    tokio::spawn(async move {
        let audio_exts = ["wav", "mp3", "flac", "ogg", "m4a"];
        let entries = match std::fs::read_dir(&tracks_dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        // Collect files that need processing.
        let pending: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .filter(|p| {
                p.extension()
                    .map(|e| audio_exts.contains(&e.to_string_lossy().to_lowercase().as_str()))
                    .unwrap_or(false)
            })
            .filter(|p| read_peaks_cache(p).is_none())
            .collect();

        progress
            .total
            .fetch_add(pending.len() as u32, Ordering::Relaxed);

        for path in pending {
            let permit = semaphore.clone().acquire_owned().await.ok();
            let path_clone = path.clone();
            let progress_clone = progress.clone();
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                if compute_and_cache_peaks(&path_clone).is_some() {
                    progress_clone.done.fetch_add(1, Ordering::Relaxed);
                } else {
                    progress_clone.failed.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });
}

/// Resolve a URL path segment against the tracks directory, preventing traversal.
fn resolve_path(tracks_dir: &str, path: &str) -> Option<PathBuf> {
    let base = std::fs::canonicalize(tracks_dir).ok()?;
    // Strip leading slashes/dots
    let rel: PathBuf = path
        .split('/')
        .filter(|c| !c.is_empty() && *c != "..")
        .collect();
    let full = base.join(rel);
    // Security check: must be within tracks_dir
    let canon = std::fs::canonicalize(&full).unwrap_or(full.clone());
    if canon.starts_with(&base) {
        Some(full)
    } else {
        None
    }
}
