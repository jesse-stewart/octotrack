//! SSE event types and broadcaster.

use serde::Serialize;
use tokio::sync::broadcast;

/// Events pushed over the SSE stream.
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OctoeventEvent {
    Levels {
        input: Vec<f32>,
        output: Vec<f32>,
    },
    RecordingProgress {
        path: String,
        size_bytes: u64,
        duration_secs: f32,
        files_written: u32,
    },
    TrackChanged {
        name: String,
        duration_secs: Option<f32>,
        channels: Option<u8>,
    },
    PlaybackPosition {
        position_secs: f32,
    },
    DeviceStatus {
        recording: bool,
        monitoring: bool,
        playing: bool,
        current_track: Option<String>,
    },
    NetworkStatus {
        ap_active: bool,
        ap_clients: u8,
        lan_connected: bool,
        lan_ip: Option<String>,
        lan_ssid: Option<String>,
    },
}

/// Wrapper around a `broadcast::Sender<String>` for SSE JSON payloads.
#[derive(Clone)]
pub struct SseBroadcaster {
    tx: broadcast::Sender<String>,
}

impl SseBroadcaster {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self { tx }
    }

    /// Send an event to all connected SSE clients.
    pub fn send(&self, event: &OctoeventEvent) {
        if let Ok(json) = serde_json::to_string(event) {
            // Ignore send errors (no subscribers is fine).
            let _ = self.tx.send(json);
        }
    }

    /// Subscribe to the broadcast channel.
    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }
}

impl Default for SseBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}
