# Raspberry Pi Setup

## Running on Boot

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
StandardInput=tty
StandardOutput=tty
StandardError=tty
TTYPath=/dev/tty1
Environment=TERM=linux
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
```

**Why `StandardError=tty`:** Ratatui writes to stderr by design. If this is set to `journal`, all rendering output is swallowed by the system log and nothing appears on screen.

**Why `TERM=linux`:** Crossterm needs this to know which escape sequences to use. Without it the variable is unset inside the service and the terminal may not render correctly.

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
