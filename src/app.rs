use std::{error, path::PathBuf, process::Command, fs};
use walkdir::WalkDir;
use crate::audio::AudioPlayer;
use serde_json::{json, Value};

/// Application result type.
pub type AppResult<T> = std::result::Result<T, Box<dyn error::Error>>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoopMode {
    NoLoop,
    LoopSingle,
    LoopAll,
}

/// Application.
pub struct App {
    /// Is the application running?
    pub running: bool,
    pub audio_player: AudioPlayer,
    pub track_list: Vec<PathBuf>,
    pub is_playing: bool,
    pub current_track_index: usize,
    pub track_title: String,
    pub track_artist: String,
    pub comment: String,
    pub track_channel_count: u32,
    pub loop_mode: LoopMode,
    pub volume: u8, // Master volume 0-100
    pub current_position: Option<f32>, // Current playback position in seconds
    pub track_duration: Option<f32>, // Total track duration in seconds
    pub channel_levels: Vec<f32>, // Per-channel RMS levels in dB
}

impl Default for App {
    fn default() -> Self {
        Self {
            running: true,
            audio_player: AudioPlayer::new(),
            track_list: vec![],
            is_playing: false,
            current_track_index: 0,
            track_title: String::new(),
            track_artist: String::new(),
            comment: String::new(),
            track_channel_count: 0,
            loop_mode: LoopMode::LoopSingle,
            volume: 100, // Start at 100%
            current_position: None,
            track_duration: None,
            channel_levels: vec![],
        }
    }
}

impl App {
    /// Constructs a new instance of [`App`].
    pub fn new() -> Self {
        let mut app = Self::default();
        let _ = app.load_config();
        app
    }

    /// Handles the tick event of the terminal.
    pub fn tick(&self) {}

    /// Set running to false to quit the application.
    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn increment_track(&mut self) {
        if self.current_track_index + 1 < self.track_list.len() {
            self.current_track_index += 1;
        } else if self.loop_mode == LoopMode::LoopAll {
            self.current_track_index = 0;
        }
        self.get_metadata();
        if self.is_playing {
            self.stop().unwrap();
            self.play();
        }
    }

    pub fn decrement_track(&mut self) {
        if self.current_track_index > 0 {
            self.current_track_index -= 1;
        } else if self.loop_mode == LoopMode::LoopAll {
            self.current_track_index = self.track_list.len() - 1;
        }
        self.get_metadata();
        if self.is_playing {
            self.stop().unwrap();
            self.play();
        }
    }

    pub fn toggle_loop_mode(&mut self) {
        self.loop_mode = match self.loop_mode {
            LoopMode::NoLoop => LoopMode::LoopSingle,
            LoopMode::LoopSingle => LoopMode::LoopAll,
            LoopMode::LoopAll => LoopMode::NoLoop,
        };
    }

    pub fn increase_volume(&mut self) {
        self.volume = (self.volume + 1).min(100);
        let _ = self.audio_player.set_volume(self.volume);
        let _ = self.save_config();
    }

    pub fn decrease_volume(&mut self) {
        self.volume = self.volume.saturating_sub(1);
        let _ = self.audio_player.set_volume(self.volume);
        let _ = self.save_config();
    }

    pub fn load_tracks(&mut self, folder_path: &str) -> AppResult<()> {
        let mut tracks = vec![];

        for entry in WalkDir::new(folder_path)
            .min_depth(1)
            .max_depth(1)
            .sort_by(|a, b| a.file_name().cmp(b.file_name())) // Sort entries alphabetically by file name
        {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => {
                    continue; // Skip this entry and log the error
                }
            };

            let path = entry.path();
            let is_hidden = path.file_name()
                .map(|name| name.to_string_lossy().starts_with('.'))
                .unwrap_or(true);

            if is_hidden {
                continue;
            }

            // Check if it's a directory with audio files
            if path.is_dir() {
                let has_audio_files = std::fs::read_dir(path)
                    .ok()
                    .map(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .any(|e| {
                                e.path().extension()
                                    .map_or(false, |ext| ext.eq_ignore_ascii_case("mp3")
                                        || ext.eq_ignore_ascii_case("wav")
                                        || ext.eq_ignore_ascii_case("flac"))
                            })
                    })
                    .unwrap_or(false);

                if has_audio_files {
                    tracks.push(path.to_path_buf());
                }
            } else if path.is_file() {
                let valid_extension = path.extension()
                    .map_or(false, |ext| ext.eq_ignore_ascii_case("mp3")
                        || ext.eq_ignore_ascii_case("wav")
                        || ext.eq_ignore_ascii_case("flac"));

                if valid_extension {
                    tracks.push(path.to_path_buf());
                }
            }
        }

        tracks.sort();
        self.track_list = tracks;

        Ok(())
    }



    pub fn play(&mut self) {
        if let Some(current_track) = self.track_list.get(self.current_track_index) {
            if !self.is_playing {
                self.audio_player.play(current_track, self.track_channel_count, self.volume).unwrap();
                self.is_playing = true;
            }
        }
    }

    pub fn get_metadata(&mut self) {
        let track_path = &self.track_list[self.current_track_index];

        // If it's a directory, get metadata from the first audio file
        if track_path.is_dir() {
            // Get all audio files in the directory
            let audio_files: Vec<PathBuf> = std::fs::read_dir(track_path)
                .unwrap()
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .map_or(false, |ext| ext.eq_ignore_ascii_case("mp3")
                            || ext.eq_ignore_ascii_case("wav")
                            || ext.eq_ignore_ascii_case("flac"))
                })
                .collect();

            if audio_files.is_empty() {
                self.track_title = track_path.file_name().unwrap().to_string_lossy().to_string();
                self.track_artist = String::new();
                self.comment = "Multi-file track".to_string();
                self.track_channel_count = 8; // Default for multi-file tracks
                return;
            }

            // Use first file for metadata, but calculate total channel count
            let first_file = audio_files[0].clone();
            let total_channels: u32 = audio_files.iter()
                .filter_map(|file| {
                    let meta = Command::new("ffprobe")
                        .arg("-v")
                        .arg("error")
                        .arg("-show_streams")
                        .arg("-of")
                        .arg("json")
                        .arg(file)
                        .output()
                        .ok()?;
                    let meta_str = String::from_utf8_lossy(&meta.stdout);
                    let meta_json: serde_json::Value = serde_json::from_str(&meta_str).ok()?;
                    meta_json["streams"][0]["channels"].as_u64().map(|c| c as u32)
                })
                .sum();

            self.track_channel_count = total_channels;

            let meta_info = Command::new("ffprobe")
                .arg("-v")
                .arg("error")
                .arg("-show_format")
                .arg("-show_streams")
                .arg("-of")
                .arg("json")
                .arg(&first_file)
                .output()
                .unwrap()
                .stdout;
            let meta_info: std::borrow::Cow<str> = String::from_utf8_lossy(&meta_info);
            let meta_info: serde_json::Value = serde_json::from_str(&meta_info).unwrap();

            let fallback_title = track_path.file_name().unwrap().to_string_lossy().to_string();

            self.track_title = meta_info["format"]["tags"]["TITLE"].as_str().unwrap_or(&fallback_title).to_string();
            self.track_artist = meta_info["format"]["tags"]["ARTIST"].as_str().unwrap_or("").to_string();
            self.comment = meta_info["format"]["tags"]["comment"].as_str().unwrap_or("").to_string();
            self.track_duration = meta_info["format"]["duration"].as_str()
                .and_then(|d| d.parse::<f32>().ok());
        } else {
            let meta_info = Command::new("ffprobe")
                .arg("-v")
                .arg("error")
                .arg("-show_format")
                .arg("-show_streams")
                .arg("-of")
                .arg("json")
                .arg(track_path)
                .output()
                .unwrap()
                .stdout;
            let meta_info: std::borrow::Cow<str> = String::from_utf8_lossy(&meta_info);
            let meta_info: serde_json::Value = serde_json::from_str(&meta_info).unwrap();

            let fallback_title = track_path.to_str().unwrap().to_string();

            self.track_title = meta_info["format"]["tags"]["TITLE"].as_str().unwrap_or(&fallback_title).to_string();
            self.track_artist = meta_info["format"]["tags"]["ARTIST"].as_str().unwrap_or("").to_string();
            self.comment = meta_info["format"]["tags"]["comment"].as_str().unwrap_or("").to_string();
            self.track_channel_count = meta_info["streams"][0]["channels"].as_u64().unwrap_or(0) as u32;
            self.track_duration = meta_info["format"]["duration"].as_str()
                .and_then(|d| d.parse::<f32>().ok());
        }

        // Initialize channel levels vector
        self.channel_levels = vec![-60.0; self.track_channel_count as usize];
    }

    pub fn stop(&mut self) -> AppResult<()> {
        self.audio_player.stop()?;
        self.is_playing = false;
        Ok(())
    }

    pub fn check_playback_status(&mut self) {
        if self.is_playing && !self.audio_player.is_running() {
            // Track finished playing
            self.is_playing = false;

            match self.loop_mode {
                LoopMode::NoLoop => {
                    // Do nothing, just stop
                }
                LoopMode::LoopSingle => {
                    // Replay the same track
                    self.play();
                }
                LoopMode::LoopAll => {
                    // Move to next track (or loop back to first)
                    if self.current_track_index + 1 < self.track_list.len() {
                        self.current_track_index += 1;
                    } else {
                        self.current_track_index = 0;
                    }
                    self.get_metadata();
                    self.play();
                }
            }
        }
    }

    pub fn update_playback_info(&mut self) {
        if self.is_playing {
            self.current_position = self.audio_player.get_time_pos().ok();
            self.channel_levels = self.audio_player.get_channel_levels();
        }
    }

    fn get_config_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config").join("octotrack").join("config.json")
    }

    pub fn load_config(&mut self) -> AppResult<()> {
        let config_path = Self::get_config_path();

        if config_path.exists() {
            let config_str = fs::read_to_string(config_path)?;
            let config: Value = serde_json::from_str(&config_str)?;

            if let Some(volume) = config["volume"].as_u64() {
                self.volume = volume as u8;
            }
        }

        Ok(())
    }

    fn save_config(&self) -> AppResult<()> {
        let config_path = Self::get_config_path();

        // Create config directory if it doesn't exist
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let config = json!({
            "volume": self.volume
        });

        fs::write(config_path, serde_json::to_string_pretty(&config)?)?;

        Ok(())
    }

}

