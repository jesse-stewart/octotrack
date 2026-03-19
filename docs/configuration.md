# Configuration

Octotrack stores its configuration in `~/.config/octotrack/config.toml`. The file is created automatically on first run and updated whenever settings are changed via the keyboard. Comments you add to the file are preserved on save.

If a legacy `config.json` is found on first run, it is automatically migrated to `config.toml` and renamed to `config.json.bak`.

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

[monitoring]
output_device = "hw:0,0"
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

## `[monitoring]`

| Setting | Default | Description |
|---------|---------|-------------|
| `output_device` | `"hw:0,0"` | ALSA device for monitoring output. |

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
