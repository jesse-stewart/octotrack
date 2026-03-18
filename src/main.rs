use clap::Parser;
use octotrack::app::{App, AppResult, AutoMode};
use octotrack::event::{Event, EventHandler};
use octotrack::handler::handle_key_events;
use octotrack::schedule;
use octotrack::tui::Tui;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
enum RunMode {
    Headless,
    Tui,
    Gui,
}

#[derive(Parser, Debug)]
#[command(name = "octotrack", about = "Multi-channel audio playback and recording")]
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
}

fn detect_mode(cli: &Cli) -> RunMode {
    match (cli.headless, cli.gui) {
        (true, _) => RunMode::Headless,
        (_, true) => RunMode::Gui,
        _ if has_display() => RunMode::Gui,
        _ => RunMode::Tui,
    }
}

fn has_display() -> bool {
    std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok()
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

fn run_tui(mut app: App) -> AppResult<()> {
    let backend = CrosstermBackend::new(io::stderr());
    let terminal = Terminal::new(backend)?;
    let events = EventHandler::new(250);
    let mut tui = Tui::new(terminal, events);
    tui.init()?;

    // Start the main loop.
    while app.running {
        // Update playback info (position and levels)
        app.update_playback_info();

        // Render the user interface.
        tui.draw(&mut app)?;

        // Check if track finished and handle looping
        app.check_playback_status();

        // Handle events.
        match tui.events.next()? {
            Event::Tick => app.tick(),
            Event::Key(key_event) => handle_key_events(key_event, &mut app)?,
            Event::Mouse(_) => {}
            Event::Resize(_, _) => {}
        }
    }

    // Exit the user interface.
    let _ = app.audio_player.stop_recording();
    app.audio_player.stop()?;
    tui.exit()?;
    Ok(())
}

fn run_headless(app: Arc<Mutex<App>>) {
    // start audio engine, web server, schedule runner
    loop {
        {
            let mut app = app.lock().unwrap();
            app.update_playback_info();
            app.tick();
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn main() -> AppResult<()> {
    let cli = Cli::parse();
    let mode = detect_mode(&cli);

    let mut app = App::new();

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

    match mode {
        RunMode::Tui => run_tui(app),
        RunMode::Headless => {
            let app = Arc::new(Mutex::new(app));
            run_headless(app);
            Ok(())
        }
        RunMode::Gui => {
            // Phase 2: egui implementation — fall back to TUI for now
            run_tui(app)
        }
    }
}
