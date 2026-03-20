use clap::Parser;
use octotrack::app::{App, AppCommand, AppResult, AutoMode};
use octotrack::config::Config;
use octotrack::event::{Event, EventHandler};
use octotrack::handler::handle_key_events;
use octotrack::schedule;
use octotrack::setup;
use octotrack::tui::Tui;
use octotrack::web::sse::OctoeventEvent;
use octotrack::web::{self, SharedStatus, SseBroadcaster, TrackEntry};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
enum RunMode {
    Headless,
    Tui,
    Gui,
}

#[derive(Parser, Debug)]
#[command(
    name = "octotrack",
    about = "Multi-channel audio playback and recording"
)]
struct Cli {
    /// Run as a headless background daemon
    #[arg(long)]
    headless: bool,

    /// Force GUI mode
    #[arg(long)]
    gui: bool,

    /// Run interactive password setup then exit
    #[arg(long)]
    set_password: bool,

    /// Clear stored passwords and force first-run prompt
    #[arg(long)]
    reset: bool,

    /// Pre-compute waveform peaks for all tracks and exit
    #[arg(long)]
    precompute_peaks: bool,
}

fn detect_mode(cli: &Cli) -> RunMode {
    match (cli.headless, cli.gui) {
        (true, _) => RunMode::Headless,
        (_, true) => RunMode::Gui,
        _ if has_display() => RunMode::Gui,
        _ if !has_tty() => RunMode::Headless,
        _ => RunMode::Tui,
    }
}

fn has_display() -> bool {
    std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok()
}

fn has_tty() -> bool {
    // If stdin is not a terminal (e.g. redirected to /dev/null), there's no TTY.
    unsafe { libc::isatty(libc::STDIN_FILENO) != 0 }
}

/// Returns the path of the controlling terminal (e.g. `/dev/tty1`), if any.
fn controlling_tty() -> Option<String> {
    unsafe {
        if libc::isatty(libc::STDIN_FILENO) == 0 {
            return None;
        }
        let mut buf = [0u8; 256];
        if libc::ttyname_r(
            libc::STDIN_FILENO,
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
        ) == 0
        {
            std::ffi::CStr::from_ptr(buf.as_ptr() as *const libc::c_char)
                .to_string_lossy()
                .into_owned()
                .into()
        } else {
            None
        }
    }
}

/// Finds the tracks directory, checking USB storage first, then falling back to local directory
fn find_tracks_directory() -> String {
    // Check common USB/removable storage mount points
    #[cfg(target_os = "linux")]
    let usb_mount_points = vec![PathBuf::from("/media"), PathBuf::from("/mnt")];

    #[cfg(target_os = "macos")]
    let usb_mount_points = vec![PathBuf::from("/Volumes")];

    #[cfg(target_os = "windows")]
    let usb_mount_points: Vec<PathBuf> = ('D'..='Z')
        .map(|c| PathBuf::from(format!("{}:\\", c)))
        .filter(|p| p.exists())
        .collect();

    for mount_root in usb_mount_points {
        if let Ok(entries) = fs::read_dir(&mount_root) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();

                // Skip if not a directory
                if !path.is_dir() {
                    continue;
                }

                // Check if there's a 'tracks' subdirectory
                let tracks_path = path.join("tracks");
                if tracks_path.exists() && tracks_path.is_dir() {
                    // Check if the tracks directory has audio files
                    if has_audio_files(&tracks_path) {
                        return tracks_path.to_string_lossy().to_string();
                    }
                }
            }
        }
    }

    // Fall back to local tracks directory
    "tracks".to_string()
}

/// Checks if a directory contains any audio files (mp3, wav, flac)
fn has_audio_files(dir: &Path) -> bool {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();

            // Check for audio files directly in the directory
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext.eq_ignore_ascii_case("mp3")
                        || ext.eq_ignore_ascii_case("wav")
                        || ext.eq_ignore_ascii_case("flac")
                    {
                        return true;
                    }
                }
            }

            // Check subdirectories for audio files
            if path.is_dir() {
                if let Ok(sub_entries) = fs::read_dir(&path) {
                    for sub_entry in sub_entries.filter_map(|e| e.ok()) {
                        let sub_path = sub_entry.path();
                        if sub_path.is_file() {
                            if let Some(ext) = sub_path.extension() {
                                if ext.eq_ignore_ascii_case("mp3")
                                    || ext.eq_ignore_ascii_case("wav")
                                    || ext.eq_ignore_ascii_case("flac")
                                {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    false
}

/// Read channel count and duration for any audio file.
///
/// WAV and RF64 files are parsed directly from the file header (fast, no
/// subprocess). All other formats (FLAC, MP3, etc.) fall back to ffprobe.
fn audio_info(path: &std::path::Path) -> (Option<u8>, Option<f32>) {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if ext == "wav" {
        let result = wav_info(path);
        if result.0.is_some() || result.1.is_some() {
            return result;
        }
    }
    // ffprobe fallback for FLAC, MP3, OGG, M4A, and broken/unrecognised WAV.
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=channels,duration",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output();
    if let Ok(out) = out {
        let s = String::from_utf8_lossy(&out.stdout);
        // Output format: channels,duration  (duration may be "N/A")
        let mut parts = s.trim().splitn(2, ',');
        let channels = parts
            .next()
            .and_then(|c| c.trim().parse::<u8>().ok())
            .filter(|&c| c > 0);
        let duration = parts
            .next()
            .and_then(|d| d.trim().parse::<f32>().ok())
            .filter(|&d| d > 0.0);
        return (channels, duration);
    }
    (None, None)
}

/// Read channel count and duration from a WAV or RF64 file without spawning a
/// process (WAV/RF64 only).
fn wav_info(path: &std::path::Path) -> (Option<u8>, Option<f32>) {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return (None, None),
    };

    // Read and validate the 12-byte RIFF/RF64 file header.
    let mut file_hdr = [0u8; 12];
    if f.read_exact(&mut file_hdr).is_err() {
        return (None, None);
    }
    let is_rf64 = &file_hdr[0..4] == b"RF64";
    if !is_rf64 && &file_hdr[0..4] != b"RIFF" {
        return (None, None);
    }
    if &file_hdr[8..12] != b"WAVE" {
        return (None, None);
    }

    let mut channels: Option<u16> = None;
    let mut sample_rate: Option<u32> = None;
    let mut bits_per_sample: Option<u16> = None;
    let mut data_bytes: Option<u64> = None;
    // ds64 data-size field for RF64 files.
    let mut ds64_data_bytes: Option<u64> = None;

    // Scan chunks starting at offset 12.
    let mut chunk_id = [0u8; 4];
    let mut chunk_size_buf = [0u8; 4];
    for _ in 0..32 {
        if f.read_exact(&mut chunk_id).is_err() {
            break;
        }
        if f.read_exact(&mut chunk_size_buf).is_err() {
            break;
        }
        let chunk_size = u32::from_le_bytes(chunk_size_buf) as u64;

        match &chunk_id {
            b"ds64" => {
                // RF64 extension: riff_size(8) + data_size(8) + sample_count(8) ...
                let mut ds64 = [0u8; 24];
                if f.read_exact(&mut ds64).is_ok() {
                    ds64_data_bytes =
                        Some(u64::from_le_bytes(ds64[8..16].try_into().unwrap_or([0; 8])));
                }
                // Skip remaining ds64 bytes.
                let remaining = chunk_size.saturating_sub(24);
                let _ = f.seek(SeekFrom::Current(remaining as i64));
            }
            b"fmt " => {
                let mut fmt = vec![0u8; chunk_size.min(40) as usize];
                if f.read_exact(&mut fmt).is_ok() && fmt.len() >= 16 {
                    channels = Some(u16::from_le_bytes([fmt[2], fmt[3]]));
                    sample_rate = Some(u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]));
                    bits_per_sample = Some(u16::from_le_bytes([fmt[14], fmt[15]]));
                }
                let remaining = chunk_size.saturating_sub(fmt.len() as u64);
                let _ = f.seek(SeekFrom::Current(remaining as i64));
            }
            b"data" => {
                // 0xFFFFFFFF means the real size is in ds64.
                data_bytes = if chunk_size == 0xFFFF_FFFF {
                    ds64_data_bytes
                } else {
                    Some(chunk_size)
                };
                break; // data is always last; no need to continue
            }
            _ => {
                let _ = f.seek(SeekFrom::Current(chunk_size as i64));
            }
        }
    }

    let ch = channels.filter(|&c| c > 0);
    let duration = match (ch, sample_rate, bits_per_sample, data_bytes) {
        (Some(c), Some(sr), Some(bps), Some(db)) if sr > 0 && bps > 0 => {
            let bytes_per_frame = c as u64 * (bps as u64 / 8);
            if bytes_per_frame > 0 {
                Some(db as f32 / bytes_per_frame as f32 / sr as f32)
            } else {
                None
            }
        }
        _ => None,
    };

    (ch.map(|c| c as u8), duration)
}

/// Cached track metadata to avoid re-scanning every tick.
struct TrackListCache {
    /// The track paths when the cache was last built.
    paths: Vec<PathBuf>,
    /// The computed metadata for those paths.
    entries: Vec<TrackEntry>,
}

impl TrackListCache {
    fn new() -> Self {
        Self {
            paths: Vec::new(),
            entries: Vec::new(),
        }
    }

    /// Returns cached entries if the track list hasn't changed, otherwise
    /// recomputes and caches the result.
    fn get(&mut self, track_list: &[PathBuf]) -> &[TrackEntry] {
        if self.paths != track_list {
            self.paths = track_list.to_vec();
            self.entries = track_list
                .iter()
                .map(|p| {
                    let meta = fs::metadata(p).ok();
                    let size_bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                    let modified_secs = meta
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs());
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let path = p.to_string_lossy().into_owned();
                    let (channels, duration_secs) = audio_info(p);
                    TrackEntry {
                        name,
                        path,
                        size_bytes,
                        duration_secs,
                        channels,
                        modified_secs,
                    }
                })
                .collect();
        }
        &self.entries
    }
}

/// Build a `SharedStatus` snapshot from the current App state.
fn build_shared_status(app: &App, cache: &mut TrackListCache) -> SharedStatus {
    let track_list = cache.get(&app.track_list).to_vec();

    SharedStatus {
        playing: app.is_playing,
        recording: app.is_recording,
        monitoring: app.is_monitoring,
        current_track: app.track_list.get(app.current_track_index).map(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        }),
        position_secs: app.current_position,
        duration_secs: app.track_duration,
        volume: app.volume,
        input_levels: app.channel_levels.clone(),
        output_levels: app.channel_levels.clone(),
        recording_path: app
            .recording_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        recording_size_bytes: if app.is_recording {
            app.recording_file_bytes()
        } else {
            0
        },
        recording_duration_secs: app.recording_elapsed(),
        files_written: 0, // TODO: wire up files_written counter
        tracks_dir: app.tracks_dir.clone(),
        track_list,
    }
}

/// Update `SharedStatus` and push SSE events on each tick.
fn update_shared(app: &App, status: &Arc<RwLock<SharedStatus>>, broadcaster: &SseBroadcaster, cache: &mut TrackListCache) {
    let new_status = build_shared_status(app, cache);

    {
        let mut s = status.write().unwrap();
        *s = new_status.clone();
    }

    // Push level meters
    broadcaster.send(&OctoeventEvent::Levels {
        input: new_status.input_levels.clone(),
        output: new_status.output_levels.clone(),
    });

    // Push device status
    broadcaster.send(&OctoeventEvent::DeviceStatus {
        recording: new_status.recording,
        monitoring: new_status.monitoring,
        playing: new_status.playing,
        current_track: new_status.current_track.clone(),
    });

    // Push playback position if playing
    if new_status.playing {
        if let Some(pos) = new_status.position_secs {
            broadcaster.send(&OctoeventEvent::PlaybackPosition { position_secs: pos });
        }
    }

    // Push recording progress if recording
    if new_status.recording {
        if let Some(ref path) = new_status.recording_path {
            broadcaster.send(&OctoeventEvent::RecordingProgress {
                path: path.clone(),
                size_bytes: new_status.recording_size_bytes,
                duration_secs: new_status.recording_duration_secs,
                files_written: new_status.files_written,
            });
        }
    }
}

/// Drain any pending AppCommands and dispatch them to the App.
fn drain_commands(app: &mut App, cmd_rx: &std::sync::mpsc::Receiver<AppCommand>) {
    while let Ok(cmd) = cmd_rx.try_recv() {
        match cmd {
            AppCommand::Play => app.play(),
            AppCommand::Stop => {
                let _ = app.stop();
            }
            AppCommand::Prev => app.decrement_track(),
            AppCommand::Next => app.increment_track(),
            AppCommand::Seek(_pos) => {
                // TODO: implement seek via mplayer slave command
            }
            AppCommand::StartRecording => {
                let _ = app.start_recording();
            }
            AppCommand::StopRecording => {
                let _ = app.stop_recording();
            }
            AppCommand::JumpToTrack(idx) => {
                app.current_track_index = idx;
                if !app.track_list.is_empty() && idx < app.track_list.len() {
                    app.get_metadata();
                }
            }
            AppCommand::RemoveTrack(path) => {
                app.track_list.retain(|p| p != &path);
                // Keep current_track_index in bounds.
                if app.current_track_index >= app.track_list.len() {
                    app.current_track_index = app.track_list.len().saturating_sub(1);
                }
            }
        }
    }
}

fn run_tui(
    mut app: App,
    status: Arc<RwLock<SharedStatus>>,
    broadcaster: SseBroadcaster,
    cmd_rx: std::sync::mpsc::Receiver<AppCommand>,
) -> AppResult<()> {
    let backend = CrosstermBackend::new(io::stderr());
    let terminal = Terminal::new(backend)?;
    let events = EventHandler::new(250);
    let mut tui = Tui::new(terminal, events);
    tui.init()?;
    let mut track_cache = TrackListCache::new();
    let mut needs_redraw = true;

    // Start the main loop.
    while app.running {
        // Drain web commands
        drain_commands(&mut app, &cmd_rx);

        // Render the user interface only when state changed.
        if needs_redraw {
            tui.draw(&mut app)?;
            needs_redraw = false;
        }

        // Handle events (blocks until next event).
        match tui.events.next()? {
            Event::Tick => {
                app.update_playback_info();
                app.check_playback_status();
                app.tick();
                update_shared(&app, &status, &broadcaster, &mut track_cache);
                needs_redraw = true;
            }
            Event::Key(key_event) => {
                handle_key_events(key_event, &mut app)?;
                needs_redraw = true;
            }
            Event::Mouse(_) => {}
            Event::Resize(_, _) => {
                needs_redraw = true;
            }
        }
    }

    // Exit the user interface.
    let _ = app.audio_player.stop_recording();
    app.audio_player.stop()?;
    tui.exit()?;
    Ok(())
}

fn run_headless(
    app: Arc<Mutex<App>>,
    status: Arc<RwLock<SharedStatus>>,
    broadcaster: SseBroadcaster,
    cmd_rx: std::sync::mpsc::Receiver<AppCommand>,
) {
    let mut track_cache = TrackListCache::new();
    loop {
        {
            let mut app = app.lock().unwrap();
            drain_commands(&mut app, &cmd_rx);
            app.update_playback_info();
            app.tick();
            update_shared(&app, &status, &broadcaster, &mut track_cache);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Returns true if wlan0 is currently connected to a network in managed (client) mode.
fn wlan0_is_connected(nmcli: &str) -> bool {
    use std::process::Command;
    // nmcli -t -f DEVICE,TYPE,STATE dev prints lines like: wlan0:wifi:connected
    let out = Command::new(nmcli)
        .args(["-t", "-f", "DEVICE,TYPE,STATE", "dev"])
        .output();
    if let Ok(out) = out {
        let s = String::from_utf8_lossy(&out.stdout);
        for line in s.lines() {
            let parts: Vec<&str> = line.splitn(3, ':').collect();
            if parts.len() == 3 && parts[0] == "wlan0" && parts[2] == "connected" {
                return true;
            }
        }
    }
    false
}

/// Ensure the `octotrack-ap` NetworkManager hotspot connection exists and is up.
///
/// Only starts the AP when wlan0 is not already connected to a LAN — failover mode.
/// Uses `ipv4.method shared` so NM handles DHCP and routing automatically.
fn start_access_point(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    use std::process::Command;

    let ap = &config.network.ap;
    let nmcli = &config.tools.nmcli;
    let con_name = "octotrack-ap";

    // If wlan0 is already connected to a network, skip AP — can't use the same
    // radio for both client and AP on different channels.
    if wlan0_is_connected(nmcli) {
        eprintln!("Access point skipped: wlan0 is connected to a network.");
        return Ok(());
    }

    // Check if the connection already exists.
    let exists = Command::new(nmcli)
        .args(["con", "show", con_name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !exists {
        // Create a persistent WiFi hotspot connection.
        let status = Command::new(nmcli)
            .args([
                "con",
                "add",
                "type",
                "wifi",
                "ifname",
                "wlan0",
                "con-name",
                con_name,
                "autoconnect",
                "no", // we bring it up explicitly; let NM manage LAN separately
                "ssid",
                &ap.ssid,
                "mode",
                "ap",
                "band",
                "bg",
                "channel",
                &ap.channel.to_string(),
                "ipv4.method",
                "shared",
                "ipv4.addresses",
                &format!("{}/24", ap.address),
                "wifi-sec.key-mgmt",
                "wpa-psk",
                "wifi-sec.psk",
                &ap.password,
            ])
            .status()?;
        if !status.success() {
            return Err("nmcli con add failed".into());
        }
    } else {
        // Update the password in case it changed.
        let _ = Command::new(nmcli)
            .args([
                "con",
                "modify",
                con_name,
                "wifi-sec.psk",
                &ap.password,
                "ssid",
                &ap.ssid,
            ])
            .status();
    }

    // Bring the connection up.
    let status = Command::new(nmcli).args(["con", "up", con_name]).status()?;
    if !status.success() {
        return Err("nmcli con up failed".into());
    }

    eprintln!("Access point '{}' started at {}", ap.ssid, ap.address);
    Ok(())
}

fn main() -> AppResult<()> {
    let cli = Cli::parse();
    let mode = detect_mode(&cli);

    // --- Pre-compute peaks -----------------------------------------------
    if cli.precompute_peaks {
        let tracks_dir = find_tracks_directory();
        let audio_exts = ["wav", "mp3", "flac", "ogg", "m4a"];
        let entries = fs::read_dir(&tracks_dir).unwrap_or_else(|e| {
            eprintln!(
                "error: cannot read tracks directory '{}': {}",
                tracks_dir, e
            );
            std::process::exit(1);
        });
        let mut count = 0;
        let mut skipped = 0;
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if !audio_exts.contains(&ext.as_str()) {
                continue;
            }
            if octotrack::web::routes::read_peaks_cache(&path).is_some() {
                skipped += 1;
                continue;
            }
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            eprint!("  computing peaks: {} ...", name);
            if octotrack::web::routes::compute_and_cache_peaks(&path).is_some() {
                eprintln!(" done");
                count += 1;
            } else {
                eprintln!(" failed");
            }
        }
        eprintln!("computed: {}  skipped (cached): {}", count, skipped);
        return Ok(());
    }

    // --- Factory reset ---------------------------------------------------
    if cli.reset {
        setup::factory_reset(Config::load())?;
        return Ok(());
    }

    // --- Explicit set-password -------------------------------------------
    if cli.set_password {
        setup::run_setup(Config::load())?;
        return Ok(());
    }

    // --- First-run detection ---------------------------------------------
    let config = {
        let c = Config::load();
        if setup::needs_setup(&c) {
            setup::run_setup(c)?
        } else {
            c
        }
    };

    let mut app = App::new_with_config(config.clone());

    // --- Access point --------------------------------------------------------
    if config.network.ap.enabled && !config.network.ap.password.is_empty() {
        if let Err(e) = start_access_point(&config) {
            eprintln!("warning: could not start access point: {}", e);
        }
    }

    // Load and start the cron scheduler if any schedules are configured.
    let schedules = schedule::load_schedules();
    if !schedules.is_empty() {
        let (tx, rx) = std::sync::mpsc::channel();
        schedule::run_scheduler(schedules, tx);
        app.schedule_rx = Some(rx);
    }

    // Find tracks directory (USB first, then local fallback)
    let tracks_dir = find_tracks_directory();
    app.load_tracks(&tracks_dir).unwrap();

    // Get initial track metadata
    if !app.track_list.is_empty() {
        app.get_metadata();
    }

    // Auto action on startup
    match app.auto_mode {
        AutoMode::Rec => {
            let _ = app.start_recording();
        }
        AutoMode::Play => {
            if !app.track_list.is_empty() {
                app.play();
            }
        }
        AutoMode::Off => {}
    }

    // Set up shared state for the web server
    let mut initial_cache = TrackListCache::new();
    let status: Arc<RwLock<SharedStatus>> = Arc::new(RwLock::new(build_shared_status(&app, &mut initial_cache)));
    let broadcaster = SseBroadcaster::new();
    let config = Arc::new(RwLock::new(app.config.clone()));

    // Command channel: web → app
    let (cmd_tx, cmd_rx) = std::sync::mpsc::sync_channel::<AppCommand>(64);

    // Spawn the web server
    let tty = controlling_tty();
    let _web_handle = web::spawn(
        config.clone(),
        status.clone(),
        cmd_tx,
        broadcaster.clone(),
        tty,
    );

    match mode {
        RunMode::Tui => run_tui(app, status, broadcaster, cmd_rx),
        RunMode::Headless => {
            let app = Arc::new(Mutex::new(app));
            run_headless(app, status, broadcaster, cmd_rx);
            Ok(())
        }
        RunMode::Gui => {
            // Phase 2: egui implementation — fall back to TUI for now
            run_tui(app, status, broadcaster, cmd_rx)
        }
    }
}
