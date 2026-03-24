# Configuration

Octotrack stores its configuration in `~/.config/octotrack/config.toml`. The file is created automatically on first run and updated whenever settings are changed via the keyboard. Comments you add to the file are preserved on save.

If a legacy `config.json` is found on first run, it is automatically migrated to `config.toml` and renamed to `config.json.bak`.

## First-run setup

The web UI and WiFi access point are independent optional features:

- **Web UI** — serves a browser interface on the local network (LAN). If the access point is also enabled, it is reachable there too.
- **WiFi access point** — creates a dedicated `octotrack` hotspot, useful when no LAN is available (e.g. on stage). The web UI is accessible over the AP at the same address.

Setup runs **once** — when `config.toml` does not yet exist. Each feature is opt-in; answering `n` disables it and skips the password step.

```
octotrack — first run setup

  Enable web interface? [Y/n]:
  Web UI password
  Enter password:
  Confirm:

  Enable WiFi access point? [Y/n]:
  Access point password (min 8 characters)
  Enter password:
  Confirm:

  Setup complete. Starting octotrack...
  AP network : octotrack
  Web UI     : http://octotrack.local:8080
```

Once `config.toml` exists, the setup prompt never runs again automatically. Use `--set-password` to change passwords or re-enable a feature, and `--reset` to wipe passwords if you need to start over.

The Web UI password is hashed with Argon2id and stored as a PHC string in `web.password_hash`. The AP password is stored plaintext in `network.ap.password` (consistent with wpa_supplicant/nmcli behaviour).

If octotrack is started without a TTY (e.g. via systemd) before setup has been completed, it exits with an error instructing you to run `octotrack --set-password` interactively first.

### CLI flags

| Flag | Description |
|------|-------------|
| `--set-password` | Run the interactive setup prompt and exit. Use this to change passwords or re-enable a feature at any time. |
| `--reset` | Clear both password fields and exit. Run `--set-password` afterwards to reconfigure. |

## Example config

```toml
[playback]
device = "hw:0,0"
channel_count = 8
volume = 80
max_volume = 100
loop_mode = "single"
auto_mode = "off"
start_track = ""

[playback.eq]
enabled = true
bands = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]

[recording]
input_device = "hw:0,0"
channel_count = 8
sample_rate = 192000
bit_depth = 32
split_file_mb = 0
max_file_mb = 0
max_file_mode = "stop"
min_free_mb = 1024
filename_template = "REC_{timestamp}"

[monitoring]
output_device = "hw:0,0"
peak_hold_ms = 1500
meter_decay_db_per_sec = 20.0

[storage]
tracks_dir = ""
usb_mount_paths = ["/media", "/mnt"]
recordings_subdir = ""

[web]
enabled = true
port = 8080
password_hash = ""   # set via first-run prompt or --set-password
session_timeout_hours = 8
hostname = "octotrack"

[network.ap]
enabled = true
ssid = "octotrack"
password = ""        # set via first-run prompt or --set-password
channel = 6
country_code = "US"
address = "192.168.42.1"
dhcp_range_start = "192.168.42.2"
dhcp_range_end = "192.168.42.20"

[display.eink]
enabled = false
rotation = 0             # 0 / 90 / 180 / 270 degrees clockwise
dc_pin = 25
rst_pin = 17
busy_pin = 24
refresh_interval_secs = 30

[logging]
level = "info"
log_file = ""
max_size_mb = 10

[tools]
mplayer = "mplayer"
ffmpeg = "ffmpeg"
nmcli = "nmcli"
```

## `[playback]`

| Setting | Default | Description |
|---------|---------|-------------|
| `volume` | `80` | Current volume level (0–100) as a percentage of `max_volume`. Updated automatically via `↑`/`↓`. |
| `max_volume` | `100` | Volume ceiling passed to mplayer's `softvol-max`. Set to `50` to halve the maximum output level if audio is too loud at low volumes. |
| `auto_mode` | `"off"` | What to do on startup: `"off"` (do nothing), `"play"` (start playback), `"rec"` (start recording). Toggle with `A`. |
| `loop_mode` | `"single"` | Loop behaviour: `"off"` (stop after last track), `"single"` (repeat current track), `"all"` (loop through all tracks). Toggle with `L`. |
| `start_track` | `""` | Filename (or partial name) to select on startup. Case-insensitive substring match against track filenames — e.g. `"song_1"` selects the first track whose filename contains `song_1`. Defaults to the first track if empty or no match is found. |
| `device` | `"hw:0,0"` | ALSA device for audio playback. |
| `channel_count` | `8` | Number of output channels the playback device supports. |

## `[playback.eq]`

| Setting | Default | Description |
|---------|---------|-------------|
| `bands` | `[0,…,0]` | 10-band EQ gains in dB (−12 to +12) for 31 Hz, 62 Hz, 125 Hz, 250 Hz, 500 Hz, 1 kHz, 2 kHz, 4 kHz, 8 kHz, 16 kHz. |
| `enabled` | `true` | Enable/disable the EQ. Toggle with `B` inside the EQ overlay. |

## `[recording]`

| Setting | Default | Description |
|---------|---------|-------------|
| `input_device` | `"hw:0,0"` | ALSA capture device. |
| `channel_count` | `8` | Number of channels to record. |
| `sample_rate` | `192000` | Sample rate in Hz (e.g. `44100`, `48000`, `96000`, `192000`). |
| `bit_depth` | `32` | Bit depth: `16`, `24`, or `32`. |
| `max_file_mb` | `0` | Total recording size limit in MB. `0` = unlimited. |
| `max_file_mode` | `"stop"` | What happens when the size limit is hit (or when rolling a split file): `"stop"` keeps all files and stops recording; `"drop"` deletes the previous file on each split roll (dashcam) or overwrites the oldest audio in-place (circular buffer, no splitting). |
| `min_free_mb` | `1024` | Stop recording (or lock the circular-buffer size) when free disk space drops below this many MB. |
| `split_file_mb` | `0` | Split recording into multiple files of this size in MB. `0` = no splitting. See [Recording modes](../README.md#recording-modes) for the full behaviour matrix. |
| `filename_template` | `"REC_{timestamp}"` | Template for recording filenames. Supported tokens: `{timestamp}`, `{date}`, `{track}`. |

## `[monitoring]`

| Setting | Default | Description |
|---------|---------|-------------|
| `output_device` | `"hw:0,0"` | ALSA device for monitoring output. |
| `peak_hold_ms` | `1500` | How long in milliseconds to hold peak level indicators before they decay. |
| `meter_decay_db_per_sec` | `20.0` | Rate at which level meters decay in dB per second. |

## `[storage]`

| Setting | Default | Description |
|---------|---------|-------------|
| `tracks_dir` | `""` | Explicit path to the tracks directory. `""` = auto-detect USB then `./tracks`. |
| `usb_mount_paths` | `["/media", "/mnt"]` | Paths to scan for USB drives containing a `tracks/` folder. |
| `recordings_subdir` | `""` | Subdirectory under `tracks_dir` to save recordings. `""` = same directory as tracks. |

## `[web]`

| Setting | Default | Description |
|---------|---------|-------------|
| `enabled` | `true` | Enable the web interface. When enabled, the UI is served on the LAN and on the access point (if enabled). |
| `port` | `8080` | TCP port the web server listens on. |
| `password_hash` | `""` | Argon2id PHC string. Set automatically by the first-run prompt or `--set-password`. Do not edit by hand. |
| `session_timeout_hours` | `8` | How long a web session stays valid without activity. |
| `hostname` | `"octotrack"` | mDNS hostname used in the setup completion message (`http://<hostname>.local:<port>`). |

## `[network.ap]`

| Setting | Default | Description |
|---------|---------|-------------|
| `enabled` | `true` | Enable the WiFi access point. When `false`, no hotspot is created and the AP setup step is skipped. The web UI remains available on the LAN regardless of this setting. |
| `ssid` | `"octotrack"` | Access point network name. |
| `password` | `""` | WPA2 passphrase (minimum 8 characters). Set automatically by the first-run prompt or `--set-password`. Stored plaintext, consistent with wpa_supplicant/nmcli. |
| `channel` | `6` | WiFi channel (1–13 for 2.4 GHz). |
| `country_code` | `"US"` | ISO 3166-1 alpha-2 country code for regulatory compliance. |
| `address` | `"192.168.42.1"` | IP address assigned to the AP interface. |
| `dhcp_range_start` | `"192.168.42.2"` | Start of DHCP address pool. |
| `dhcp_range_end` | `"192.168.42.20"` | End of DHCP address pool. |

## `[display.eink]`

Settings for the Waveshare 2.13" SPI e-Paper HAT (V2/V3/V4 compatible). Requires SPI enabled on the Pi (`sudo raspi-config → Interface Options → SPI`) and the app started with `--eink`.

| Setting | Default | Description |
|---------|---------|-------------|
| `enabled` | `false` | Enable the e-ink display driver. |
| `rotation` | `0` | Content rotation in degrees clockwise. `0` = portrait cable-down, `90` = landscape cable-right, `180` = portrait cable-up, `270` = landscape cable-left (standard HAT orientation). |
| `dc_pin` | `25` | BCM GPIO pin for the DC (Data/Command) line. |
| `rst_pin` | `17` | BCM GPIO pin for the RST (Reset) line. |
| `busy_pin` | `24` | BCM GPIO pin for the BUSY line. |
| `refresh_interval_secs` | `30` | Minimum seconds between full display refreshes. A full refresh is also triggered immediately on track change or play/stop state change. |

### Pin connections

| HAT signal | BCM pin | Notes |
|------------|---------|-------|
| MOSI (DIN) | 10 | SPI0 hardware — not configurable |
| SCLK (CLK) | 11 | SPI0 hardware — not configurable |
| CS (CE0)   | 8  | SPI0 hardware — not configurable |
| DC         | 25 | Configurable via `dc_pin` |
| RST        | 17 | Configurable via `rst_pin` |
| BUSY       | 24 | Configurable via `busy_pin` |

### Test the hardware

Run `octotrack --test-eink` to fill the display all-black then all-white and exit. This confirms SPI/GPIO access is working before running the app normally.

## `[logging]`

| Setting | Default | Description |
|---------|---------|-------------|
| `level` | `"info"` | Log verbosity: `"error"`, `"warn"`, `"info"`, `"debug"`. |
| `log_file` | `""` | Path to log file. `""` = `/tmp/octotrack.log`. |
| `max_size_mb` | `10` | Maximum log file size in MB before rotation. |

## `[tools]`

| Setting | Default | Description |
|---------|---------|-------------|
| `mplayer` | `"mplayer"` | Path or command name for mplayer. Override if installed to a non-standard location. |
| `ffmpeg` | `"ffmpeg"` | Path or command name for ffmpeg. |
| `nmcli` | `"nmcli"` | Path or command name for nmcli (used for network management). |

## Listing audio devices

To find the correct ALSA device IDs for your audio interfaces, run:

```bash
# List playback devices
aplay -l

# List recording/capture devices
arecord -l
```

This will show all connected audio interfaces with their card and device numbers. Use the format `hw:<card>,<device>` in the config (e.g. `hw:1,0` for card 1, device 0). For example, if `aplay -l` shows:

```
card 0: DAC8x [snd_rpi_hifiberry_dac8x], device 0: ...
card 1: UR22mkII [Steinberg UR22mkII], device 0: USB Audio ...
```

Then use `hw:0,0` for the DAC8x or `hw:1,0` for the Steinberg UR22mkII.
