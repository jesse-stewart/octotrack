use crate::app::{App, LoopMode};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style, Stylize},
    widgets::{Block, BorderType, Borders, Padding, Paragraph},
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
        .margin(1) // optional: adds a margin around the whole layout
        .constraints([
            Constraint::Length(7), // Height for the title bar
            Constraint::Min(0),    // Remaining space for the main content
            Constraint::Length(3), // Height for the status bar
        ])
        .split(frame.size());

    let title_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1)])
        .split(chunks[0]);

    let main_content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1)])
        .split(chunks[1]);

    let status_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ])
        .split(chunks[2]);

    // Create the title bar widget\
    let title_bar = Paragraph::new(format!("{}\n{}\n", app.track_title, app.track_artist))
        .block(
            Block::default()
                .title("Octotrack")
                .title_alignment(Alignment::Center)
                .border_type(BorderType::Double)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Black)) // Set border color to black
                .style(Style::default().fg(Color::Black).bg(COLOR_AMBER[0]))
                .padding(Padding::new(2, 2, 1, 0)),
        )
        .bold();


    // Create the main content widgets for each column
    let loop_text = match app.loop_mode {
        LoopMode::NoLoop => "Off",
        LoopMode::LoopSingle => "Single",
        LoopMode::LoopAll => "All",
    };

    let main_content_1 = Paragraph::new(format!(
        "Index: {}/{}    Channels: {}    Loop: {}    Volume: {}%",
        app.current_track_index + 1,
        app.track_list.len(),
        app.track_channel_count,
        loop_text,
        app.volume,
    ))
    .block(
        Block::default()
            .border_type(BorderType::Rounded)
            .borders(Borders::ALL)
            .padding(Padding::new(1, 1, 0, 0)),
    );

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
    frame.render_widget(title_bar, title_chunks[0]);
    frame.render_widget(main_content_1, main_content_chunks[0]);
    frame.render_widget(status_content_1, status_chunks[0]);
    frame.render_widget(status_content_2, status_chunks[1]);
    frame.render_widget(status_content_3, status_chunks[2]);
    frame.render_widget(status_content_4, status_chunks[3]);
    frame.render_widget(status_content_5, status_chunks[4]);
}
