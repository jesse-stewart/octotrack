# Compatibility

## Tested Hardware

| Device | CPU | RAM | Notes |
|---|---|---|---|
| Raspberry Pi 5 | ARM Cortex-A76 (AArch64) | 8 GB | Primary development platform |
| Raspberry Pi Zero W v1.1 | ARM1176JZF-S (ARMv6) | 512 MB | Use `octotrack-linux-armv6` binary |

## Pre-built Binaries

| Binary | Architecture | Covers |
|---|---|---|
| `octotrack-linux-arm64` | AArch64 | Pi 4, Pi 5, Pi Zero 2 W (64-bit), modern ARM SBCs |
| `octotrack-linux-armv7` | ARMv7 hard-float | Pi 2, Pi 3 (32-bit), most 32-bit ARM SBCs |
| `octotrack-linux-armv6` | ARMv6 hard-float | Pi Zero v1 W, Pi 1 |
| `octotrack-linux-x86_64` | x86-64 | Intel/AMD desktop and server Linux |

High-channel-count playback and recording at 192 kHz has only been tested on the Pi 5. Older models may encounter throughput limits at the highest sample rates and channel counts.

## Known Compatible Audio Interfaces

| Interface | Type | In/Out | UAC | Status |
|-----------|------|--------|-----|--------|
| HiFiBerry DAC8x | HAT/I2S | 0/8 | N/A | Tested |
| HiFiBerry ADC8x | HAT/I2S | 8/0 | N/A | Tested |
| HiFiBerry Studio DAC8x | HAT/I2S | 8/8 | N/A | Untested |
| RaspiAudio 8xIN 8xOUT | HAT/I2S | 8/8 | N/A | Untested |
| Steinberg UR22mkII | USB | 2/2 | UAC 2.0 | Tested |
| Zoom F3 | USB | 2/2 | UAC 2.0 | Tested |
| Focusrite Scarlett 2i2 | USB | 2/2 | UAC 2.0 | Untested |
| Focusrite Scarlett 18i20 | USB | 18/20 | UAC 2.0 | Untested |
| Behringer UMC202HD | USB | 2/2 | UAC 1.0 | Untested |
| Behringer UMC404HD | USB | 4/4 | UAC 1.0 | Untested |
| MOTU M2 | USB | 2/2 | UAC 2.0 | Untested |
| MOTU M4 | USB | 4/4 | UAC 2.0 | Untested |
| PreSonus AudioBox USB 96 | USB | 2/2 | UAC 1.0 | Untested |
| Native Instruments Komplete Audio 6 | USB | 6/6 | UAC 2.0 | Untested |
| Audient iD4 | USB | 2/2 | UAC 2.0 | Untested |
| Audient iD14 | USB | 2/10 | UAC 2.0 | Untested |

Octotrack works with any ALSA-supported audio interface on Linux. This includes:

- **USB Audio Class (UAC) interfaces** — Most USB audio interfaces follow UAC 1.0 or UAC 2.0 and work on Linux without additional drivers. If your interface advertises "class-compliant" or "driverless" operation, it will work out of the box.
- **HAT/I2S audio boards** — Boards that connect to the Raspberry Pi's GPIO header, such as the HiFiBerry DAC8x. These typically require a device tree overlay in `/boot/config.txt`.

To verify your interface is detected, plug it in and run `aplay -l`. If it appears, it's ready to use.

**Note:** Some professional interfaces require proprietary drivers on macOS/Windows but are still UAC-compliant and work natively on Linux. Check if your interface supports "class-compliant" mode — some require a switch or firmware setting to enable it.

If you have tested an interface not listed here, please open an issue or PR to update this table.
