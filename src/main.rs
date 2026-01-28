use octotrack::app::{App, AppResult};
use octotrack::event::{Event, EventHandler};
use octotrack::handler::handle_key_events;
use octotrack::tui::Tui;
use std::io;
use std::path::{Path, PathBuf};
use std::fs;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

/// Finds the tracks directory, checking USB storage first, then falling back to local directory
fn find_tracks_directory() -> String {
    // Check common USB mount points
    let usb_mount_points = vec![
        PathBuf::from("/media"),
        PathBuf::from("/mnt"),
    ];

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
                        println!("Found tracks on USB storage: {}", tracks_path.display());
                        return tracks_path.to_string_lossy().to_string();
                    }
                }
            }
        }
    }

    // Fall back to local tracks directory
    println!("Using local tracks directory");
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
                        || ext.eq_ignore_ascii_case("flac") {
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
                                    || ext.eq_ignore_ascii_case("flac") {
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

fn main() -> AppResult<()> {
    // Create an application.
    let mut app = App::new();

    // Initialize the terminal user interface.
    let backend = CrosstermBackend::new(io::stderr());
    let terminal = Terminal::new(backend)?;
    let events = EventHandler::new(250);
    let mut tui = Tui::new(terminal, events);
    tui.init()?;

    // Find tracks directory (USB first, then local fallback)
    let tracks_dir = find_tracks_directory();
    app.load_tracks(&tracks_dir).unwrap();


    // Get initial track metadata
    app.get_metadata();

    // Auto-play if enabled in config
    if app.autoplay && !app.track_list.is_empty() {
        app.play();
    }

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
    app.audio_player.stop()?;
    tui.exit()?;
    Ok(())
}
