# Contributing to Octotrack

Thanks for your interest in contributing to Octotrack! This guide will help you get started.

## Getting Started

### Prerequisites

- Rust (install via [rustup](https://rustup.rs/))
- mplayer
- ffmpeg
- alsa-utils (`aplay`, `arecord`)

On Debian/Ubuntu/Raspberry Pi OS:

```bash
sudo apt-get install mplayer ffmpeg alsa-utils
```

### Setting Up the Dev Environment

1. Fork and clone the repo
2. Build in debug mode for faster compilation:

```bash
cargo build
```

3. Place some audio files in a `tracks/` directory at the project root
4. Run the app:

```bash
cargo run
```

### Project Structure

```
src/
├── app.rs     → Application state, config, playback logic
├── audio.rs   → Audio engine (mplayer, ffmpeg, ALSA)
├── bigtext.rs → Large text rendering for titles
├── event.rs   → Terminal event handling
├── handler.rs → Keyboard event handlers
├── lib.rs     → Module definitions
├── main.rs    → Entry point, USB detection, track loading
├── tui.rs     → Terminal interface initialization
└── ui.rs      → UI rendering and widgets
```

## Making Changes

### Branching

1. Create a branch from `main` for your changes
2. Use a descriptive branch name (e.g. `fix/playback-lag`, `feat/midi-support`)

### Code Style

- Follow standard Rust conventions (`cargo fmt` and `cargo clippy`)
- Keep changes focused — one feature or fix per PR
- Add logging for new audio functionality (see `log()` in `audio.rs`)

### Testing

Octotrack is a hardware-dependent audio application, so manual testing is important:

- Test with at least one audio interface if possible
- Verify playback, recording, and monitoring still work after your changes
- Check the log file at `/tmp/octotrack.log` for errors

### Submitting a Pull Request

1. Push your branch to your fork
2. Open a PR against `main`
3. Fill out the PR template
4. Describe what you changed and why
5. Note what hardware/setup you tested with

## Reporting Bugs

- Use the **Bug Report** issue template
- Include your hardware setup (audio interface, Pi model, OS)
- Attach relevant lines from `/tmp/octotrack.log`

## Suggesting Features

- Use the **Feature Request** issue template
- Describe the use case, not just the solution

## Hardware Testing

One of the most valuable contributions is testing Octotrack with different audio interfaces. If you've verified compatibility with an interface not listed in the README, please open a PR to update the compatibility table.

## License

By contributing, you agree that your contributions will be licensed under the GNU General Public License v3.0.
