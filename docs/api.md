# API Reference

Octotrack exposes a REST API over HTTP. All `/api/*` endpoints require a Bearer token obtained from `/auth/login`.

## Authentication

### `POST /auth/login`

No token required.

**Request:**
```json
{ "password": "yourpassword" }
```

**Response:**
```json
{ "token": "eyJ..." }
```

Also sets a `session` HTTP-only cookie for browser use.

### `POST /auth/logout`

Clears the session cookie. No request body.

---

## Using the token

Pass the token in the `Authorization` header on every `/api/*` request:

```bash
TOKEN=$(curl -s -X POST http://localhost:8080/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"password":"yourpassword"}' | jq -r .token)

curl http://localhost:8080/api/status \
  -H "Authorization: Bearer $TOKEN"
```

---

## Status

### `GET /api/status`

Returns the current application state.

**Response:**
```json
{
  "playing": false,
  "recording": false,
  "monitoring": false,
  "current_track": "song.wav",
  "position_secs": 12.4,
  "duration_secs": 180.0,
  "volume": 80,
  "input_levels": [0.1, 0.2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
  "output_levels": [0.1, 0.2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
  "recording_path": null,
  "recording_size_bytes": 0,
  "recording_duration_secs": 0.0,
  "files_written": 0,
  "tracks_dir": "/media/usb/tracks",
  "track_list": [
    {
      "name": "song.wav",
      "path": "/media/usb/tracks/song.wav",
      "size_bytes": 52428800,
      "duration_secs": 180.0,
      "channels": 8,
      "modified_secs": 1700000000
    }
  ]
}
```

`position_secs`, `duration_secs`, `current_track`, `recording_path`, `duration_secs`, and `modified_secs` may be `null`.

---

## Transport

### `POST /api/transport/play`
### `POST /api/transport/stop`
### `POST /api/transport/prev`
### `POST /api/transport/next`

No request body. Returns `200 OK`.

### `POST /api/transport/load`

Load and play a specific track by filename.

**Request:**
```json
{ "name": "song.wav" }
```

Returns `404` if the track is not found.

### `POST /api/transport/seek`

**Request:**
```json
{ "position_secs": 45.0 }
```

---

## Recording

### `POST /api/recording/start`
### `POST /api/recording/stop`

No request body. Returns `200 OK`.

---

## Files

### `GET /api/files`

List tracks with pagination.

**Query params:**
- `page` — page number, default `1`
- `per_page` — items per page, default `20`

**Response:**
```json
{
  "items": [ /* TrackEntry objects, same shape as status.track_list */ ],
  "total": 42,
  "page": 1,
  "per_page": 20,
  "pages": 3
}
```

Results are sorted by last-modified time, most recent first.

### `GET /api/files/{name}/info`

Returns a single `TrackEntry` for the named file. `404` if not found.

### `GET /api/files/{name}/detail`

Extended file info via ffprobe.

**Response:**
```json
{
  "name": "song.wav",
  "path": "/media/usb/tracks/song.wav",
  "size_bytes": 52428800,
  "duration_secs": 180.0,
  "channels": 8,
  "modified_secs": 1700000000,
  "codec": "pcm_s24le",
  "sample_rate": 48000,
  "bits_per_sample": 24,
  "bit_rate_bps": 9216000,
  "format_name": "wav",
  "format_long_name": "WAV / WAVE (Waveform Audio)"
}
```

Fields other than `name` may be `null` if ffprobe cannot read them.

### `GET /api/files/{name}/peaks`

Returns pre-computed waveform peak data for display. One array of 0.0–1.0 values per channel (up to 1000 buckets each).

**Response:**
```json
[ [0.1, 0.4, 0.8, ...], [0.2, 0.3, 0.7, ...] ]
```

Returns `202 Accepted` with body `"pending"` if peaks are still being computed — retry after a short delay. Returns `409 Conflict` if the file is currently being recorded.

### `GET /api/files/{name}`

Download the raw audio file.

### `DELETE /api/files/{name}`

Delete a file and its peaks cache. Returns `409 Conflict` if the file is currently being recorded.

---

## Waveform peaks

### `GET /api/peaks/status`

Progress of a background precompute job.

**Response:**
```json
{ "total": 10, "done": 7, "failed": 0, "pending": 3 }
```

### `POST /api/peaks/precompute`

Trigger background precomputation of peaks for all tracks that don't yet have a cache. Returns `202 Accepted`.

---

## Config

### `GET /api/config`

Returns the full config. `password_hash` and `network.ap.password` are redacted (returned as empty strings).

### `PATCH /api/config`

Deep-merge a partial config object. Only the fields you send are updated — you don't need to send the whole config.

**Examples:**

```bash
# Change volume
curl -X PATCH http://localhost:8080/api/config \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"playback":{"volume":75}}'

# Disable access point
curl -X PATCH http://localhost:8080/api/config \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"network":{"ap":{"enabled":false}}}'

# Change loop mode
curl -X PATCH http://localhost:8080/api/config \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"playback":{"loop_mode":"all"}}'
```

`password_hash` and `network.ap.password` are ignored even if sent — use the dedicated endpoints below.

### `PATCH /api/config/password`

Change the web UI password.

**Request:**
```json
{ "password": "newpassword" }
```

---

## Network

### `GET /api/network/scan`

Scan for nearby WiFi networks via `nmcli`.

**Response:**
```json
[
  { "ssid": "MyNetwork", "signal": 85 },
  { "ssid": "Neighbor", "signal": 42 }
]
```

Results are deduplicated and sorted by signal strength descending.

### `POST /api/network/connect`

Connect to a WiFi network.

**Request:**
```json
{ "ssid": "MyNetwork", "psk": "password" }
```

### `POST /api/network/ap/password`

Set the access point password.

**Request:**
```json
{ "password": "appassword" }
```

---

## Devices

### `GET /api/devices/playback`

List available ALSA playback devices.

**Response:**
```json
[
  { "name": "DAC8x", "id": "hw:0,0" },
  { "name": "bcm2835 Headphones", "id": "hw:1,0" }
]
```

### `GET /api/devices/capture`

List available ALSA capture devices. Same response shape as above.

---

## System

### `GET /api/system/info`

**Response:**
```json
{
  "hostname": "octotrack",
  "uptime_secs": 3600.5,
  "load_avg": "0.12 0.08 0.05",
  "mem_total_mb": 3884,
  "mem_available_mb": 3200,
  "disk_used_bytes": 4294967296,
  "disk_total_bytes": 64424509440
}
```

`disk_used_bytes` and `disk_total_bytes` reflect the filesystem containing the tracks directory.

### `POST /api/system/restart`

Restart the octotrack process. Stops playback and recording cleanly before re-exec. The web UI polls for the process to come back up.

### `POST /api/system/reboot`

Reboot the device (`sudo reboot`).

### `POST /api/system/shutdown`

Shut down the device (`sudo shutdown -h now`).

---

## Server-Sent Events

### `GET /api/events`

Subscribe to a real-time event stream. Uses the session cookie for auth (browsers don't support custom headers with `EventSource`).

Each message is a JSON object on the `data:` field. Event types:

**`levels`** — emitted every tick (~250 ms) while the app is running:
```json
{ "type": "levels", "input": [0.1, 0.2, ...], "output": [0.1, 0.2, ...] }
```

**`device_status`** — emitted every tick:
```json
{
  "type": "device_status",
  "recording": false,
  "monitoring": false,
  "playing": true,
  "current_track": "song.wav"
}
```

**`playback_position`** — emitted every tick while playing:
```json
{ "type": "playback_position", "position_secs": 12.4 }
```

**`recording_progress`** — emitted every tick while recording:
```json
{
  "type": "recording_progress",
  "path": "/media/usb/tracks/recordings/REC_20240101_120000.wav",
  "size_bytes": 10485760,
  "duration_secs": 5.5,
  "files_written": 1
}
```

**Example (curl):**
```bash
curl -N http://localhost:8080/api/events \
  -H "Cookie: session=$TOKEN"
```
