# Development

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

## Debug Mode

Run in debug mode for faster compilation during development:

```bash
cargo run
```

## Running Tests

```bash
cargo test
```

For test coverage reports (requires [cargo-tarpaulin](https://github.com/xd009642/tarpaulin)):

```bash
cargo install cargo-tarpaulin
cargo tarpaulin
```

## Adding New Features

The app follows a clean separation of concerns:
- Modify [app.rs](../src/app.rs) for state and application logic
- Modify [handler.rs](../src/handler.rs) for new keyboard shortcuts
- Modify [ui.rs](../src/ui.rs) for UI changes
- Modify [audio.rs](../src/audio.rs) for audio engine changes
