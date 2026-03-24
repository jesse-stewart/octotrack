//! Waveshare 2.13-inch e-Paper HAT (V2/V3/V4) display driver.
//!
//! Drives the display via SPI0 using rppal (Raspberry Pi only).
//! The display is 122 × 250 pixels (portrait) with 1-bit monochrome colour.
//!
//! Default pin connections (BCM numbering, configurable in config.toml):
//!
//! | Signal | Default BCM | Notes                        |
//! |--------|-------------|------------------------------|
//! | MOSI   | 10          | SPI0 hardware (not configurable) |
//! | SCLK   | 11          | SPI0 hardware (not configurable) |
//! | CS/CE0 | 8           | SPI0 CE0   (not configurable) |
//! | DC     | 25          | display.eink.dc_pin          |
//! | RST    | 17          | display.eink.rst_pin         |
//! | BUSY   | 24          | display.eink.busy_pin        |
//!
//! Enable SPI on the Pi: `sudo raspi-config → Interfacing Options → SPI`

use crate::config::EinkConfig;
use crate::web::SharedStatus;
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, ascii::FONT_9X15_BOLD, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::{Baseline, Text},
    Pixel,
};
use rppal::gpio::{Gpio, InputPin, OutputPin};
use rppal::spi::{Bus, Mode, SlaveSelect, Spi};
use std::convert::Infallible;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, RwLock,
};
use std::thread;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Display geometry constants
// ---------------------------------------------------------------------------

/// Pixel width (portrait orientation — shorter dimension).
const WIDTH: usize = 122;
/// Pixel height (portrait orientation — longer dimension).
const HEIGHT: usize = 250;
/// Bytes per scan row — rounded up to the next byte boundary from 122 pixels.
/// The hardware RAM X window spans bytes 0..15 (128 bits).
const BYTES_PER_ROW: usize = 16;
/// Total byte count of a complete frame buffer.
const BUFFER_SIZE: usize = BYTES_PER_ROW * HEIGHT; // 4 000

// ---------------------------------------------------------------------------
// Framebuffer
// ---------------------------------------------------------------------------

/// A 1-bpp monochrome frame buffer matching the e-ink panel RAM layout.
///
/// Each bit represents one pixel:
/// - `1` → white (no ink)
/// - `0` → black (ink)
///
/// Pixel `(x, y)` occupies bit `7 − (x % 8)` of byte `y * BYTES_PER_ROW + x / 8`.
struct Framebuffer {
    data: [u8; BUFFER_SIZE],
}

impl Framebuffer {
    fn new() -> Self {
        Self {
            data: [0xFF; BUFFER_SIZE],
        } // start all-white
    }

    fn set_pixel(&mut self, x: usize, y: usize, black: bool) {
        if x >= WIDTH || y >= HEIGHT {
            return;
        }
        let byte = y * BYTES_PER_ROW + x / 8;
        let bit = 7 - (x % 8);
        if black {
            self.data[byte] &= !(1u8 << bit);
        } else {
            self.data[byte] |= 1u8 << bit;
        }
    }
}

impl DrawTarget for Framebuffer {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Infallible>
    where
        I: IntoIterator<Item = Pixel<BinaryColor>>,
    {
        for Pixel(point, color) in pixels {
            if point.x >= 0 && point.y >= 0 {
                self.set_pixel(point.x as usize, point.y as usize, color == BinaryColor::On);
            }
        }
        Ok(())
    }
}

impl OriginDimensions for Framebuffer {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32, HEIGHT as u32)
    }
}

// ---------------------------------------------------------------------------
// Hardware driver
// ---------------------------------------------------------------------------

struct EinkDisplay {
    spi: Spi,
    dc: OutputPin,
    rst: OutputPin,
    busy: InputPin,
}

impl EinkDisplay {
    fn new(cfg: &EinkConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let spi = Spi::new(Bus::Spi0, SlaveSelect::Ss0, 4_000_000, Mode::Mode0)?;
        let gpio = Gpio::new()?;
        Ok(Self {
            spi,
            dc: gpio.get(cfg.dc_pin)?.into_output(),
            rst: gpio.get(cfg.rst_pin)?.into_output(),
            busy: gpio.get(cfg.busy_pin)?.into_input(),
        })
    }

    fn send_command(&mut self, cmd: u8) {
        self.dc.set_low();
        let _ = self.spi.write(&[cmd]);
    }

    fn send_data(&mut self, data: u8) {
        self.dc.set_high();
        let _ = self.spi.write(&[data]);
    }

    fn send_data_bulk(&mut self, data: &[u8]) {
        self.dc.set_high();
        // rppal SPI write limit is typically 4096 bytes; split if needed.
        for chunk in data.chunks(4096) {
            let _ = self.spi.write(chunk);
        }
    }

    fn wait_busy(&self) {
        while self.busy.is_high() {
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn hw_reset(&mut self) {
        self.rst.set_high();
        thread::sleep(Duration::from_millis(10));
        self.rst.set_low();
        thread::sleep(Duration::from_millis(2));
        self.rst.set_high();
        thread::sleep(Duration::from_millis(10));
    }

    fn set_window(&mut self, x_start: u8, y_start: u16, x_end: u8, y_end: u16) {
        self.send_command(0x44);
        self.send_data(x_start >> 3);
        self.send_data(x_end >> 3);

        self.send_command(0x45);
        self.send_data((y_start & 0xFF) as u8);
        self.send_data(((y_start >> 8) & 0xFF) as u8);
        self.send_data((y_end & 0xFF) as u8);
        self.send_data(((y_end >> 8) & 0xFF) as u8);
    }

    fn set_cursor(&mut self, x: u8, y: u16) {
        self.send_command(0x4E);
        self.send_data(x >> 3);

        self.send_command(0x4F);
        self.send_data((y & 0xFF) as u8);
        self.send_data(((y >> 8) & 0xFF) as u8);
    }

    /// Full initialisation (must call before first `display_full`).
    fn init(&mut self) {
        self.hw_reset();
        self.wait_busy();

        self.send_command(0x12); // Software reset
        self.wait_busy();

        self.send_command(0x01); // Driver output control
        self.send_data(0xF9);
        self.send_data(0x00);
        self.send_data(0x00);

        self.send_command(0x11); // Data entry mode: Y increment, X increment
        self.send_data(0x03);

        self.set_window(0, 0, (WIDTH - 1) as u8, (HEIGHT - 1) as u16);
        self.set_cursor(0, 0);

        self.send_command(0x3C); // Border waveform
        self.send_data(0x05);

        self.send_command(0x21); // Display update control
        self.send_data(0x00);
        self.send_data(0x80);

        self.send_command(0x18); // Built-in temperature sensor
        self.send_data(0x80);

        self.wait_busy();
    }

    /// Fast initialisation for partial refresh.
    fn init_fast(&mut self) {
        self.hw_reset();

        self.send_command(0x12); // Software reset
        self.wait_busy();

        self.send_command(0x18); // Temperature sensor
        self.send_data(0x80);

        self.send_command(0x11); // Data entry mode
        self.send_data(0x03);

        self.set_window(0, 0, (WIDTH - 1) as u8, (HEIGHT - 1) as u16);
        self.set_cursor(0, 0);

        self.send_command(0x22); // Load temperature + waveform
        self.send_data(0xB1);
        self.send_command(0x20);
        self.wait_busy();

        self.send_command(0x1A); // Temperature register
        self.send_data(0x64);
        self.send_data(0x00);

        self.send_command(0x22);
        self.send_data(0x91);
        self.send_command(0x20);
        self.wait_busy();
    }

    /// Write buffer and trigger a full (slow, non-flickering) refresh.
    fn display_full(&mut self, fb: &Framebuffer) {
        self.set_cursor(0, 0);
        self.send_command(0x24); // Write RAM (B/W)
        self.send_data_bulk(&fb.data);

        self.send_command(0x21); // Display update control
        self.send_data(0x40);
        self.send_data(0x00);
        self.send_command(0x22); // Display update sequence
        self.send_data(0xF7);
        self.send_command(0x20); // Activate update sequence
        self.wait_busy();
    }

    /// Write buffer and trigger a fast partial refresh.
    ///
    /// Use this for minor content changes (e.g. position updates).
    /// Perform a full refresh periodically to avoid ghosting.
    fn display_partial(&mut self, fb: &Framebuffer) {
        self.set_cursor(0, 0);
        self.send_command(0x24);
        self.send_data_bulk(&fb.data);

        self.send_command(0x21);
        self.send_data(0x00);
        self.send_data(0x00);
        self.send_command(0x22);
        self.send_data(0xFC);
        self.send_command(0x20);
        self.wait_busy();
    }

    /// Put the display into deep sleep mode (low power).
    fn sleep(&mut self) {
        self.send_command(0x10);
        self.send_data(0x01);
    }
}

// ---------------------------------------------------------------------------
// Renderer
// ---------------------------------------------------------------------------

fn fmt_time(secs: f32) -> String {
    let s = secs as u32;
    format!("{:02}:{:02}", s / 60, s % 60)
}

fn fmt_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000 {
        format!("{:.1}MB", bytes as f64 / 1_000_000.0)
    } else {
        format!("{}KB", bytes / 1_000)
    }
}

/// Render the current playback state onto `fb`.
///
/// Layout (portrait 122 × 250 px):
///
/// ```text
/// y= 0.. 16  status icon + track number     (FONT_6X10)
/// y=17.. 35  track name                     (FONT_9X15_BOLD, truncated)
/// y=36.. 48  artist / file name             (FONT_6X10)
/// y=49       ─── separator ───
/// y=55.. 67  progress bar  (outlined rect with filled fraction)
/// y=72.. 84  "mm:ss / mm:ss"                (FONT_6X10)
/// y=85       ─── separator ───
/// y=93..103  "Vol: NN%"                     (FONT_6X10)
/// y=104..114 recording info                 (FONT_6X10, conditional)
/// y=115..125 channel count                  (FONT_6X10, conditional)
/// ```
fn render(fb: &mut Framebuffer, status: &SharedStatus) {
    fb.data.fill(0xFF); // clear to white

    let small = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let large = MonoTextStyle::new(&FONT_9X15_BOLD, BinaryColor::On);
    let stroke = PrimitiveStyle::with_stroke(BinaryColor::On, 1);

    // --- Status icon + track count ---
    let icon = if status.recording {
        "* REC"
    } else if status.playing {
        "> PLAY"
    } else if status.monitoring {
        "~ MON"
    } else {
        "= STOP"
    };
    let total = status.track_list.len();
    let track_num: String = if total > 0 {
        // Find current track index by name.
        let idx = status
            .current_track
            .as_deref()
            .and_then(|name| status.track_list.iter().position(|e| e.name == name))
            .map(|i| i + 1)
            .unwrap_or(0);
        format!("{icon}  {idx}/{total}")
    } else {
        icon.to_string()
    };
    let _ = Text::with_baseline(&track_num, Point::new(2, 2), small, Baseline::Top).draw(fb);

    // --- Track name ---
    let raw_name = status.current_track.as_deref().unwrap_or("No track");
    // Strip common extension suffixes and truncate to ~13 chars (FONT_9X15_BOLD is 9px wide).
    let name_no_ext = raw_name
        .rfind('.')
        .map(|i| &raw_name[..i])
        .unwrap_or(raw_name);
    let track_label: String = name_no_ext.chars().take(13).collect();
    let _ = Text::with_baseline(&track_label, Point::new(2, 18), large, Baseline::Top).draw(fb);

    // --- Channels ---
    let ch = status
        .track_list
        .iter()
        .find(|e| status.current_track.as_deref() == Some(&e.name))
        .and_then(|e| e.channels)
        .unwrap_or(0);
    if ch > 0 {
        let ch_str = format!("{ch}ch");
        let _ = Text::with_baseline(&ch_str, Point::new(2, 36), small, Baseline::Top).draw(fb);
    }

    // --- Separator ---
    let _ = Line::new(Point::new(0, 49), Point::new((WIDTH - 1) as i32, 49))
        .into_styled(stroke)
        .draw(fb);

    // --- Progress bar ---
    let pos = status.position_secs.unwrap_or(0.0);
    let dur = status.duration_secs.unwrap_or(0.0);
    let progress = if dur > 0.0 {
        (pos / dur).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let bar_left = 2i32;
    let bar_top = 55i32;
    let bar_w = (WIDTH as i32) - 4;
    let bar_h = 10i32;

    let _ = Rectangle::new(
        Point::new(bar_left, bar_top),
        Size::new(bar_w as u32, bar_h as u32),
    )
    .into_styled(stroke)
    .draw(fb);

    let fill_w = ((bar_w - 2) as f32 * progress) as u32;
    if fill_w > 0 {
        let _ = Rectangle::new(
            Point::new(bar_left + 1, bar_top + 1),
            Size::new(fill_w, (bar_h - 2) as u32),
        )
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
        .draw(fb);
    }

    // --- Time ---
    let time_str = if dur > 0.0 {
        format!("{} / {}", fmt_time(pos), fmt_time(dur))
    } else {
        "--:-- / --:--".to_string()
    };
    let _ = Text::with_baseline(&time_str, Point::new(2, 70), small, Baseline::Top).draw(fb);

    // --- Separator ---
    let _ = Line::new(Point::new(0, 83), Point::new((WIDTH - 1) as i32, 83))
        .into_styled(stroke)
        .draw(fb);

    // --- Volume ---
    let vol_str = format!("Vol: {}%", status.volume);
    let _ = Text::with_baseline(&vol_str, Point::new(2, 87), small, Baseline::Top).draw(fb);

    // --- Recording info ---
    if status.recording {
        let elapsed = fmt_time(status.recording_duration_secs);
        let size = fmt_bytes(status.recording_size_bytes);
        let rec_str = format!("REC {elapsed} {size}");
        let _ = Text::with_baseline(&rec_str, Point::new(2, 100), small, Baseline::Top).draw(fb);
    }
}

// ---------------------------------------------------------------------------
// Background thread
// ---------------------------------------------------------------------------

/// Spawn the e-ink display update thread.
///
/// The thread runs until `shutdown` is set, then clears the display and
/// puts it into deep sleep before exiting.
pub fn spawn(cfg: EinkConfig, status: Arc<RwLock<SharedStatus>>, shutdown: Arc<AtomicBool>) {
    thread::spawn(move || {
        let mut display = match EinkDisplay::new(&cfg) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("eink: failed to open display: {e}");
                return;
            }
        };

        display.init();

        let refresh_interval = Duration::from_secs(cfg.refresh_interval_secs as u64);
        let partial_interval = Duration::from_secs(5);

        let mut fb = Framebuffer::new();
        let mut last_full = Instant::now()
            .checked_sub(refresh_interval)
            .unwrap_or_else(Instant::now);
        let mut last_partial = Instant::now();
        let mut last_track: Option<String> = None;
        let mut last_playing = false;
        let mut last_recording = false;
        let mut needs_full = true;

        while !shutdown.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(250));

            let s = {
                let Ok(g) = status.read() else { continue };
                g.clone()
            };

            let track_changed = s.current_track != last_track;
            let play_changed = s.playing != last_playing;
            let rec_changed = s.recording != last_recording;
            let timer_full = last_full.elapsed() >= refresh_interval;

            if needs_full || track_changed || play_changed || rec_changed || timer_full {
                render(&mut fb, &s);
                display.init();
                display.display_full(&fb);

                last_track = s.current_track.clone();
                last_playing = s.playing;
                last_recording = s.recording;
                last_full = Instant::now();
                last_partial = Instant::now();
                needs_full = false;
            } else if s.playing && last_partial.elapsed() >= partial_interval {
                render(&mut fb, &s);
                display.init_fast();
                display.display_partial(&fb);
                last_partial = Instant::now();
            }
        }

        // Show a goodbye screen then sleep.
        let mut goodbye = Framebuffer::new();
        let style = MonoTextStyle::new(&FONT_9X15_BOLD, BinaryColor::On);
        let _ = Text::with_baseline("octotrack", Point::new(10, 110), style, Baseline::Top)
            .draw(&mut goodbye);
        let small_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
        let _ = Text::with_baseline(
            "powered off",
            Point::new(15, 130),
            small_style,
            Baseline::Top,
        )
        .draw(&mut goodbye);
        display.init();
        display.display_full(&goodbye);
        display.sleep();
    });
}
