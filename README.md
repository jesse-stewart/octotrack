# Octotrack

A terminal-based multi-channel audio player built with Rust and Ratatui. Designed for playing back multi-track audio projects with real-time channel level metering and playback controls.

![Octotrack Screenshot](docs/image.png)

## Features

- Multi-channel audio playback (up to 8+ channels)
- Real-time per-channel level metering (dB)
- Support for single audio files or multi-file tracks (folders with multiple mono/stereo files)
- Loop modes: Off, Single, All
- Volume control with persistent settings
- Autoplay mode for automatic playback on startup
- Track navigation (previous/next)
- Progress indicator with time display
- Metadata display (artist, title from file tags)

## Installation

### Dependencies

Install required system dependencies:

```bash
sudo apt-get install mpv libmpv-dev ffmpeg
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

The app expects audio files to be in a `tracks/` directory in the same location where you run the command.

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
| `Q` or `ESC` | Quit (with confirmation dialog) |
| `Ctrl-C` | Quit (with confirmation dialog) |

When the quit confirmation dialog appears:
- Press `Y` to confirm and quit
- Press `N` or `ESC` to cancel and return to the app

### Supported Audio Formats

- WAV (`.wav`)
- FLAC (`.flac`)
- MP3 (`.mp3`)

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

## Configuration

Octotrack stores its configuration in `~/.config/octotrack/config.json`.

Settings that are saved:
- Volume level
- Autoplay preference

The configuration file is automatically created and updated when you change settings in the app.

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
├── audio.rs   → Audio playback engine (libmpv wrapper)
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

### Adding New Features

The app follows a clean separation of concerns:
- Modify [app.rs](src/app.rs) for state and application logic
- Modify [handler.rs](src/handler.rs) for new keyboard shortcuts
- Modify [ui.rs](src/ui.rs) for UI changes
- Modify [audio.rs](src/audio.rs) for audio engine changes

## Troubleshooting

### Audio not playing
- Ensure `mpv` and `libmpv-dev` are installed
- Check that audio files are in the `tracks/` directory
- Verify file formats are supported (WAV, FLAC, or MP3)
- Check system audio output is working

### Merge script fails
- Ensure `ffmpeg` is installed
- Verify all audio files have the same sample rate
- Check that files are valid audio files

### Service won't start
- Check the paths in the service file are correct
- Verify the binary exists: `ls -l target/release/octotrack`
- Check logs: `sudo journalctl -u octotrack.service -n 50`
- Ensure the user has permission to access the audio device

## License

This project is open source. See LICENSE file for details.
