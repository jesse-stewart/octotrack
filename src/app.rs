use std::{error, io::Write, path::PathBuf};

use walkdir::WalkDir;

use crate::audio::AudioPlayer;

/// Application result type.
pub type AppResult<T> = std::result::Result<T, Box<dyn error::Error>>;

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
        }
    }
}

impl App {
    /// Constructs a new instance of [`App`].
    pub fn new() -> Self {
        Self::default()
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
        }
        if self.is_playing {
            self.stop().unwrap();
            self.play();
        }
    }

    pub fn decrement_track(&mut self) {
        if self.current_track_index > 0 {
            self.current_track_index -= 1;
        }
        if self.is_playing {
            self.stop().unwrap();
            self.play();
        }
    }

    pub fn load_tracks(&mut self, folder_path: &str) -> AppResult<()> {
        let mut tracks = vec![];
    
        for entry in WalkDir::new(folder_path)
            .sort_by(|a, b| a.file_name().cmp(b.file_name())) // Sort entries alphabetically by file name
        {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => {
                    continue; // Skip this entry and log the error
                }
            };
    
            let path = entry.path();
            if path.is_file() {
                let is_hidden = path.file_name()
                    .map(|name| name.to_string_lossy().starts_with('.'))
                    .unwrap_or(true); // Assume hidden if there's any issue getting the file name
    
                let valid_extension = path.extension()
                    .map_or(false, |ext| ext.eq_ignore_ascii_case("mp3") || ext.eq_ignore_ascii_case("wav") || ext.eq_ignore_ascii_case("flac"));
    
                if !is_hidden && valid_extension {
                    tracks.push(path.to_path_buf());
                }
            }
        }
    
        tracks.sort();
        self.track_list = tracks;
    
        Ok(())
    }



    pub fn play(&mut self) {
        if !self.is_playing {
            if let Some(current_track) = self.track_list.get(self.current_track_index) {
                let meta_info = self.audio_player.get_metadata(current_track).unwrap();
                self.track_title = meta_info["format"]["tags"]["TITLE"].as_str().unwrap_or(current_track.to_str().unwrap()).to_string();
                self.track_artist = meta_info["format"]["tags"]["ARTIST"].as_str().unwrap_or("").to_string();
                self.comment = meta_info["format"]["tags"]["comment"].as_str().unwrap_or("").to_string();
                self.track_channel_count = meta_info["streams"][0]["channels"].as_u64().unwrap_or(0) as u32;

                // write meta_info to a file
                let mut file = std::fs::File::create("meta_info.json").unwrap();
                file.write_all(serde_json::to_string(&meta_info).unwrap().as_bytes()).unwrap();

                self.audio_player.play(current_track, self.track_channel_count).unwrap();
                self.is_playing = true;
            }
        }
    }


    pub fn stop(&mut self) -> AppResult<()> {
        self.audio_player.stop()?;
        self.is_playing = false;
        Ok(())
    }

}

