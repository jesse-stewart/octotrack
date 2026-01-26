use crate::app::{App, LoopMode};
use crate::bigtext::BigText;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style, Stylize},
    widgets::{Block, BorderType, Borders, Padding, Paragraph, Gauge, Bar, BarChart, BarGroup},
    Frame,
};

const COLOR_AMBER: [Color; 8] = [
    Color::Rgb(255, 204, 0),
    Color::Rgb(226, 177, 0),
    Color::Rgb(197, 151, 0),
    Color::Rgb(168, 126, 0),
    Color::Rgb(138, 102, 0),
    Color::Rgb(109, 79, 0),
    Color::Rgb(80, 56, 0),
    Color::Rgb(51, 35, 0),
];

/// Renders the user interface widgets.
pub fn render(app: &mut App, frame: &mut Frame) {
    // Define the layout constraints for each section
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(7), // Height for the title bar (big text)
            Constraint::Length(3), // Height for the progress gauge
            Constraint::Min(0),    // Channel meters (dynamic)
            Constraint::Length(3), // Height for the status bar
        ])
        .split(frame.size());

    let title_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1)])
        .split(chunks[0]);

    let progress_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1)])
        .split(chunks[1]);

    let meter_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(1),     // Channel levels
            Constraint::Length(9), // Volume indicator
            Constraint::Length(25), // Info sidebar (increased for artist/track)
        ])
        .split(chunks[2]);

    let status_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ])
        .split(chunks[3]);

    // Create the title bar widget with big text
    let mut title_text = if app.track_title.is_empty() {
        "No Track".to_string()
    } else {
        app.track_title.clone()
    };

    // Remove "tracks/" prefix if present
    if title_text.starts_with("tracks/") {
        title_text = title_text.strip_prefix("tracks/").unwrap_or(&title_text).to_string();
    }

    // Truncate if too long (limit to ~12 chars for small screen)
    if title_text.len() > 32 {
        title_text.truncate(32);
    }

    let title_block = Block::default()
        .title("Octotrack")
        .title_alignment(Alignment::Center)
        .border_type(BorderType::Double)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Black))
        .style(Style::default().bg(Color::Black));

    let title_bar = BigText::new(
        title_text,
        Style::default().fg(COLOR_AMBER[0]).bg(Color::Black)
    );

    // Create the progress gauge
    let (progress_ratio, time_text) = if let (Some(pos), Some(dur)) = (app.current_position, app.track_duration) {
        let ratio = if dur > 0.0 { (pos / dur).min(1.0) as f64 } else { 0.0 };
        let current_min = (pos / 60.0) as u32;
        let current_sec = (pos % 60.0) as u32;
        let total_min = (dur / 60.0) as u32;
        let total_sec = (dur % 60.0) as u32;
        let text = format!("{}:{:02} / {}:{:02}", current_min, current_sec, total_min, total_sec);
        (ratio, text)
    } else {
        (0.0, "--:-- / --:--".to_string())
    };

    let progress_gauge = Gauge::default()
        .block(
            Block::default()
                .title("Progress")
                .title_alignment(Alignment::Left)
                .border_type(BorderType::Rounded)
                .borders(Borders::ALL),
        )
        .gauge_style(Style::default().fg(COLOR_AMBER[0]).bg(Color::Black))
        .ratio(progress_ratio);

    // Create the info content
    let loop_text = match app.loop_mode {
        LoopMode::NoLoop => "Off",
        LoopMode::LoopSingle => "Single",
        LoopMode::LoopAll => "All",
    };

    // Prepare artist and title display
    let artist_display = if app.track_artist.is_empty() {
        "Unknown".to_string()
    } else {
        app.track_artist.clone()
    };

    let title_display = if app.track_title.is_empty() {
        "No Track".to_string()
    } else {
        // Remove "tracks/" prefix if present
        let mut title = app.track_title.clone();
        if title.starts_with("tracks/") {
            title = title.strip_prefix("tracks/").unwrap_or(&title).to_string();
        }
        title
    };

    let autoplay_text = if app.autoplay { "On" } else { "Off" };

    let info_content = Paragraph::new(format!(
        "Artist:\n{}\n\nTrack:\n{}\n\nTrack #:{}/{}\n\n{} Channels\n\nLoop: {}\nAutoplay: {}",
        artist_display,
        title_display,
        app.current_track_index + 1,
        app.track_list.len(),
        app.track_channel_count,
        loop_text,
        autoplay_text,
    ))
    .block(
        Block::default()
            .title_alignment(Alignment::Left)
            .padding(Padding::new(0, 0, 0, 0)),
    );

    // Create channel meters using BarChart
    let channel_count = app.channel_levels.len().min(app.track_channel_count as usize);

    let bar_chart_block = Block::default()
        .title("Channel Levels (dB)")
        .title_alignment(Alignment::Left)
        .border_type(BorderType::Rounded)
        .borders(Borders::ALL)
        .padding(Padding::new(1, 1, 0, 0));

    if channel_count > 0 && app.is_playing {
        let bars: Vec<Bar> = app.channel_levels.iter().enumerate().take(channel_count)
            .map(|(i, &level)| {
                // Convert dB to a display value (0-60 range for visualization)
                let level_clamped = level.max(-60.0).min(0.0);
                let display_value = (level_clamped + 60.0) as u64;

                // Choose color based on level
                let color = if level > -6.0 {
                    Color::Red  // Clipping warning
                } else if level > -12.0 {
                    COLOR_AMBER[0]  // Good level
                } else if level > -24.0 {
                    Color::Yellow  // Medium level
                } else {
                    Color::Green  // Low level
                };

                let label = format!("Ch{} {:.0}dB", i + 1, level);

                Bar::default()
                    .value(display_value)
                    .label(label.into())
                    .style(Style::default().fg(color))
                    .value_style(Style::default().fg(color).bold())
            })
            .collect();

        let bar_chart = BarChart::default()
            .block(bar_chart_block)
            .data(BarGroup::default().bars(&bars))
            .bar_width(3)
            .bar_gap(1)
            .max(60);  // -60dB to 0dB range

        frame.render_widget(bar_chart, meter_chunks[0]);
    } else {
        // Show placeholder when not playing
        let placeholder = Paragraph::new(if !app.is_playing {
            "Press [Space] to start playback"
        } else {
            "Initializing meters..."
        })
        .block(bar_chart_block)
        .alignment(Alignment::Center);

        frame.render_widget(placeholder, meter_chunks[0]);
    }

    // Create volume indicator (vertical bar)
    let volume_bar = Bar::default()
        .value(app.volume as u64)
        .label(format!("{}%", app.volume).into())
        .style(Style::default().fg(COLOR_AMBER[0]))
        .value_style(Style::default().fg(COLOR_AMBER[0]).bold());

    let volume_chart = BarChart::default()
        .block(
            Block::default()
                .title("Volume")
                .title_alignment(Alignment::Center)
                .padding(Padding::new(2, 1, 1, 0)),
        )
        .data(BarGroup::default().bars(&[volume_bar]))
        .bar_width(5)
        .max(100);

    let status_content_1 = Paragraph::new("[←] Prev")
        .block(
            Block::default()
                .border_type(BorderType::Double)
                .borders(Borders::ALL)
                .padding(Padding::new(1, 1, 0, 0)),
        )
        .alignment(Alignment::Center);

    let status_content_2 = Paragraph::new("[Space] Play")
        .block(
            Block::default()
                .border_type(BorderType::Double)
                .borders(Borders::ALL)
                .padding(Padding::new(1, 1, 0, 0)),
        )
        .style(if app.is_playing {
            Style::default().fg(COLOR_AMBER[0])
        } else {
            Style::default()
        })
        .alignment(Alignment::Center);

    let status_content_3 = Paragraph::new("[S] Stop")
        .block(
            Block::default()
                .border_type(BorderType::Double)
                .borders(Borders::ALL)
                .padding(Padding::new(1, 1, 0, 0)),
        )
        .style(if !app.is_playing {
            Style::default().fg(COLOR_AMBER[0])
        } else {
            Style::default()
        })
        .alignment(Alignment::Center);

    let status_content_4 = Paragraph::new("[→] Next")
        .block(
            Block::default()
                .border_type(BorderType::Double)
                .borders(Borders::ALL)
                .padding(Padding::new(1, 1, 0, 0)),
        )
        .alignment(Alignment::Center);

    let loop_mode_text = match app.loop_mode {
        LoopMode::NoLoop => "[L] Loop: Off",
        LoopMode::LoopSingle => "[L] Loop: 1",
        LoopMode::LoopAll => "[L] Loop: All",
    };

    let status_content_5 = Paragraph::new(loop_mode_text)
        .block(
            Block::default()
                .border_type(BorderType::Double)
                .borders(Borders::ALL)
                .padding(Padding::new(1, 1, 0, 0)),
        )
        .style(if app.loop_mode != LoopMode::NoLoop {
            Style::default().fg(COLOR_AMBER[0])
        } else {
            Style::default()
        })
        .alignment(Alignment::Center);

    // Render each widget in its respective area
    frame.render_widget(title_block.clone(), title_chunks[0]);
    let title_inner = title_block.inner(title_chunks[0]);
    frame.render_widget(title_bar, title_inner);
    frame.render_widget(progress_gauge, progress_chunks[0]);
    // Channel meters are already rendered above
    frame.render_widget(volume_chart, meter_chunks[1]);
    frame.render_widget(info_content, meter_chunks[2]);
    frame.render_widget(status_content_1, status_chunks[0]);
    frame.render_widget(status_content_2, status_chunks[1]);
    frame.render_widget(status_content_3, status_chunks[2]);
    frame.render_widget(status_content_4, status_chunks[3]);
    frame.render_widget(status_content_5, status_chunks[4]);
}
