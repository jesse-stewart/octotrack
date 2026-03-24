# Troubleshooting

## Log file

Octotrack writes runtime logs to `/tmp/octotrack.log`. Check this file for detailed error messages when something isn't working:

```bash
tail -f /tmp/octotrack.log
```

If running via systemd, also check the journal:

```bash
journalctl -u octotrack -f
```

## Audio not playing

**Check the log first:**
```bash
cat /tmp/octotrack.log
```

**Verify audio devices are present:**
```bash
aplay -l    # playback devices
arecord -l  # capture devices
```

**Test playback directly** (replace `hw:0,0` with your device):
```bash
# mplayer
mplayer -ao alsa:device=plughw=0.0 tracks/somefile.mp3

# mpv
mpv --ao=alsa --audio-device=alsa/plughw:0,0 --no-audio-display tracks/somefile.mp3
```

**Common causes:**
- `Illegal instruction` when running mplayer — the installed `mplayer` package is ARMv7 (`armhf`) but your Pi is ARMv6 (Pi Zero / Pi 1). Set `player = "mpv"` in `[tools]` and install mpv: `sudo apt-get install mpv`
- Wrong ALSA device — set `device` under `[playback]` to match a device shown by `aplay -l` (format: `hw:<card>,<device>`)
- No tracks found — check that audio files are in the `tracks/` directory
- `auto_mode = "off"` — playback won't start automatically; set `auto_mode = "play"` or trigger via the web UI or keyboard
- Missing tools — ensure the player (`mplayer` or `mpv`) and `ffmpeg` are installed

## Player backend (mplayer vs mpv)

The default player is `mplayer`. On ARMv6 hardware (Pi Zero W, Pi 1) the Debian `mplayer` package is compiled for ARMv7 and will crash with `Illegal instruction`. Use `mpv` instead:

1. Install mpv: `sudo apt-get install mpv`
2. Test it works: `mpv --ao=alsa --audio-device=alsa/plughw:0,0 --no-audio-display tracks/somefile.mp3`
3. Set in `~/.config/octotrack/config.toml`:
   ```toml
   [tools]
   player = "mpv"
   ```

## Recording or monitoring not working

- Verify `input_device` under `[recording]` in `config.toml` matches a capture device shown by `arecord -l`
- Ensure `channel_count` under `[recording]` does not exceed the number of channels your interface supports
- Check that another application isn't already using the audio device

## E-ink display not working

**Test the hardware first:**
```bash
octotrack --test-eink
```

This fills the display all-black then all-white and prints a step-by-step log to stderr. If it hangs or prints nothing, check:

- SPI is enabled: `sudo raspi-config → Interface Options → SPI`
- SPI device exists: `ls /dev/spidev0.0`
- User is in the `spi` and `gpio` groups: `groups`
- `[display.eink] enabled = true` in `config.toml`
- Pin assignments match your HAT wiring (default: DC=25, RST=17, BUSY=24)

If `--test-eink` works but `--eink` shows nothing, check the log for errors from the eink thread.

## Merge script fails

- Ensure `ffmpeg` is installed
- Verify all audio files have the same sample rate
- Check that files are valid audio files

## Service won't start

- Check the paths in the service file are correct
- Verify the binary exists: `ls -l target/release/octotrack`
- Check logs: `sudo journalctl -u octotrack.service -n 50`
- Ensure the user has permission to access the audio device
- If setup was never completed (no `config.toml`), run `octotrack --set-password` interactively before starting the service
