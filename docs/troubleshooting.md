# Troubleshooting

## Log file

Octotrack writes runtime logs to `/tmp/octotrack.log`. Check this file for detailed error messages when something isn't working:

```bash
tail -f /tmp/octotrack.log
```

## Audio not playing
- Ensure `mplayer`, `ffmpeg`, and `alsa-utils` are installed
- Check that audio files are in the `tracks/` directory
- Verify file formats are supported (WAV, FLAC, or MP3)
- Verify `device` under `[playback]` in `config.toml` matches a device shown by `aplay -l`
- Check `/tmp/octotrack.log` for mplayer error output

## Recording or monitoring not working
- Verify `input_device` under `[recording]` in `config.toml` matches a capture device shown by `arecord -l`
- Ensure `channel_count` under `[recording]` does not exceed the number of channels your interface supports
- Check that another application isn't already using the audio device

## Merge script fails
- Ensure `ffmpeg` is installed
- Verify all audio files have the same sample rate
- Check that files are valid audio files

## Service won't start
- Check the paths in the service file are correct
- Verify the binary exists: `ls -l target/release/octotrack`
- Check logs: `sudo journalctl -u octotrack.service -n 50`
- Ensure the user has permission to access the audio device
