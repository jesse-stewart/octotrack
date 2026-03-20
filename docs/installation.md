# Installation

## Option 1: Download a pre-built binary (recommended)

Pre-built binaries are published with each release. No Rust toolchain required.

**Step 1 — Pick the right binary for your hardware:**

| Binary | Use for |
|--------|---------|
| `octotrack-linux-arm64` | Raspberry Pi 4, Pi 5, Pi Zero 2 W (64-bit OS) |
| `octotrack-linux-armv7` | Raspberry Pi 2, Pi 3 (32-bit OS) |
| `octotrack-linux-armv6` | Raspberry Pi Zero v1 W, Pi 1 |
| `octotrack-linux-x86_64` | Intel / AMD Linux desktop or server |

Not sure which Pi you have? Run `uname -m`:
- `aarch64` → arm64
- `armv7l` → armv7
- `armv6l` → armv6

**Step 2 — Download and install:**

```bash
# Set these for your version and architecture
VERSION="v0.2.2"
ARCH="arm64"   # arm64 | armv7 | armv6 | x86_64

curl -L -o octotrack \
  "https://github.com/jesse-stewart/octotrack/releases/download/${VERSION}/octotrack-linux-${ARCH}"

chmod +x octotrack
sudo mv octotrack /usr/local/bin/octotrack
```

Or to always get the latest release in one shot:

```bash
ARCH="arm64"   # change as needed

curl -L -o octotrack \
  "$(curl -s https://api.github.com/repos/jesse-stewart/octotrack/releases/latest \
    | grep "browser_download_url.*linux-${ARCH}" \
    | cut -d '"' -f 4)"

chmod +x octotrack
sudo mv octotrack /usr/local/bin/octotrack
```

**Step 3 — Install runtime dependencies:**

```bash
sudo apt-get install mplayer ffmpeg alsa-utils
```

**Step 4 — Run first-time setup:**

```bash
octotrack --set-password
```

This walks you through setting your web UI password, access point password, and autostart configuration. You can re-run it any time to change these settings. To reconfigure autostart independently, run `octotrack --configure-autostart`.

---

## Option 2: Build from source

Install Rust if you don't have it:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

Install build and runtime dependencies:

```bash
sudo apt-get install mplayer ffmpeg alsa-utils libasound2-dev
```

Clone and build:

```bash
git clone https://github.com/jesse-stewart/octotrack
cd octotrack
cargo build --release
```

The binary will be at `target/release/octotrack`. Run first-time setup:

```bash
./target/release/octotrack --set-password
```

This covers passwords and autostart configuration.

---

## Updating

To update to a newer release, repeat Step 2 from Option 1 with the new version number, then restart the service:

```bash
sudo systemctl restart octotrack
```

Or if you built from source:

```bash
git pull
cargo build --release
sudo systemctl restart octotrack
```
