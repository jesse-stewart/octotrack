use crate::audio::{AudioPlayer, PlayerBackend, RecordingConfig};
use crate::config::Config;
use crate::schedule::{ScheduleAction, ScheduleMsg};
use std::sync::mpsc;
use std::{error, path::PathBuf, process::Command, time::Instant};
use walkdir::WalkDir;

/// Application result type.
pub type AppResult<T> = std::result::Result<T, Box<dyn error::Error>>;

/// Commands sent from the web server to the App main loop.
pub enum AppCommand {
    Play,
    Stop,
    Prev,
    Next,
    Seek(f32),
    StartRecording,
    StopRecording,
    JumpToTrack(usize),
    RemoveTrack(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoopMode {
    NoLoop,
    LoopSingle,
    LoopAll,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AutoMode {
    Off,
    Play,
    Rec,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RecMaxMode {
    Stop,
    Drop,
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
    pub volume: u8,                    // Master volume 0-100
    pub max_volume: u8,                // Maximum volume level (softvol-max for mplayer)
    pub auto_mode: AutoMode,           // Auto action on startup: Off, Play, or Rec
    pub current_position: Option<f32>, // Current playback position in seconds
    pub track_duration: Option<f32>,   // Total track duration in seconds
    pub channel_levels: Vec<f32>,      // Per-channel RMS levels in dB
    pub show_quit_dialog: bool,        // Show quit confirmation dialog
    pub show_save_dialog: bool,        // Show save config confirmation dialog
    pub eq_bands: [i8; 10],            // 10-band EQ gain values (-12 to +12 dB)
    pub eq_enabled: bool,              // EQ bypass toggle
    pub show_eq: bool,                 // Show EQ overlay
    pub eq_selected_band: usize,       // Currently selected EQ band (0-9)
    pub is_recording: bool,
    pub recording_start_time: Option<Instant>,
    pub recording_path: Option<PathBuf>,
    pub tracks_dir: String,
    pub playback_device: String,       // ALSA output device for playback
    pub playback_channel_count: u32,   // Number of output channels the playback device supports
    pub rec_input_device: String,      // ALSA input device for recording
    pub rec_channel_count: u32,        // Number of channels to record
    pub rec_sample_rate: u32, // Sample rate for recording (e.g. 44100, 48000, 96000, 192000)
    pub rec_bit_depth: u16,   // Bit depth for recording (16, 24, or 32)
    pub rec_max_file_mb: u64, // Max recording file size in MB (0 = unlimited)
    pub rec_max_file_mode: RecMaxMode, // What to do when max size is reached
    pub rec_min_free_mb: u64, // Stop/drop when free disk space drops below this (MB)
    pub rec_split_file_mb: u64, // Split recording into files of this size in MB (0 = no splitting)
    pub mon_output_device: String, // ALSA output device for monitoring (should match playback card)
    pub is_monitoring: bool,
    pub start_track: String, // Filename (or partial name) to select on startup; "" = first track
    pub schedule_rx: Option<mpsc::Receiver<ScheduleMsg>>,
    /// Full TOML config, kept in sync with flat runtime fields on save.
    /// Sections not yet wired to runtime state (network, web, channels, etc.)
    /// live here until their respective phases are implemented.
    pub config: Config,
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
            volume: 100,     // Start at 100%
            max_volume: 100, // 100% of original audio level
            auto_mode: AutoMode::Off,
            current_position: None,
            track_duration: None,
            channel_levels: vec![],
            show_quit_dialog: false, // Dialog hidden by default
            show_save_dialog: false,
            eq_bands: [0; 10], // Flat EQ by default
            eq_enabled: true,
            show_eq: false,
            eq_selected_band: 0,
            is_recording: false,
            recording_start_time: None,
            recording_path: None,
            tracks_dir: "tracks".to_string(),
            playback_device: "hw:0,0".to_string(),
            playback_channel_count: 8,
            rec_input_device: "hw:0,0".to_string(),
            rec_channel_count: 8,
            rec_sample_rate: 192_000,
            rec_bit_depth: 32,
            rec_max_file_mb: 0, // 0 = unlimited
            rec_max_file_mode: RecMaxMode::Stop,
            rec_min_free_mb: 1024, // 1 GB safety margin
            rec_split_file_mb: 0,  // 0 = no splitting
            mon_output_device: "hw:0,0".to_string(),
            is_monitoring: false,
            start_track: String::new(),
            schedule_rx: None,
            config: Config::default(),
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

    /// Constructs a new [`App`] from an already-loaded [`Config`].
    /// Used when the caller has pre-loaded or mutated config (e.g. first-run setup).
    pub fn new_with_config(config: Config) -> Self {
        let mut app = Self::default();
        app.apply_config(&config);
        app.config = config;
        app
    }

    /// Handles the tick event of the terminal.
    pub fn tick(&mut self) {
        // Drain any pending scheduled actions.
        // Take the receiver out to avoid holding a borrow while calling &mut self methods.
        let rx = self.schedule_rx.take();
        if let Some(ref r) = rx {
            let msgs: Vec<ScheduleMsg> = std::iter::from_fn(|| r.try_recv().ok()).collect();
            self.schedule_rx = rx;
            for msg in msgs {
                match msg {
                    ScheduleMsg::Start {
                        action: ScheduleAction::Rec,
                        ..
                    } => {
                        let _ = self.start_recording();
                    }
                    ScheduleMsg::Stop(ScheduleAction::Rec) => {
                        let _ = self.stop_recording();
                    }
                    ScheduleMsg::Start {
                        action: ScheduleAction::Play,
                        start_track,
                    } => {
                        // If a start_track is specified, find and select it first.
                        if let Some(ref needle) = start_track {
                            let needle = needle.to_lowercase();
                            if let Some(idx) = self.track_list.iter().position(|p| {
                                p.file_name()
                                    .map(|n| n.to_string_lossy().to_lowercase().contains(&needle))
                                    .unwrap_or(false)
                            }) {
                                self.current_track_index = idx;
                                self.get_metadata();
                            }
                        }
                        if !self.track_list.is_empty() {
                            self.play();
                        }
                    }
                    ScheduleMsg::Stop(ScheduleAction::Play) => {
                        let _ = self.stop();
                    }
                }
            }
        } else {
            self.schedule_rx = rx;
        }
    }

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

    pub fn cycle_auto_mode(&mut self) {
        self.auto_mode = match self.auto_mode {
            AutoMode::Off => AutoMode::Play,
            AutoMode::Play => AutoMode::Rec,
            AutoMode::Rec => AutoMode::Off,
        };
    }

    pub fn increase_volume(&mut self) {
        self.volume = (self.volume + 1).min(100);
        let _ = self.audio_player.set_volume(self.volume);
    }

    pub fn decrease_volume(&mut self) {
        self.volume = self.volume.saturating_sub(1);
        let _ = self.audio_player.set_volume(self.volume);
    }

    pub fn toggle_eq_view(&mut self) {
        self.show_eq = !self.show_eq;
    }

    pub fn toggle_eq_enabled(&mut self) {
        self.eq_enabled = !self.eq_enabled;
        if self.is_playing {
            let _ = self
                .audio_player
                .set_eq_enabled(&self.eq_bands, self.eq_enabled);
        }
    }

    pub fn eq_select_next(&mut self) {
        self.eq_selected_band = (self.eq_selected_band + 1).min(9);
    }

    pub fn eq_select_prev(&mut self) {
        self.eq_selected_band = self.eq_selected_band.saturating_sub(1);
    }

    pub fn eq_increase_band(&mut self) {
        let band = self.eq_selected_band;
        self.eq_bands[band] = (self.eq_bands[band] + 1).min(12);
        if self.is_playing && self.eq_enabled {
            let _ = self.audio_player.update_eq_bands(&self.eq_bands);
        }
    }

    pub fn eq_decrease_band(&mut self) {
        let band = self.eq_selected_band;
        self.eq_bands[band] = (self.eq_bands[band] - 1).max(-12);
        if self.is_playing && self.eq_enabled {
            let _ = self.audio_player.update_eq_bands(&self.eq_bands);
        }
    }

    pub fn load_tracks(&mut self, folder_path: &str) -> AppResult<()> {
        self.tracks_dir = folder_path.to_string();
        let mut tracks = vec![];

        for entry in WalkDir::new(folder_path)
            .min_depth(1)
            .max_depth(1)
            .sort_by(|a, b| a.file_name().cmp(b.file_name()))
        // Sort entries alphabetically by file name
        {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => {
                    continue; // Skip this entry and log the error
                }
            };

            let path = entry.path();
            let is_hidden = path
                .file_name()
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
                        entries.filter_map(|e| e.ok()).any(|e| {
                            e.path().extension().is_some_and(|ext| {
                                ext.eq_ignore_ascii_case("mp3")
                                    || ext.eq_ignore_ascii_case("wav")
                                    || ext.eq_ignore_ascii_case("flac")
                            })
                        })
                    })
                    .unwrap_or(false);

                if has_audio_files {
                    tracks.push(path.to_path_buf());
                }
            } else if path.is_file() {
                let valid_extension = path.extension().is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("mp3")
                        || ext.eq_ignore_ascii_case("wav")
                        || ext.eq_ignore_ascii_case("flac")
                });

                if valid_extension {
                    tracks.push(path.to_path_buf());
                }
            }
        }

        tracks.sort();
        self.track_list = tracks;

        // Apply start_track: find the first track whose filename contains the search string
        // (case-insensitive). Falls back to index 0 if empty or no match.
        if !self.start_track.is_empty() {
            let needle = self.start_track.to_lowercase();
            if let Some(idx) = self.track_list.iter().position(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().to_lowercase().contains(&needle))
                    .unwrap_or(false)
            }) {
                self.current_track_index = idx;
            } else {
                self.current_track_index = 0;
            }
        } else {
            self.current_track_index = 0;
        }

        Ok(())
    }

    pub fn play(&mut self) {
        if self.track_list.get(self.current_track_index).is_some() && !self.is_playing {
            if self.is_monitoring {
                let _ = self.stop_monitoring();
            }
            let current_track = self.track_list[self.current_track_index].clone();
            self.audio_player
                .play(
                    &current_track,
                    self.track_channel_count,
                    self.playback_channel_count,
                    self.volume,
                    self.max_volume,
                    &self.eq_bands,
                    self.eq_enabled,
                    &self.playback_device,
                )
                .unwrap();
            self.is_playing = true;
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
                    p.extension().is_some_and(|ext| {
                        ext.eq_ignore_ascii_case("mp3")
                            || ext.eq_ignore_ascii_case("wav")
                            || ext.eq_ignore_ascii_case("flac")
                    })
                })
                .collect();

            if audio_files.is_empty() {
                self.track_title = track_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                self.track_artist = String::new();
                self.comment = "Multi-file track".to_string();
                self.track_channel_count = 8; // Default for multi-file tracks
                return;
            }

            // Use first file for metadata; infer total channels from first file's channels * file count
            let first_file = audio_files[0].clone();

            let meta_info = Command::new("ffprobe")
                .arg("-v")
                .arg("error")
                .arg("-show_format")
                .arg("-show_streams")
                .arg("-of")
                .arg("json")
                .arg(&first_file)
                .output()
                .map(|o| o.stdout)
                .unwrap_or_default();
            let meta_info: std::borrow::Cow<str> = String::from_utf8_lossy(&meta_info);
            let meta_info: serde_json::Value =
                serde_json::from_str(&meta_info).unwrap_or(serde_json::Value::Null);

            let channels_per_file =
                meta_info["streams"][0]["channels"].as_u64().unwrap_or(1) as u32;
            let total_channels = channels_per_file * audio_files.len() as u32;
            self.track_channel_count = total_channels;

            let fallback_title = track_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();

            // Try both uppercase and lowercase tag names (different formats use different cases)
            let title = meta_info["format"]["tags"]["TITLE"]
                .as_str()
                .or_else(|| meta_info["format"]["tags"]["title"].as_str())
                .unwrap_or(&fallback_title);
            let artist = meta_info["format"]["tags"]["ARTIST"]
                .as_str()
                .or_else(|| meta_info["format"]["tags"]["artist"].as_str())
                .unwrap_or("");
            let comment = meta_info["format"]["tags"]["comment"]
                .as_str()
                .or_else(|| meta_info["format"]["tags"]["COMMENT"].as_str())
                .unwrap_or("");

            self.track_title = title.to_string();
            self.track_artist = artist.to_string();
            self.comment = comment.to_string();
            self.track_duration = meta_info["format"]["duration"]
                .as_str()
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
                .map(|o| o.stdout)
                .unwrap_or_default();
            let meta_info: std::borrow::Cow<str> = String::from_utf8_lossy(&meta_info);
            let meta_info: serde_json::Value =
                serde_json::from_str(&meta_info).unwrap_or(serde_json::Value::Null);

            // Use filename without extension as fallback
            let fallback_title = track_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .to_string();

            // Try both uppercase and lowercase tag names (different formats use different cases)
            let title = meta_info["format"]["tags"]["TITLE"]
                .as_str()
                .or_else(|| meta_info["format"]["tags"]["title"].as_str())
                .unwrap_or(&fallback_title);
            let artist = meta_info["format"]["tags"]["ARTIST"]
                .as_str()
                .or_else(|| meta_info["format"]["tags"]["artist"].as_str())
                .unwrap_or("");
            let comment = meta_info["format"]["tags"]["comment"]
                .as_str()
                .or_else(|| meta_info["format"]["tags"]["COMMENT"].as_str())
                .unwrap_or("");

            self.track_title = title.to_string();
            self.track_artist = artist.to_string();
            self.comment = comment.to_string();
            self.track_channel_count =
                meta_info["streams"][0]["channels"].as_u64().unwrap_or(0) as u32;
            self.track_duration = meta_info["format"]["duration"]
                .as_str()
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

    pub fn toggle_recording(&mut self) {
        if self.is_recording {
            let _ = self.stop_recording();
        } else {
            let _ = self.start_recording();
        }
    }

    pub fn start_recording(&mut self) -> AppResult<()> {
        if self.is_playing {
            self.stop()?;
        }

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let filename = format!("REC_{}.wav", ts);
        let output_path = PathBuf::from(&self.tracks_dir).join(&filename);

        let input_device = self.rec_input_device.clone();
        let channel_count = self.rec_channel_count;
        let sample_rate = self.rec_sample_rate;
        let bit_depth = self.rec_bit_depth;
        let max_data_bytes = if self.rec_max_file_mb > 0 {
            Some(self.rec_max_file_mb * 1024 * 1024)
        } else {
            None
        };
        let rec_cfg = RecordingConfig {
            max_data_bytes,
            drop_mode: self.rec_max_file_mode == RecMaxMode::Drop,
            min_free_bytes: self.rec_min_free_mb * 1024 * 1024,
            split_size_bytes: if self.rec_split_file_mb > 0 {
                Some(self.rec_split_file_mb * 1024 * 1024)
            } else {
                None
            },
        };
        self.audio_player.start_recording(
            &output_path,
            &input_device,
            channel_count,
            sample_rate,
            bit_depth,
            rec_cfg,
        )?;
        // If monitoring was active, re-enable it on the new capture session.
        if self.is_monitoring {
            let output_device = self.mon_output_device.clone();
            let _ = self.audio_player.start_monitoring(
                &input_device,
                &output_device,
                channel_count,
                sample_rate,
                bit_depth,
            );
        }
        self.is_recording = true;
        self.recording_start_time = Some(Instant::now());
        let stem = filename.strip_suffix(".wav").unwrap_or(&filename);
        self.track_title = format!("{}/{}", self.tracks_dir, stem);
        // When splitting, the first file on disk is _001.wav — use that as the recording path
        // so stop_recording can find it in the track list after the session ends.
        let first_path = if self.rec_split_file_mb > 0 {
            PathBuf::from(&self.tracks_dir).join(format!("{}_001.wav", stem))
        } else {
            output_path
        };
        self.recording_path = Some(first_path);
        Ok(())
    }

    pub fn stop_recording(&mut self) -> AppResult<()> {
        self.audio_player.stop_recording()?;
        self.is_recording = false;
        self.recording_start_time = None;
        // If monitoring was active, restart it (stop_recording tears down the shared capture).
        if self.is_monitoring {
            let input_device = self.rec_input_device.clone();
            let output_device = self.mon_output_device.clone();
            let channel_count = self.rec_channel_count;
            let _ = self.audio_player.start_monitoring(
                &input_device,
                &output_device,
                channel_count,
                self.rec_sample_rate,
                self.rec_bit_depth,
            );
        }

        let saved_path = self.recording_path.take();

        // Reload tracks so the new recording appears in the list
        let tracks_dir = self.tracks_dir.clone();
        let _ = self.load_tracks(&tracks_dir);

        // Select the track we just recorded
        if let Some(ref rec_path) = saved_path {
            // Canonicalise both sides of the comparison so relative vs absolute doesn't matter
            let canon_rec = rec_path.canonicalize().unwrap_or_else(|_| rec_path.clone());
            if let Some(idx) = self
                .track_list
                .iter()
                .position(|p| p.canonicalize().unwrap_or_else(|_| p.clone()) == canon_rec)
            {
                self.current_track_index = idx;
                self.get_metadata();
            }
        }
        Ok(())
    }

    pub fn recording_elapsed(&self) -> f32 {
        self.recording_start_time
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0)
    }

    pub fn recording_file_bytes(&self) -> u64 {
        *self.audio_player.capture_recording_bytes.lock().unwrap()
    }

    pub fn check_playback_status(&mut self) {
        // Detect if monitoring stopped unexpectedly
        if self.is_monitoring && !self.audio_player.is_monitoring() {
            self.is_monitoring = false;
        }

        // Detect if recording stopped unexpectedly (e.g. ffmpeg error)
        if self.is_recording && !self.audio_player.is_recording() {
            self.is_recording = false;
            self.recording_start_time = None;
            let saved_path = self.recording_path.take();
            let tracks_dir = self.tracks_dir.clone();
            let _ = self.load_tracks(&tracks_dir);
            if let Some(ref rec_path) = saved_path {
                let canon_rec = rec_path.canonicalize().unwrap_or_else(|_| rec_path.clone());
                if let Some(idx) = self
                    .track_list
                    .iter()
                    .position(|p| p.canonicalize().unwrap_or_else(|_| p.clone()) == canon_rec)
                {
                    self.current_track_index = idx;
                    self.get_metadata();
                }
            }
        }

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

    pub fn toggle_monitoring(&mut self) {
        if self.is_monitoring {
            let _ = self.stop_monitoring();
        } else {
            let _ = self.start_monitoring();
        }
    }

    pub fn start_monitoring(&mut self) -> AppResult<()> {
        if self.is_playing {
            self.stop()?;
        }
        let input_device = self.rec_input_device.clone();
        let output_device = self.mon_output_device.clone();
        let channel_count = self.rec_channel_count;
        self.channel_levels = vec![-60.0; channel_count as usize];
        self.audio_player.start_monitoring(
            &input_device,
            &output_device,
            channel_count,
            self.rec_sample_rate,
            self.rec_bit_depth,
        )?;
        self.is_monitoring = true;
        Ok(())
    }

    pub fn stop_monitoring(&mut self) -> AppResult<()> {
        self.audio_player.stop_monitoring()?;
        self.is_monitoring = false;
        Ok(())
    }

    pub fn update_playback_info(&mut self) {
        if self.is_playing {
            self.current_position = self.audio_player.get_time_pos().ok();
            self.channel_levels = self.audio_player.get_channel_levels();
        } else if self.is_recording || self.is_monitoring {
            self.channel_levels = self.audio_player.get_raw_levels();
        }
    }

    /// Load config from disk (TOML, or migrate from legacy JSON) and apply
    /// all active fields to this App instance.
    pub fn load_config(&mut self) -> AppResult<()> {
        let cfg = Config::load();
        self.apply_config(&cfg);
        self.config = cfg;
        Ok(())
    }

    /// Apply the fields of a `Config` that have runtime counterparts on `App`.
    pub fn apply_config(&mut self, cfg: &Config) {
        // [playback]
        self.volume = cfg.playback.volume;
        self.max_volume = cfg.playback.max_volume;
        self.auto_mode = match cfg.playback.auto_mode.as_str() {
            "play" => AutoMode::Play,
            "rec" => AutoMode::Rec,
            _ => AutoMode::Off,
        };
        self.loop_mode = match cfg.playback.loop_mode.as_str() {
            "single" => LoopMode::LoopSingle,
            "all" => LoopMode::LoopAll,
            _ => LoopMode::NoLoop,
        };
        self.start_track = cfg.playback.start_track.clone();
        self.playback_device = cfg.playback.device.clone();
        self.playback_channel_count = cfg.playback.channel_count;

        // [playback.eq]
        let mut bands = [0i8; 10];
        for (i, &b) in cfg.playback.eq.bands.iter().enumerate().take(10) {
            bands[i] = b.clamp(-12, 12);
        }
        self.eq_bands = bands;
        self.eq_enabled = cfg.playback.eq.enabled;

        // [recording]
        self.rec_input_device = cfg.recording.input_device.clone();
        self.rec_channel_count = cfg.recording.channel_count;
        self.rec_sample_rate = cfg.recording.sample_rate;
        let bd = cfg.recording.bit_depth;
        if bd == 16 || bd == 24 || bd == 32 {
            self.rec_bit_depth = bd;
        }
        self.rec_max_file_mb = cfg.recording.max_file_mb;
        self.rec_max_file_mode = match cfg.recording.max_file_mode.as_str() {
            "drop" => RecMaxMode::Drop,
            _ => RecMaxMode::Stop,
        };
        self.rec_min_free_mb = cfg.recording.min_free_mb;
        self.rec_split_file_mb = cfg.recording.split_file_mb;

        // [monitoring]
        self.mon_output_device = cfg.monitoring.output_device.clone();

        // [tools]
        self.audio_player.backend = PlayerBackend::from_str(&cfg.tools.player);
        self.audio_player.player_cmd = if cfg.tools.player == "mpv" {
            cfg.tools.mpv.clone()
        } else {
            cfg.tools.mplayer.clone()
        };
        self.audio_player.mpv_cmd = cfg.tools.mpv.clone();
        self.audio_player.ffmpeg_cmd = cfg.tools.ffmpeg.clone();
    }

    /// Sync runtime App state back into `self.config` and write to disk.
    pub fn save_config(&self) -> AppResult<()> {
        let mut cfg = self.config.clone();

        // [playback]
        cfg.playback.volume = self.volume;
        cfg.playback.max_volume = self.max_volume;
        cfg.playback.auto_mode = match self.auto_mode {
            AutoMode::Off => "off",
            AutoMode::Play => "play",
            AutoMode::Rec => "rec",
        }
        .to_string();
        cfg.playback.loop_mode = match self.loop_mode {
            LoopMode::NoLoop => "off",
            LoopMode::LoopSingle => "single",
            LoopMode::LoopAll => "all",
        }
        .to_string();
        cfg.playback.start_track = self.start_track.clone();
        cfg.playback.device = self.playback_device.clone();
        cfg.playback.channel_count = self.playback_channel_count;

        // [playback.eq]
        cfg.playback.eq.bands = self.eq_bands.to_vec();
        cfg.playback.eq.enabled = self.eq_enabled;

        // [recording]
        cfg.recording.input_device = self.rec_input_device.clone();
        cfg.recording.channel_count = self.rec_channel_count;
        cfg.recording.sample_rate = self.rec_sample_rate;
        cfg.recording.bit_depth = self.rec_bit_depth;
        cfg.recording.max_file_mb = self.rec_max_file_mb;
        cfg.recording.max_file_mode = match self.rec_max_file_mode {
            RecMaxMode::Stop => "stop",
            RecMaxMode::Drop => "drop",
        }
        .to_string();
        cfg.recording.min_free_mb = self.rec_min_free_mb;
        cfg.recording.split_file_mb = self.rec_split_file_mb;

        // [monitoring]
        cfg.monitoring.output_device = self.mon_output_device.clone();

        cfg.save()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create an App without loading config from disk.
    fn test_app() -> App {
        App::default()
    }

    // -----------------------------------------------------------------------
    // Loop mode cycling
    // -----------------------------------------------------------------------

    #[test]
    fn toggle_loop_mode_cycles() {
        let mut app = test_app();
        assert_eq!(app.loop_mode, LoopMode::LoopSingle);
        app.toggle_loop_mode();
        assert_eq!(app.loop_mode, LoopMode::LoopAll);
        app.toggle_loop_mode();
        assert_eq!(app.loop_mode, LoopMode::NoLoop);
        app.toggle_loop_mode();
        assert_eq!(app.loop_mode, LoopMode::LoopSingle);
    }

    // -----------------------------------------------------------------------
    // Auto mode cycling
    // -----------------------------------------------------------------------

    #[test]
    fn cycle_auto_mode_cycles() {
        let mut app = test_app();
        assert_eq!(app.auto_mode, AutoMode::Off);
        app.cycle_auto_mode();
        assert_eq!(app.auto_mode, AutoMode::Play);
        app.cycle_auto_mode();
        assert_eq!(app.auto_mode, AutoMode::Rec);
        app.cycle_auto_mode();
        assert_eq!(app.auto_mode, AutoMode::Off);
    }

    // -----------------------------------------------------------------------
    // Volume
    // -----------------------------------------------------------------------

    #[test]
    fn volume_clamps_at_100() {
        let mut app = test_app();
        app.volume = 100;
        app.increase_volume();
        assert_eq!(app.volume, 100);
    }

    #[test]
    fn volume_clamps_at_0() {
        let mut app = test_app();
        app.volume = 0;
        app.decrease_volume();
        assert_eq!(app.volume, 0);
    }

    #[test]
    fn volume_increments() {
        let mut app = test_app();
        app.volume = 50;
        app.increase_volume();
        assert_eq!(app.volume, 51);
        app.decrease_volume();
        assert_eq!(app.volume, 50);
    }

    // -----------------------------------------------------------------------
    // EQ
    // -----------------------------------------------------------------------

    #[test]
    fn eq_select_wraps_at_bounds() {
        let mut app = test_app();
        assert_eq!(app.eq_selected_band, 0);
        app.eq_select_prev();
        assert_eq!(app.eq_selected_band, 0); // stays at 0

        app.eq_selected_band = 9;
        app.eq_select_next();
        assert_eq!(app.eq_selected_band, 9); // stays at 9
    }

    #[test]
    fn eq_band_clamps_at_bounds() {
        let mut app = test_app();
        app.eq_selected_band = 0;
        for _ in 0..20 {
            app.eq_increase_band();
        }
        assert_eq!(app.eq_bands[0], 12);

        for _ in 0..30 {
            app.eq_decrease_band();
        }
        assert_eq!(app.eq_bands[0], -12);
    }

    #[test]
    fn eq_toggle() {
        let mut app = test_app();
        assert!(app.eq_enabled);
        app.toggle_eq_enabled();
        assert!(!app.eq_enabled);
        app.toggle_eq_enabled();
        assert!(app.eq_enabled);
    }

    #[test]
    fn eq_view_toggle() {
        let mut app = test_app();
        assert!(!app.show_eq);
        app.toggle_eq_view();
        assert!(app.show_eq);
        app.toggle_eq_view();
        assert!(!app.show_eq);
    }

    // -----------------------------------------------------------------------
    // Track navigation (no audio)
    // -----------------------------------------------------------------------

    #[test]
    fn increment_track_no_wrap_without_loop_all() {
        let mut app = test_app();
        app.loop_mode = LoopMode::NoLoop;
        app.track_list = vec![
            PathBuf::from("a.wav"),
            PathBuf::from("b.wav"),
            PathBuf::from("c.wav"),
        ];
        app.current_track_index = 2;
        app.increment_track();
        assert_eq!(app.current_track_index, 2); // stays at end
    }

    #[test]
    fn increment_track_wraps_with_loop_all() {
        let mut app = test_app();
        app.loop_mode = LoopMode::LoopAll;
        app.track_list = vec![PathBuf::from("a.wav"), PathBuf::from("b.wav")];
        app.current_track_index = 1;
        app.increment_track();
        assert_eq!(app.current_track_index, 0);
    }

    #[test]
    fn decrement_track_wraps_with_loop_all() {
        let mut app = test_app();
        app.loop_mode = LoopMode::LoopAll;
        app.track_list = vec![
            PathBuf::from("a.wav"),
            PathBuf::from("b.wav"),
            PathBuf::from("c.wav"),
        ];
        app.current_track_index = 0;
        app.decrement_track();
        assert_eq!(app.current_track_index, 2);
    }

    #[test]
    fn decrement_track_stays_at_zero() {
        let mut app = test_app();
        app.loop_mode = LoopMode::NoLoop;
        app.track_list = vec![PathBuf::from("a.wav")];
        app.current_track_index = 0;
        app.decrement_track();
        assert_eq!(app.current_track_index, 0);
    }

    // -----------------------------------------------------------------------
    // Quit
    // -----------------------------------------------------------------------

    #[test]
    fn quit_sets_running_false() {
        let mut app = test_app();
        assert!(app.running);
        app.quit();
        assert!(!app.running);
    }

    // -----------------------------------------------------------------------
    // Config application (apply_config)
    // -----------------------------------------------------------------------

    fn full_config() -> Config {
        use crate::config::*;
        Config {
            playback: PlaybackConfig {
                device: "hw:1,0".to_string(),
                channel_count: 2,
                volume: 75,
                max_volume: 80,
                loop_mode: "all".to_string(),
                auto_mode: "rec".to_string(),
                start_track: "my_song".to_string(),
                eq: EqConfig {
                    enabled: false,
                    bands: vec![1, 2, 3, 4, 5, -1, -2, -3, -4, -5],
                },
            },
            recording: RecordingSettings {
                input_device: "hw:2,0".to_string(),
                channel_count: 4,
                sample_rate: 96_000,
                bit_depth: 24,
                max_file_mb: 4000,
                max_file_mode: "drop".to_string(),
                min_free_mb: 2048,
                split_file_mb: 3900,
                filename_template: "REC_{timestamp}".to_string(),
            },
            monitoring: MonitoringConfig {
                output_device: "hw:3,0".to_string(),
                ..MonitoringConfig::default()
            },
            ..Config::default()
        }
    }

    #[test]
    fn apply_config_applies_all_fields() {
        let mut app = test_app();
        app.apply_config(&full_config());

        assert_eq!(app.volume, 75);
        assert_eq!(app.max_volume, 80);
        assert_eq!(app.auto_mode, AutoMode::Rec);
        assert_eq!(app.eq_bands, [1, 2, 3, 4, 5, -1, -2, -3, -4, -5]);
        assert!(!app.eq_enabled);
        assert_eq!(app.playback_device, "hw:1,0");
        assert_eq!(app.playback_channel_count, 2);
        assert_eq!(app.rec_input_device, "hw:2,0");
        assert_eq!(app.rec_channel_count, 4);
        assert_eq!(app.rec_sample_rate, 96_000);
        assert_eq!(app.rec_bit_depth, 24);
        assert_eq!(app.rec_max_file_mb, 4000);
        assert_eq!(app.rec_max_file_mode, RecMaxMode::Drop);
        assert_eq!(app.rec_min_free_mb, 2048);
        assert_eq!(app.rec_split_file_mb, 3900);
        assert_eq!(app.loop_mode, LoopMode::LoopAll);
        assert_eq!(app.start_track, "my_song");
        assert_eq!(app.mon_output_device, "hw:3,0");
    }

    #[test]
    fn apply_config_ignores_invalid_bit_depth() {
        use crate::config::RecordingSettings;
        let mut app = test_app();
        let mut cfg = Config::default();
        cfg.recording = RecordingSettings {
            bit_depth: 20, // invalid
            ..RecordingSettings::default()
        };
        app.apply_config(&cfg);
        assert_eq!(app.rec_bit_depth, 32); // unchanged from App::default()
    }

    #[test]
    fn apply_config_clamps_eq_bands() {
        use crate::config::EqConfig;
        let mut app = test_app();
        let mut cfg = Config::default();
        cfg.playback.eq = EqConfig {
            enabled: true,
            bands: vec![99, -99, 0, 0, 0, 0, 0, 0, 0, 0],
        };
        app.apply_config(&cfg);
        assert_eq!(app.eq_bands[0], 12);
        assert_eq!(app.eq_bands[1], -12);
    }

    #[test]
    fn apply_config_partial_preserves_defaults() {
        let mut app = test_app();
        let mut cfg = Config::default();
        cfg.playback.volume = 42;
        app.apply_config(&cfg);
        assert_eq!(app.volume, 42);
        assert_eq!(app.rec_sample_rate, 192_000); // App::default() preserved
    }

    // -----------------------------------------------------------------------
    // Defaults
    // -----------------------------------------------------------------------

    #[test]
    fn default_values() {
        let app = test_app();
        assert!(app.running);
        assert!(!app.is_playing);
        assert!(!app.is_recording);
        assert!(!app.is_monitoring);
        assert_eq!(app.volume, 100);
        assert_eq!(app.loop_mode, LoopMode::LoopSingle);
        assert_eq!(app.auto_mode, AutoMode::Off);
        assert_eq!(app.eq_bands, [0; 10]);
        assert!(app.eq_enabled);
        assert_eq!(app.rec_bit_depth, 32);
        assert_eq!(app.rec_sample_rate, 192_000);
    }
}
