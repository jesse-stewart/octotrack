# Raspberry Pi Setup

## Running on Boot

The easiest way to configure autostart is to let the setup wizard do it:

```bash
octotrack --configure-autostart
```

This is idempotent — safe to run again if you want to change the method or display type. It is also run automatically during first-run setup and when you run `--set-password`.

Two methods are supported depending on your display setup:

---

### Method 1: systemd service (recommended)

Best for TFT/framebuffer displays and headless setups. The service restarts automatically on crash.

**TFT / framebuffer on tty1:**

```ini
[Unit]
Description=Octotrack Multi-Channel Audio Player
After=sound.target multi-user.target

[Service]
Type=simple
User=jesse
WorkingDirectory=/home/jesse/octotrack
ExecStartPre=/bin/sleep 5
ExecStartPre=/bin/chvt 1
ExecStartPre=/usr/bin/clear
ExecStart=/home/jesse/octotrack/target/release/octotrack
StandardInput=tty
StandardOutput=tty
StandardError=tty
TTYPath=/dev/tty1
TTYReset=yes
TTYVHangup=yes
Environment=TERM=linux
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
```

**HDMI / headless:**

```ini
[Unit]
Description=Octotrack Multi-Channel Audio Player
After=sound.target multi-user.target

[Service]
Type=simple
User=jesse
WorkingDirectory=/home/jesse/octotrack
ExecStart=/home/jesse/octotrack/target/release/octotrack
StandardOutput=journal
StandardError=journal
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
```

**Why `StandardError=tty` for TFT:** Ratatui writes to stderr by design. If set to `journal`, all rendering output is swallowed and nothing appears on screen.

**Why `TERM=linux`:** Crossterm needs this to select the right escape sequences. Without it the terminal may not render correctly.

Install and enable manually:

```bash
sudo nano /etc/systemd/system/octotrack.service
# paste one of the blocks above, adjusted for your username/path

sudo systemctl daemon-reload
sudo systemctl enable --now octotrack
```

Managing the service:

```bash
sudo systemctl status octotrack
sudo journalctl -u octotrack -f
sudo systemctl stop octotrack
sudo systemctl disable octotrack
```

---

### Method 2: .bashrc autologin

A simpler approach that works well with HDMI displays. Octotrack launches when tty1 logs in automatically.

**Step 1 — Enable tty1 autologin:**

```bash
sudo mkdir -p /etc/systemd/system/getty@tty1.service.d
sudo tee /etc/systemd/system/getty@tty1.service.d/autologin.conf << 'EOF'
[Service]
ExecStart=
ExecStart=-/sbin/agetty --autologin jesse --noclear %I $TERM
EOF
sudo systemctl daemon-reload
```

**Step 2 — Add launch block to `~/.bashrc`:**

```bash
# octotrack autostart (begin)
if [ "$(tty)" = "/dev/tty1" ]; then
    sleep 4
    clear
    /home/jesse/octotrack/target/release/octotrack
fi
# octotrack autostart (end)
```

The `--configure-autostart` command writes and maintains this block automatically, using the markers to replace it on re-runs.

Note: this method has no automatic restart on crash. Use the systemd method if that matters.

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

Single audio files are played directly. Subdirectories are treated as multi-file tracks — each file in the folder is merged into a single multi-channel stream for playback (e.g. 3 mono files become a 3-channel track).

When a USB drive with a `tracks/` directory is mounted, Octotrack will use it automatically. If no USB drive is found, it falls back to a local `tracks/` directory.

### Auto-mounting USB drives

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
