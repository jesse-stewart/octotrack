/// Audio
pub mod audio;

/// Application.
pub mod app;

/// Application configuration (TOML schema, load, save, migrate).
pub mod config;

/// Terminal events handler.
pub mod event;

/// Widget renderer.
pub mod ui;

/// Terminal user interface.
pub mod tui;

/// Event handler.
pub mod handler;

/// Big text widget.
pub mod bigtext;

/// Cron-style task scheduler.
pub mod schedule;

/// First-run setup and factory reset.
pub mod setup;

/// Web UI server (actix-web).
pub mod web;

/// SPI e-ink display driver (Waveshare 2.13" HAT).
#[cfg(feature = "eink")]
pub mod eink;
