# Octotrack

A terminal-based multi-channel audio player built with Rust and Ratatui. Designed for playing back multi-track audio projects with real-time channel level metering and playback controls.

![Octotrack Screenshot](docs/image.png)

## Features

- Multi-channel audio playback (up to 8+ channels)
- Real-time per-channel level metering (dB)
- Support for single audio files or multi-file tracks (folders with multiple mono/stereo files)
- Loop modes: Off, Single, All
- Volume control with persistent settings
- 10-band graphic equalizer with bypass
- Autoplay mode for automatic playback on startup
- Track navigation (previous/next)
- Progress indicator with time display
- Metadata display (artist, title from file tags)
- Multi-channel recording via ALSA (configurable device, channel count, sample rate, bit depth)
- Configurable recording limits: stop at a file size, circular-buffer overwrite, or split into multiple files
- Dashcam mode: rolling file splits that delete the previous file, keeping only the current recording on disk
- Real-time input monitoring with level metering
- Configurable audio devices for playback, recording, and monitoring

## Platform Support

Octotrack is **Linux-only**. It relies on ALSA, mplayer, and ffmpeg, which are Linux-specific. macOS and Windows are not supported.

Only the **Raspberry Pi 5** has been tested. The Pi 5's improved I/O bandwidth may be required for reliable 8-channel audio — older Pi models have not been verified and may not handle the throughput needed for high-channel-count playback and recording at 192kHz.

## Installation

### Dependencies

Install Rust (if not already installed):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

Install required system dependencies:

```bash
sudo apt-get install mplayer ffmpeg alsa-utils
```

### Building

Build the release version:

```bash
cargo build --release
```

The compiled binary will be at `target/release/octotrack`.

## Usage

### Running the App

```bash
cargo run --release
```

Or run the compiled binary directly:

```bash
./target/release/octotrack
```

The app looks for a `tracks/` directory in the following order:

1. **USB storage** - scans `/media/` and `/mnt/` for any mounted drive containing a `tracks/` folder
2. **Local directory** - falls back to a `tracks/` folder in the current working directory

### Keyboard Controls

| Key | Action |
|-----|--------|
| `Space` | Play/Resume playback |
| `S` | Stop playback |
| `←` | Previous track |
| `→` | Next track |
| `↑` | Increase volume |
| `↓` | Decrease volume |
| `L` | Toggle loop mode (Off → Single → All) |
| `A` | Toggle autoplay on startup |
| `R` | Toggle recording |
| `M` | Toggle input monitoring |
| `E` | Open equalizer |
| `Q` or `ESC` | Quit (with confirmation dialog) |
| `Ctrl-C` | Quit (with confirmation dialog) |
| `Ctrl-S` | Save current settings to config file |

When the quit confirmation dialog appears:
- Press `Y` to confirm and quit
- Press `N` or `ESC` to cancel and return to the app

### Equalizer Controls

Press `E` to open the 10-band graphic equalizer overlay:

| Key | Action |
|-----|--------|
| `←` | Select previous band |
| `→` | Select next band |
| `↑` | Increase selected band (+1 dB) |
| `↓` | Decrease selected band (-1 dB) |
| `B` | Toggle EQ bypass (on/off) |
| `E` or `ESC` | Close equalizer |

EQ bands: 31Hz, 62Hz, 125Hz, 250Hz, 500Hz, 1kHz, 2kHz, 4kHz, 8kHz, 16kHz — each adjustable from -12 to +12 dB.

### Supported Audio Formats

- WAV (`.wav`)
- FLAC (`.flac`)
- MP3 (`.mp3`)

## Recording

Press `R` to start recording from the configured input device. Recordings are saved as WAV files in the current tracks directory.

- **Format:** configurable bit depth (16, 24, or 32-bit PCM) at a configurable sample rate
- **Channels:** set by `rec_channel_count` in the config
- **Filename:** `REC_<timestamp>.wav` — or `REC_<timestamp>_001.wav`, `_002.wav`, … when file splitting is enabled

Press `R` again to stop. The new recording appears in your track list automatically.

Press `M` to toggle input monitoring — this routes audio from the input device to the monitoring output in real-time with level metering, so you can hear what's coming in. Monitoring can be active independently or while recording.

**Crash protection:** Recordings are protected against crashes and power loss. The WAV header is flushed to disk every ~10 seconds during recording, so at most a few seconds of metadata is lost on an unexpected shutdown. Additionally, when you play a recording, the header is automatically checked and repaired if the file is larger than what the header claims — so a file left mid-recording will play back correctly with no manual intervention.

**Note:** Playback is automatically stopped when monitoring or recording starts, as the audio device may not support simultaneous playback and capture.

### Recording modes

Three settings work together to control what happens as a recording grows: `rec_split_file_mb` sets the per-file size, `rec_max_file_mode` controls what to do when limits are hit, and `rec_max_file_mb` sets the total size cap.

#### Without splitting (`rec_split_file_mb: 0`)

| `rec_max_file_mode` | `rec_max_file_mb` | Behaviour |
|---|---|---|
| `"stop"` | `0` | Record a single file until you press stop |
| `"stop"` | `4000` | Record a single file, stop automatically at 4000 MB |
| `"drop"` | `4000` | Circular buffer: keep recording forever into a single 4000 MB file, overwriting the oldest audio as new audio arrives |

#### With splitting (`rec_split_file_mb: 3900`)

Files are named `REC_<timestamp>_001.wav`, `_002.wav`, … A new file is opened automatically each time the current one reaches `rec_split_file_mb`.

| `rec_max_file_mode` | `rec_max_file_mb` | Behaviour |
|---|---|---|
| `"stop"` | `0` | Split into files ≤ 3900 MB, keep all of them, record until you press stop |
| `"stop"` | `20000` | Split into files ≤ 3900 MB, keep all of them, stop automatically once the total reaches 20000 MB |
| `"drop"` | `20000` | **Rolling window:** split into files ≤ 3900 MB and keep at most `20000 / 3900 ≈ 5` files on disk. When the 6th file starts, the 1st is deleted — the disk always holds ~20000 MB of the most recent audio |
| `"drop"` | `0` | Split into files ≤ 3900 MB, keep all of them indefinitely (same as `"stop"` with no limit) |

**Why use splitting?** Standard WAV has a 4 GB data limit per file. Splitting at 3900 MB keeps every file safely under that limit. It also protects against data loss from file corruption — if a file is damaged, only that segment is affected. RF64 is supported for single-file recordings that exceed 4 GB, but most DAWs handle split files more reliably.

**Rolling window use case:** A monitoring system that should run indefinitely without filling the disk. Set `rec_split_file_mb` to a convenient chunk size (e.g. 3900 MB ≈ ~30 minutes at 8ch/192kHz/32bit), set `rec_max_file_mb` to however much total disk you want to use, and set `rec_max_file_mode` to `"drop"`. The recorder writes new files and deletes old ones automatically — you always have the most recent N minutes on disk.

## Preparing Multi-Channel Tracks with merge_tracks.sh

The `merge_tracks.sh` script helps you combine multiple mono or stereo audio files into a single multi-channel file.

### Setup

1. Create a `merge/` directory in the project root
2. Create subdirectories inside `merge/` - each subdirectory represents one track
3. Place your audio files in each subdirectory

Example structure:
```
merge/
├── song_1/
│   ├── kick.wav
│   ├── snare.wav
│   ├── hihat.wav
│   ├── bass.wav
│   ├── guitar_L.wav
│   ├── guitar_R.wav
│   ├── vocal_L.wav
│   └── vocal_R.wav
└── song_2/
    ├── drums.wav
    ├── bass.wav
    ├── keys_L.wav
    └── keys_R.wav
```

### Running the Merge Script

```bash
chmod +x merge_tracks.sh
./merge_tracks.sh
```

The script will:
- Process each subdirectory in `merge/`
- Combine all audio files in each folder into a single multi-channel file
- Output the merged files to `tracks/` directory
- Preserve the format (WAV or FLAC) from the input files
- Show a summary of successful, skipped, and failed merges

**Notes:**
- Files are merged in alphabetical order
- Each subdirectory must contain at least 2 audio files
- All files in a folder should have the same sample rate and bit depth
- The output will have as many channels as the sum of all input files

### Adding Metadata

To add artist and title metadata to your tracks, use `ffmpeg`:

```bash
ffmpeg -i input.wav -metadata artist="Artist Name" -metadata title="Track Title" -codec copy output.wav
```

## Scheduled Tasks

Octotrack can start and stop playback or recording automatically on a cron-style schedule. Schedules are stored in `~/.config/octotrack/schedules.json` — create this file manually (it is not generated automatically).

### Schedule file format

```json
[
  {
    "cron": "0 1 * * *",
    "action": "rec",
    "duration_minutes": 60
  },
  {
    "cron": "0 22 * * 1-5",
    "action": "play",
    "duration_minutes": 120,
    "start_track": "evening_set"
  }
]
```

Any number of entries can be added to the array. Each entry fires independently — two entries with the same action at the same time will both fire.

Each entry supports the following fields:

| Field | Required | Description |
|---|---|---|
| `cron` | yes | 5-field cron expression: `minute hour day-of-month month day-of-week` |
| `action` | yes | `"rec"` to start/stop recording, `"play"` to start/stop playback |
| `duration_minutes` | one of these | How long to run before stopping automatically |
| `duration_seconds` | one of these | Same as above, for sub-minute precision |
| `start_track` | no | Filename (or partial name) to select before starting playback. Case-insensitive substring match — e.g. `"evening_set"` selects the first track whose filename contains that string. Ignored for `"rec"` actions. |

### Cron expression format

Standard 5-field cron — the same format used by Unix cron:

```
┌─ minute       (0-59)
│ ┌─ hour        (0-23)
│ │ ┌─ day        (1-31)
│ │ │ ┌─ month     (1-12)
│ │ │ │ ┌─ weekday  (0-6, 0=Sunday)
│ │ │ │ │
* * * * *
```

Each field supports:

| Syntax | Meaning | Example |
|---|---|---|
| `*` | Every value | `*` in hours = every hour |
| `N` | Specific value | `5` in minutes = at :05 |
| `N-M` | Range | `1-5` in weekdays = Mon–Fri |
| `*/N` | Every Nth | `*/2` in hours = every 2 hours |
| `N-M/N` | Range with step | `0-20/2` in hours = 0,2,4,…,20 |
| `N,M,…` | List | `0,15,30,45` in minutes = every 15 min |

**Examples:**

| Expression | Meaning |
|---|---|
| `0 1 * * *` | Every day at 01:00 |
| `0 22 * * 1-5` | Weekdays at 22:00 |
| `23 0-20/2 * * *` | Minute :23 past every 2nd hour, midnight through 20:00 |
| `0 9 * * 1` | Every Monday at 09:00 |
| `0 8,20 * * *` | Every day at 08:00 and 20:00 |
| `0 0 25 12 *` | December 25th at midnight (repeats yearly) |

**Specific date/time:** Standard cron doesn't have a year field, so you can't schedule a truly one-off event. The closest you can get is being precise about day and month (e.g. `0 14 25 12 *` fires at 14:00 every December 25th). To run something once at a specific time, set the schedule, let it fire, then remove the entry from `schedules.json`.

### How schedules interact with `auto_mode`

The `auto_mode` setting in `config.json` controls what happens at **startup** only. Scheduled tasks are independent — `action: "rec"` always starts recording regardless of what `auto_mode` is set to.

Multiple schedules can be active at once. If two schedules overlap, both will fire independently.

## Configuration

Octotrack stores its configuration in `~/.config/octotrack/config.json`. The file is created automatically on first run and updated whenever settings are changed via the keyboard.

Example config:

```json
{
  "volume": 80,
  "max_volume": 100,
  "auto_mode": "off",
  "eq_bands": [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  "eq_enabled": true,
  "playback_device": "hw:0,0",
  "playback_channel_count": 8,
  "rec_input_device": "hw:0,0",
  "rec_channel_count": 8,
  "rec_sample_rate": 192000,
  "rec_bit_depth": 32,
  "rec_max_file_mb": 0,
  "rec_max_file_mode": "stop",
  "rec_min_free_mb": 1024,
  "rec_split_file_mb": 0,
  "mon_output_device": "hw:0,0"
}
```

### Playback

| Setting | Default | Description |
|---------|---------|-------------|
| `volume` | `100` | Current volume level (0–100) as a percentage of `max_volume`. Updated automatically via `↑`/`↓`. |
| `max_volume` | `100` | Volume ceiling passed to mplayer's `softvol-max`. Set to `50` to halve the maximum output level if audio is too loud at low volumes. |
| `auto_mode` | `"off"` | What to do on startup: `"off"` (do nothing), `"play"` (start playback), `"rec"` (start recording). Toggle with `A`. |
| `loop_mode` | `"single"` | Loop behaviour: `"off"` (stop after last track), `"single"` (repeat current track), `"all"` (loop through all tracks). Toggle with `L`. |
| `start_track` | `""` | Filename (or partial name) to select on startup. Case-insensitive substring match against track filenames — e.g. `"song_1"` selects the first track whose filename contains `song_1`. Defaults to the first track if empty or no match is found. |
| `eq_bands` | `[0,…,0]` | 10-band EQ gains in dB (−12 to +12) for 31 Hz, 62 Hz, 125 Hz, 250 Hz, 500 Hz, 1 kHz, 2 kHz, 4 kHz, 8 kHz, 16 kHz. |
| `eq_enabled` | `true` | Enable/disable the EQ. Toggle with `B` inside the EQ overlay. |
| `playback_device` | `"hw:0,0"` | ALSA device for audio playback. |
| `playback_channel_count` | `8` | Number of output channels the playback device supports. |

### Recording

| Setting | Default | Description |
|---------|---------|-------------|
| `rec_input_device` | `"hw:0,0"` | ALSA capture device. |
| `rec_channel_count` | `8` | Number of channels to record. |
| `rec_sample_rate` | `192000` | Sample rate in Hz (e.g. `44100`, `48000`, `96000`, `192000`). |
| `rec_bit_depth` | `32` | Bit depth: `16`, `24`, or `32`. |
| `rec_max_file_mb` | `0` | Total recording size limit in MB. `0` = unlimited. |
| `rec_max_file_mode` | `"stop"` | What happens when the size limit is hit (or when rolling a split file): `"stop"` keeps all files and stops recording; `"drop"` deletes the previous file on each split roll (dashcam) or overwrites the oldest audio in-place (circular buffer, no splitting). |
| `rec_min_free_mb` | `1024` | Stop recording (or lock the circular-buffer size) when free disk space drops below this many MB. |
| `rec_split_file_mb` | `0` | Split recording into multiple files of this size in MB. `0` = no splitting. See [Recording modes](#recording-modes) for the full behaviour matrix. |
| `mon_output_device` | `"hw:0,0"` | ALSA device for monitoring output. |

### Listing Audio Devices

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

### Compatible Audio Interfaces

Octotrack works with any audio interface that is supported by ALSA on Linux. This includes:

- **USB Audio Class (UAC) interfaces** — Most USB audio interfaces follow the USB Audio Class standard (UAC 1.0 or UAC 2.0), which means they work on Linux without any additional drivers. These are "class-compliant" devices. If a USB audio interface advertises "class-compliant" or "driverless" operation, it will work with Octotrack out of the box.
- **HAT/I2S audio boards** — Boards that connect directly to the Raspberry Pi's GPIO header, such as the HiFiBerry DAC8x. These typically require a device tree overlay to be enabled in `/boot/config.txt`.

#### Known Compatible Interfaces

| Interface | Type | In/Out | UAC | Status |
|-----------|------|--------|-----|--------|
| HiFiBerry DAC8x | HAT/I2S | 0/8 | N/A | Tested |
| HiFiBerry ADC8x | HAT/I2S | 8/0 | N/A | Tested |
| HiFiBerry Studio DAC8x | HAT/I2S | 8/8 | N/A | Untested |
| RaspiAudio 8xIN 8xOUT | HAT/I2S | 8/8 | N/A | Untested |
| Steinberg UR22mkII | USB | 2/2 | UAC 2.0 | Tested |
| Focusrite Scarlett 2i2 | USB | 2/2 | UAC 2.0 | Untested |
| Focusrite Scarlett 18i20 | USB | 18/20 | UAC 2.0 | Untested |
| Behringer UMC202HD | USB | 2/2 | UAC 1.0 | Untested |
| Behringer UMC404HD | USB | 4/4 | UAC 1.0 | Untested |
| MOTU M2 | USB | 2/2 | UAC 2.0 | Untested |
| MOTU M4 | USB | 4/4 | UAC 2.0 | Untested |
| PreSonus AudioBox USB 96 | USB | 2/2 | UAC 1.0 | Untested |
| Native Instruments Komplete Audio 6 | USB | 6/6 | UAC 2.0 | Untested |
| Audient iD4 | USB | 2/2 | UAC 2.0 | Untested |
| Audient iD14 | USB | 2/10 | UAC 2.0 | Untested |

**Note:** Any USB audio interface that is UAC class-compliant should work without additional drivers on Linux. Some professional interfaces require proprietary drivers on macOS/Windows but are still UAC-compliant and work natively on Linux. Check if your interface supports "class-compliant" mode — some require a switch or firmware setting to enable it.

To verify your interface is detected, plug it in and run `aplay -l`. If it appears in the list, it's ready to use. If you have tested an interface not listed here, please open an issue or PR to update this table.

## USB Storage

Octotrack automatically detects tracks on USB drives. Place your audio files in a `tracks/` folder at the root of any USB drive:

```
USB Drive (e.g. /media/pi/MYUSB)
└── tracks/
    ├── song_1.wav
    ├── song_2.flac
    └── multi_track_folder/
        ├── kick.wav
        ├── snare.wav
        └── bass.wav
```

Single audio files are played directly. Subdirectories are treated as multi-file tracks - each file in the folder is merged into a single multi-channel stream for playback (e.g. 3 mono files become a 3-channel track).

When a USB drive with a `tracks/` directory is mounted, Octotrack will use it automatically. If no USB drive is found, it falls back to a local `tracks/` directory.

### Auto-mounting USB drives on Raspberry Pi

By default, USB drives don't auto-mount on a headless Raspberry Pi. Use `usbmount` to mount USB drives automatically:

```bash
sudo apt-get install usbmount
```

Then edit the usbmount config to support common filesystems:

```bash
sudo nano /etc/usbmount/usbmount.conf
```

Set the following:

```
FILESYSTEMS="vfat ext2 ext3 ext4 hfsplus ntfs exfat"
MOUNTOPTIONS="sync,noexec,nodev,noatime,nodiratime"
```

For `exfat` and `ntfs` support, install the additional packages:

```bash
sudo apt-get install exfat-fuse ntfs-3g
```

USB drives will now auto-mount under `/media/usb0`, `/media/usb1`, etc. Octotrack scans these paths on startup.

**Note:** If you are using a desktop environment (e.g. Raspberry Pi OS with desktop), USB drives typically auto-mount under `/media/<username>/` already and no extra setup is needed.

## Running on Boot (Linux/Raspberry Pi)

To automatically start Octotrack when your system boots, create a systemd service.

### 1. Create a systemd service file

```bash
sudo nano /etc/systemd/system/octotrack.service
```

### 2. Add the following content

Replace `/home/yourusername` with your actual home directory path:

```ini
[Unit]
Description=Octotrack Multi-Channel Audio Player
After=sound.target

[Service]
Type=simple
User=yourusername
WorkingDirectory=/home/yourusername/octotrack
ExecStart=/home/yourusername/octotrack/target/release/octotrack
StandardOutput=journal
StandardError=journal
Restart=always
RestartSec=3

# Optional: Set environment variables if needed
Environment="DISPLAY=:0"

[Install]
WantedBy=multi-user.target
```

### 3. Enable and start the service

```bash
# Reload systemd to recognize the new service
sudo systemctl daemon-reload

# Enable the service to start on boot
sudo systemctl enable octotrack.service

# Start the service now
sudo systemctl start octotrack.service
```

### 4. Managing the service

```bash
# Check status
sudo systemctl status octotrack.service

# View logs
sudo journalctl -u octotrack.service -f

# Stop the service
sudo systemctl stop octotrack.service

# Disable autostart
sudo systemctl disable octotrack.service
```

**Note:** For headless operation (without a display), you may need to configure the terminal differently or run it through a virtual terminal. The service file above assumes you want it to run in the background with output going to the system journal.

## Project Structure

```text
src/
├── app.rs     → Application state and logic
├── audio.rs   → Audio engine (mplayer, ffmpeg, ALSA)
├── bigtext.rs → Large text rendering for titles
├── event.rs   → Terminal event handling
├── handler.rs → Keyboard event handlers
├── lib.rs     → Module definitions
├── main.rs    → Entry point
├── tui.rs     → Terminal interface initialization
└── ui.rs      → UI rendering and widgets
```

## Development

### Debug Mode

Run in debug mode for faster compilation during development:

```bash
cargo run
```

### Running Tests

```bash
cargo test
```

For test coverage reports (requires [cargo-tarpaulin](https://github.com/xd009642/tarpaulin)):

```bash
cargo install cargo-tarpaulin
cargo tarpaulin
```

### Adding New Features

The app follows a clean separation of concerns:
- Modify [app.rs](src/app.rs) for state and application logic
- Modify [handler.rs](src/handler.rs) for new keyboard shortcuts
- Modify [ui.rs](src/ui.rs) for UI changes
- Modify [audio.rs](src/audio.rs) for audio engine changes

## Troubleshooting

### Log file

Octotrack writes runtime logs to `/tmp/octotrack.log`. Check this file for detailed error messages when something isn't working:

```bash
tail -f /tmp/octotrack.log
```

### Audio not playing
- Ensure `mplayer`, `ffmpeg`, and `alsa-utils` are installed
- Check that audio files are in the `tracks/` directory
- Verify file formats are supported (WAV, FLAC, or MP3)
- Verify the `playback_device` in the config matches a device shown by `aplay -l`
- Check `/tmp/octotrack.log` for mplayer error output

### Recording or monitoring not working
- Verify the `rec_input_device` in the config matches a capture device shown by `arecord -l`
- Ensure `rec_channel_count` does not exceed the number of channels your interface supports
- Check that another application isn't already using the audio device

### Merge script fails
- Ensure `ffmpeg` is installed
- Verify all audio files have the same sample rate
- Check that files are valid audio files

### Service won't start
- Check the paths in the service file are correct
- Verify the binary exists: `ls -l target/release/octotrack`
- Check logs: `sudo journalctl -u octotrack.service -n 50`
- Ensure the user has permission to access the audio device

## Support This Project

If you find Octotrack useful, the best way to support it is to star this repo and share it with others.

The biggest challenge for this project right now is **hardware access for testing**. We need to verify compatibility across a wider range of audio interfaces and create multi-channel demo content. If you have any of the hardware listed below and would be willing to loan or donate it for testing, please open an issue or reach out at jesse@jessestewart.com.

### Hardware needed for interface testing

- 8+ channel USB 3.0 audio interface (UAC class-compliant)
- 8 channel USB 2.0 audio interface (UAC class-compliant)
- RaspiAudio 8xIN 8xOUT HAT

### Hardware needed for demo content

To create 8-channel ORTF-3D surround field recordings (4.0 Lo + 4.0 Hi) for sample tracks and demo videos:

- 8x Sonorous Objects SO.4 or SO.104 ultrasonic omni microphones
- 8 channel discrete microphone preamp

## Author

**Jesse Stewart** — [GitHub](https://github.com/jesse-stewart) · [jesse@jessestewart.com](mailto:jesse@jessestewart.com)

## License

This project is licensed under the GNU General Public License v3.0. See [LICENSE](LICENSE) for details.
