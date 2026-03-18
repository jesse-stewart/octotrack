//! Application configuration — TOML schema, load, save, and JSON migration.
//!
//! The on-disk format is `~/.config/octotrack/config.toml`.
//! On first run, if a legacy `config.json` is found, it is automatically
//! migrated and renamed to `config.json.bak`.
//!
//! `Config::save()` uses `toml_edit` for surgical in-place writes so that
//! any comments the user has added to the file are preserved.

use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// `~/.config/octotrack/config.toml`
pub fn toml_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("octotrack")
        .join("config.toml")
}

/// `~/.config/octotrack/config.json` (legacy)
pub fn json_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("octotrack")
        .join("config.json")
}

// ---------------------------------------------------------------------------
// Top-level config
// ---------------------------------------------------------------------------

/// Top-level application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub playback: PlaybackConfig,
    pub recording: RecordingSettings,
    pub monitoring: MonitoringConfig,
    /// Per-channel trim and labels.  Empty = use device defaults.
    pub channels: Vec<ChannelConfig>,
    pub storage: StorageConfig,
    pub display: DisplayConfig,
    pub network: NetworkConfig,
    pub web: WebConfig,
    pub tools: ToolsConfig,
    pub logging: LoggingConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            playback: PlaybackConfig::default(),
            recording: RecordingSettings::default(),
            monitoring: MonitoringConfig::default(),
            channels: vec![],
            storage: StorageConfig::default(),
            display: DisplayConfig::default(),
            network: NetworkConfig::default(),
            web: WebConfig::default(),
            tools: ToolsConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// [playback]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackConfig {
    pub device: String,
    pub channel_count: u32,
    pub volume: u8,
    pub max_volume: u8,
    /// `"off"` | `"single"` | `"all"`
    pub loop_mode: String,
    /// `"off"` | `"play"` | `"rec"`
    pub auto_mode: String,
    /// Filename (or partial name) to jump to on startup; `""` = first track.
    pub start_track: String,
    pub eq: EqConfig,
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            device: "hw:0,0".to_string(),
            channel_count: 8,
            volume: 80,
            max_volume: 100,
            loop_mode: "single".to_string(),
            auto_mode: "off".to_string(),
            start_track: String::new(),
            eq: EqConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// [playback.eq]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EqConfig {
    pub enabled: bool,
    /// 10 band gains in dB, clamped to -12..+12.
    pub bands: Vec<i8>,
}

impl Default for EqConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bands: vec![0; 10],
        }
    }
}

// ---------------------------------------------------------------------------
// [recording]
// Note: `audio::RecordingConfig` is the runtime byte-level config derived
// from these user-facing MB/mode values.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordingSettings {
    pub input_device: String,
    pub channel_count: u32,
    pub sample_rate: u32,
    /// 16, 24, or 32.
    pub bit_depth: u16,
    /// Split into new file every N MB.  0 = no splitting.
    pub split_file_mb: u64,
    /// Maximum single-file size in MB.  0 = unlimited.
    pub max_file_mb: u64,
    /// `"stop"` | `"drop"` (circular-buffer overwrite).
    pub max_file_mode: String,
    /// Stop/drop when free disk space drops below this threshold in MB.
    pub min_free_mb: u64,
    /// Filename template.  Tokens: `{timestamp}` `{date}` `{track}`.
    pub filename_template: String,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self {
            input_device: "hw:0,0".to_string(),
            channel_count: 8,
            sample_rate: 192_000,
            bit_depth: 32,
            split_file_mb: 0,
            max_file_mb: 0,
            max_file_mode: "stop".to_string(),
            min_free_mb: 1024,
            filename_template: "REC_{timestamp}".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// [monitoring]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MonitoringConfig {
    pub output_device: String,
    pub peak_hold_ms: u32,
    pub meter_decay_db_per_sec: f32,
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            output_device: "hw:0,0".to_string(),
            peak_hold_ms: 1500,
            meter_decay_db_per_sec: 20.0,
        }
    }
}

// ---------------------------------------------------------------------------
// [[channels]]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelConfig {
    /// 1-based channel index.
    pub index: u8,
    pub label: String,
    pub trim_db: f32,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            index: 1,
            label: "Ch 1".to_string(),
            trim_db: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// [storage]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// Explicit tracks directory.  `""` = auto-detect USB then `./tracks`.
    pub tracks_dir: String,
    pub usb_mount_paths: Vec<String>,
    /// Subdirectory under tracks_dir for recordings.  `""` = same dir.
    pub recordings_subdir: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            tracks_dir: String::new(),
            usb_mount_paths: vec!["/media".to_string(), "/mnt".to_string()],
            recordings_subdir: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// [display]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    /// `"auto"` | `"tui"` | `"gui"` | `"headless"`
    pub mode: String,
    pub scale_factor: f32,
    /// `"dark"` | `"light"`
    pub theme: String,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            mode: "auto".to_string(),
            scale_factor: 1.0,
            theme: "dark".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// [network]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    pub ap: AccessPointConfig,
    pub known_networks: Vec<KnownNetworkConfig>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            ap: AccessPointConfig::default(),
            known_networks: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AccessPointConfig {
    pub enabled: bool,
    pub ssid: String,
    /// Set at first run via `--set-password`.
    pub password: String,
    pub channel: u8,
    pub country_code: String,
    pub address: String,
    pub dhcp_range_start: String,
    pub dhcp_range_end: String,
}

impl Default for AccessPointConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ssid: "octotrack".to_string(),
            password: String::new(),
            channel: 6,
            country_code: "US".to_string(),
            address: "192.168.42.1".to_string(),
            dhcp_range_start: "192.168.42.2".to_string(),
            dhcp_range_end: "192.168.42.20".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KnownNetworkConfig {
    pub ssid: String,
    pub psk: String,
    pub priority: u8,
}

impl Default for KnownNetworkConfig {
    fn default() -> Self {
        Self {
            ssid: String::new(),
            psk: String::new(),
            priority: 1,
        }
    }
}

// ---------------------------------------------------------------------------
// [web]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebConfig {
    pub enabled: bool,
    pub port: u16,
    /// Argon2id PHC string.  Set at first run via `--set-password`.
    pub password_hash: String,
    pub session_timeout_hours: u32,
    pub hostname: String,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 8080,
            password_hash: String::new(),
            session_timeout_hours: 8,
            hostname: "octotrack".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// [tools]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    pub mplayer: String,
    pub ffmpeg: String,
    pub nmcli: String,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            mplayer: "mplayer".to_string(),
            ffmpeg: "ffmpeg".to_string(),
            nmcli: "nmcli".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// [logging]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    /// `"error"` | `"warn"` | `"info"` | `"debug"`
    pub level: String,
    /// `""` = `/tmp/octotrack.log`
    pub log_file: String,
    pub max_size_mb: u32,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            log_file: String::new(),
            max_size_mb: 10,
        }
    }
}

// ---------------------------------------------------------------------------
// Load / save / migrate
// ---------------------------------------------------------------------------

impl Config {
    /// Load config.toml, or migrate from legacy config.json, or return defaults.
    pub fn load() -> Self {
        let toml = toml_path();
        let json = json_path();

        if toml.exists() {
            match fs::read_to_string(&toml) {
                Ok(s) => toml::from_str(&s).unwrap_or_default(),
                Err(_) => Config::default(),
            }
        } else if json.exists() {
            let cfg = migrate_from_json(&json);
            // Write new config.toml
            if let Some(parent) = toml.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = write_fresh(&cfg, &toml);
            // Rename legacy file so we don't migrate again
            let bak = json.with_extension("json.bak");
            let _ = fs::rename(&json, &bak);
            cfg
        } else {
            Config::default()
        }
    }

    /// Save the config back to disk.
    ///
    /// If `config.toml` already exists it is updated in-place via `toml_edit`
    /// so that any user-added comments are preserved.  A fresh file is written
    /// with `toml::to_string_pretty` when the file does not exist yet.
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let path = toml_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        if path.exists() {
            let existing = fs::read_to_string(&path)?;
            let mut doc: toml_edit::DocumentMut = existing.parse()?;
            update_doc(&mut doc, self);
            fs::write(&path, doc.to_string())?;
        } else {
            write_fresh(self, &path)?;
        }

        Ok(())
    }
}

/// Serialize `cfg` to a new file at `path`.
fn write_fresh(cfg: &Config, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    fs::write(path, toml::to_string_pretty(cfg)?)?;
    Ok(())
}

/// Surgically update an existing `toml_edit::DocumentMut` with all runtime-
/// writable fields from `cfg`, leaving user comments and unmanaged sections
/// (e.g. `[network]`, `[[channels]]`) untouched.
fn update_doc(doc: &mut toml_edit::DocumentMut, cfg: &Config) {
    use toml_edit::{value, Array, Item, Table};

    // Ensure a top-level table section exists, then return a mutable ref.
    // We work section by section, releasing the borrow between each block.

    // [playback] -------------------------------------------------------
    if !doc.contains_key("playback") {
        doc.insert("playback", Item::Table(Table::new()));
    }
    doc["playback"]["device"] = value(cfg.playback.device.as_str());
    doc["playback"]["channel_count"] = value(cfg.playback.channel_count as i64);
    doc["playback"]["volume"] = value(cfg.playback.volume as i64);
    doc["playback"]["max_volume"] = value(cfg.playback.max_volume as i64);
    doc["playback"]["loop_mode"] = value(cfg.playback.loop_mode.as_str());
    doc["playback"]["auto_mode"] = value(cfg.playback.auto_mode.as_str());
    doc["playback"]["start_track"] = value(cfg.playback.start_track.as_str());

    // [playback.eq] ----------------------------------------------------
    {
        // Create eq sub-table inside playback if missing.
        let has_eq = doc["playback"]
            .as_table()
            .map(|t| t.contains_key("eq"))
            .unwrap_or(false);
        if !has_eq {
            if let Some(t) = doc["playback"].as_table_mut() {
                t.insert("eq", Item::Table(Table::new()));
            }
        }
    }
    doc["playback"]["eq"]["enabled"] = value(cfg.playback.eq.enabled);
    {
        let mut arr = Array::new();
        for &b in &cfg.playback.eq.bands {
            arr.push(b as i64);
        }
        doc["playback"]["eq"]["bands"] = value(arr);
    }

    // [recording] ------------------------------------------------------
    if !doc.contains_key("recording") {
        doc.insert("recording", Item::Table(Table::new()));
    }
    doc["recording"]["input_device"] = value(cfg.recording.input_device.as_str());
    doc["recording"]["channel_count"] = value(cfg.recording.channel_count as i64);
    doc["recording"]["sample_rate"] = value(cfg.recording.sample_rate as i64);
    doc["recording"]["bit_depth"] = value(cfg.recording.bit_depth as i64);
    doc["recording"]["split_file_mb"] = value(cfg.recording.split_file_mb as i64);
    doc["recording"]["max_file_mb"] = value(cfg.recording.max_file_mb as i64);
    doc["recording"]["max_file_mode"] = value(cfg.recording.max_file_mode.as_str());
    doc["recording"]["min_free_mb"] = value(cfg.recording.min_free_mb as i64);
    doc["recording"]["filename_template"] = value(cfg.recording.filename_template.as_str());

    // [monitoring] -----------------------------------------------------
    if !doc.contains_key("monitoring") {
        doc.insert("monitoring", Item::Table(Table::new()));
    }
    doc["monitoring"]["output_device"] = value(cfg.monitoring.output_device.as_str());
    doc["monitoring"]["peak_hold_ms"] = value(cfg.monitoring.peak_hold_ms as i64);
    doc["monitoring"]["meter_decay_db_per_sec"] =
        value(cfg.monitoring.meter_decay_db_per_sec as f64);

    // [storage] --------------------------------------------------------
    if !doc.contains_key("storage") {
        doc.insert("storage", Item::Table(Table::new()));
    }
    doc["storage"]["tracks_dir"] = value(cfg.storage.tracks_dir.as_str());
    {
        let mut arr = Array::new();
        for p in &cfg.storage.usb_mount_paths {
            arr.push(p.as_str());
        }
        doc["storage"]["usb_mount_paths"] = value(arr);
    }
    doc["storage"]["recordings_subdir"] = value(cfg.storage.recordings_subdir.as_str());

    // [display] --------------------------------------------------------
    if !doc.contains_key("display") {
        doc.insert("display", Item::Table(Table::new()));
    }
    doc["display"]["mode"] = value(cfg.display.mode.as_str());
    doc["display"]["scale_factor"] = value(cfg.display.scale_factor as f64);
    doc["display"]["theme"] = value(cfg.display.theme.as_str());

    // [web] ------------------------------------------------------------
    if !doc.contains_key("web") {
        doc.insert("web", Item::Table(Table::new()));
    }
    doc["web"]["enabled"] = value(cfg.web.enabled);
    doc["web"]["port"] = value(cfg.web.port as i64);
    doc["web"]["password_hash"] = value(cfg.web.password_hash.as_str());
    doc["web"]["session_timeout_hours"] = value(cfg.web.session_timeout_hours as i64);
    doc["web"]["hostname"] = value(cfg.web.hostname.as_str());

    // [tools] ----------------------------------------------------------
    if !doc.contains_key("tools") {
        doc.insert("tools", Item::Table(Table::new()));
    }
    doc["tools"]["mplayer"] = value(cfg.tools.mplayer.as_str());
    doc["tools"]["ffmpeg"] = value(cfg.tools.ffmpeg.as_str());
    doc["tools"]["nmcli"] = value(cfg.tools.nmcli.as_str());

    // [logging] --------------------------------------------------------
    if !doc.contains_key("logging") {
        doc.insert("logging", Item::Table(Table::new()));
    }
    doc["logging"]["level"] = value(cfg.logging.level.as_str());
    doc["logging"]["log_file"] = value(cfg.logging.log_file.as_str());
    doc["logging"]["max_size_mb"] = value(cfg.logging.max_size_mb as i64);

    // [[channels]], [[network.*]], [[network.known_networks]] are user-managed
    // and intentionally NOT updated here.
}

// ---------------------------------------------------------------------------
// JSON → TOML migration
// ---------------------------------------------------------------------------

/// Read a legacy `config.json` and map its fields into a `Config`.
/// Unknown or missing fields fall back to `Config::default()`.
pub fn migrate_from_json(path: &PathBuf) -> Config {
    let mut cfg = Config::default();

    let content = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return cfg,
    };
    let v: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return cfg,
    };

    // [playback]
    if let Some(x) = v["volume"].as_u64() {
        cfg.playback.volume = x as u8;
    }
    if let Some(x) = v["max_volume"].as_u64() {
        cfg.playback.max_volume = x as u8;
    }
    if let Some(x) = v["auto_mode"].as_str() {
        cfg.playback.auto_mode = x.to_string();
    } else if let Some(autoplay) = v["autoplay"].as_bool() {
        // backwards compat with very old configs
        cfg.playback.auto_mode = if autoplay { "play" } else { "off" }.to_string();
    }
    if let Some(x) = v["loop_mode"].as_str() {
        cfg.playback.loop_mode = x.to_string();
    }
    if let Some(x) = v["start_track"].as_str() {
        cfg.playback.start_track = x.to_string();
    }
    if let Some(x) = v["playback_device"].as_str() {
        cfg.playback.device = x.to_string();
    }
    if let Some(x) = v["playback_channel_count"].as_u64() {
        cfg.playback.channel_count = x as u32;
    }

    // [playback.eq]
    if let Some(bands) = v["eq_bands"].as_array() {
        let migrated: Vec<i8> = bands
            .iter()
            .filter_map(|b| b.as_i64())
            .map(|b| (b as i8).clamp(-12, 12))
            .collect();
        if !migrated.is_empty() {
            cfg.playback.eq.bands = migrated;
        }
    }
    if let Some(x) = v["eq_enabled"].as_bool() {
        cfg.playback.eq.enabled = x;
    }

    // [recording]
    if let Some(x) = v["rec_input_device"].as_str() {
        cfg.recording.input_device = x.to_string();
    }
    if let Some(x) = v["rec_channel_count"].as_u64() {
        cfg.recording.channel_count = x as u32;
    }
    if let Some(x) = v["rec_sample_rate"].as_u64() {
        cfg.recording.sample_rate = x as u32;
    }
    if let Some(x) = v["rec_bit_depth"].as_u64() {
        let bd = x as u16;
        if bd == 16 || bd == 24 || bd == 32 {
            cfg.recording.bit_depth = bd;
        }
    }
    if let Some(x) = v["rec_max_file_mb"].as_u64() {
        cfg.recording.max_file_mb = x;
    }
    if let Some(x) = v["rec_max_file_mode"].as_str() {
        cfg.recording.max_file_mode = x.to_string();
    }
    if let Some(x) = v["rec_min_free_mb"].as_u64() {
        cfg.recording.min_free_mb = x;
    }
    if let Some(x) = v["rec_split_file_mb"].as_u64() {
        cfg.recording.split_file_mb = x;
    }

    // [monitoring]
    if let Some(x) = v["mon_output_device"].as_str() {
        cfg.monitoring.output_device = x.to_string();
    }

    cfg
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn write_json(content: &str) -> NamedTempFile {
        let f = NamedTempFile::new().unwrap();
        fs::write(f.path(), content).unwrap();
        f
    }

    #[test]
    fn migrate_maps_all_fields() {
        let json = r#"{
            "volume": 75,
            "max_volume": 90,
            "auto_mode": "rec",
            "loop_mode": "all",
            "start_track": "my_song",
            "playback_device": "hw:1,0",
            "playback_channel_count": 2,
            "eq_bands": [1, 2, 3, 4, 5, -1, -2, -3, -4, -5],
            "eq_enabled": false,
            "rec_input_device": "hw:2,0",
            "rec_channel_count": 4,
            "rec_sample_rate": 96000,
            "rec_bit_depth": 24,
            "rec_max_file_mb": 4000,
            "rec_max_file_mode": "drop",
            "rec_min_free_mb": 2048,
            "rec_split_file_mb": 3900,
            "mon_output_device": "hw:3,0"
        }"#;
        let f = write_json(json);
        let cfg = migrate_from_json(&f.path().to_path_buf());

        assert_eq!(cfg.playback.volume, 75);
        assert_eq!(cfg.playback.max_volume, 90);
        assert_eq!(cfg.playback.auto_mode, "rec");
        assert_eq!(cfg.playback.loop_mode, "all");
        assert_eq!(cfg.playback.start_track, "my_song");
        assert_eq!(cfg.playback.device, "hw:1,0");
        assert_eq!(cfg.playback.channel_count, 2);
        assert_eq!(
            cfg.playback.eq.bands,
            vec![1, 2, 3, 4, 5, -1, -2, -3, -4, -5]
        );
        assert!(!cfg.playback.eq.enabled);
        assert_eq!(cfg.recording.input_device, "hw:2,0");
        assert_eq!(cfg.recording.channel_count, 4);
        assert_eq!(cfg.recording.sample_rate, 96000);
        assert_eq!(cfg.recording.bit_depth, 24);
        assert_eq!(cfg.recording.max_file_mb, 4000);
        assert_eq!(cfg.recording.max_file_mode, "drop");
        assert_eq!(cfg.recording.min_free_mb, 2048);
        assert_eq!(cfg.recording.split_file_mb, 3900);
        assert_eq!(cfg.monitoring.output_device, "hw:3,0");
    }

    #[test]
    fn migrate_clamps_eq_bands() {
        let json = r#"{"eq_bands": [99, -99, 0, 0, 0, 0, 0, 0, 0, 0]}"#;
        let f = write_json(json);
        let cfg = migrate_from_json(&f.path().to_path_buf());
        assert_eq!(cfg.playback.eq.bands[0], 12);
        assert_eq!(cfg.playback.eq.bands[1], -12);
    }

    #[test]
    fn migrate_ignores_invalid_bit_depth() {
        let json = r#"{"rec_bit_depth": 20}"#;
        let f = write_json(json);
        let cfg = migrate_from_json(&f.path().to_path_buf());
        assert_eq!(cfg.recording.bit_depth, 32); // default preserved
    }

    #[test]
    fn migrate_partial_preserves_defaults() {
        let json = r#"{"volume": 42}"#;
        let f = write_json(json);
        let cfg = migrate_from_json(&f.path().to_path_buf());
        assert_eq!(cfg.playback.volume, 42);
        assert_eq!(cfg.recording.sample_rate, 192_000); // default preserved
    }

    #[test]
    fn migrate_backwards_compat_autoplay() {
        let json = r#"{"autoplay": true}"#;
        let f = write_json(json);
        let cfg = migrate_from_json(&f.path().to_path_buf());
        assert_eq!(cfg.playback.auto_mode, "play");
    }

    #[test]
    fn roundtrip_toml() {
        let mut original = Config::default();
        original.playback.volume = 77;
        original.recording.sample_rate = 48000;
        original.monitoring.peak_hold_ms = 2000;

        let s = toml::to_string_pretty(&original).unwrap();
        let loaded: Config = toml::from_str(&s).unwrap();

        assert_eq!(loaded.playback.volume, 77);
        assert_eq!(loaded.recording.sample_rate, 48000);
        assert_eq!(loaded.monitoring.peak_hold_ms, 2000);
    }

    #[test]
    fn update_doc_preserves_comments() {
        // toml_edit preserves block/structural comments (comment lines, table
        // header decorations) when values are updated surgically.
        // Inline trailing comments on value lines (e.g. `key = 1  # note`) are
        // dropped when the value item is replaced — this is expected toml_edit
        // behaviour and an acceptable trade-off vs. full-file rewrites.
        let toml_with_comment = r#"
# My custom comment
[playback]
volume = 50
device = "hw:0,0"
"#;
        let mut doc: toml_edit::DocumentMut = toml_with_comment.parse().unwrap();
        let mut cfg = Config::default();
        cfg.playback.volume = 99;
        update_doc(&mut doc, &cfg);

        let result = doc.to_string();
        assert!(
            result.contains("My custom comment"),
            "block comment preserved"
        );
        assert!(result.contains("volume = 99"), "value updated");
        // Verify unmanaged keys in the file are not clobbered
        assert!(result.contains("device"), "other keys still present");
    }
}
