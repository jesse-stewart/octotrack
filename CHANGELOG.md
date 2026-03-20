# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.2.3] - 2026-03-20

### Added
- `--configure-autostart` CLI flag for interactive, idempotent autostart setup
- Autostart configuration is now part of first-run setup and `--set-password`
- Supports two autostart methods: systemd service (TFT and headless) and `.bashrc` autologin (HDMI)
- API documentation (`docs/api.md`) covering all endpoints, request/response shapes, and SSE events
- Pre-built binary download instructions in `docs/installation.md`

### Fixed
- Access point `enabled` flag and all other `[network.ap]` settings were not being saved to `config.toml` on web UI save — `update_doc` now writes the full `[network.ap]` section

### Changed
- `docs/installation.md` rewritten to lead with pre-built binary download, with build-from-source as a secondary option
- `docs/raspberry-pi.md` updated to cover both systemd service (TFT and HDMI variants) and `.bashrc` autologin approaches
- `docs/architecture.md` updated to include web server components, `SharedStatus`, `SseBroadcaster`, SSE threading, and `--configure-autostart` flag

## [0.2.2] - 2026-03-19

### Added
- Web interface: dashboard, file browser, playback, recording, and settings pages
- Server-sent events (SSE) for real-time status updates in the browser
- Password-based authentication for the web interface (Argon2 hashing)
- First-run interactive setup with `--set-password` and `--reset` flags
- WiFi hotspot provisioning via `nmcli` for headless network access
- `configuration.md` documenting all config options

### Changed
- Architecture documentation updated to reflect web interface additions

### Fixed
- Clippy warnings in `web/routes.rs` (unnecessary casts, `&PathBuf` → `&Path`)
- Formatting fixes across `main.rs`

## [0.2.1] - 2026-03-18

### Added
- Test coverage for core audio and config logic
- 32-bit ARM (`armv6`) build target in CI
- ARCHITECTURE.md documenting system design

### Changed
- Config refactored into dedicated `src/config.rs` module with strongly-typed structs and TOML round-trip support

### Fixed
- Cross-platform `statvfs` type mismatch causing build failure on 32-bit ARM
- Clippy and formatting issues across `audio.rs`, `config.rs`, and `main.rs`

## [0.2.0] - 2026-03-17

### Added
- Multi-channel recording via ALSA with configurable input device
- Real-time input monitoring with level metering
- Configurable audio devices for playback, recording, and monitoring (`playback_device`, `rec_input_device`, `mon_output_device`)
- Configurable recording channel count (`rec_channel_count`)
- Configurable recording sample rate and bit depth (`rec_sample_rate`, `rec_bit_depth`)
- RF64 format support for recordings exceeding 4 GiB
- File size cap with stop or circular-buffer drop mode (`rec_max_file_mb`, `rec_max_file_mode`)
- Disk space safety margin — stops or drops when free space falls below threshold (`rec_min_free_mb`)
- Optional file splitting into fixed-size segments (`rec_split_file_mb`)
- Auto mode (`auto_mode`: `off`, `play`, `rec`) replaces separate `autoplay` flag
- Cron-based scheduling for timed record and playback (`~/.config/octotrack/schedules.json`)
- Automatic format conversion for playback compatibility across different audio interfaces
- VU meters for recording and monitoring input levels
- CONTRIBUTING.md, CODE_OF_CONDUCT.md, issue and PR templates
- CI pipeline (GitHub Actions for build and lint)
- GPL v3 license

### Changed
- Playback device is now configurable via config instead of hardcoded
- Metadata loading optimized to reduce input lag when switching tracks

## [0.1.0] - 2026-03-10

### Added
- Multi-channel audio playback (8+ channels) via mplayer
- Real-time per-channel level metering
- Support for single audio files and multi-file tracks (folders merged into multi-channel stream)
- Loop modes (Off, Single, All)
- Volume control with configurable max volume ceiling
- 10-band graphic equalizer with bypass
- Autoplay mode
- Track navigation and metadata display (artist, title from file tags)
- Progress indicator with time display
- USB storage auto-detection for tracks
- Persistent configuration (`~/.config/octotrack/config.json`)
- merge_tracks.sh script for combining mono/stereo files into multi-channel tracks
- systemd service support for running on boot
