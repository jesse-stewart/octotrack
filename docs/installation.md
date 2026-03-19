# Installation

## Dependencies

Install Rust (if not already installed):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

Install required system dependencies:

```bash
sudo apt-get install mplayer ffmpeg alsa-utils
```

## Building

Build the release version:

```bash
cargo build --release
```

The compiled binary will be at `target/release/octotrack`.
