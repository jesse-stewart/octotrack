use ratatui::{
    layout::Alignment,
    style::{Color, Style},
    widgets::{Block, BorderType, Paragraph},
    Frame,
};

use crate::app::App;

/// Renders the user interface widgets.
pub fn render(app: &mut App, frame: &mut Frame) {
    // This is where you add new widgets.
    // See the following resources:
    // - https://docs.rs/ratatui/latest/ratatui/widgets/index.html
    // - https://github.com/ratatui-org/ratatui/tree/master/examples
    frame.render_widget(
        Paragraph::new(format!(
            "This is a tui template.\n\
                Press `Esc`, `Ctrl-C` or `q` to stop running.\n\
                Press left and right to increment and decrement the track.\n\
                Current: {:?}\n\
                is_playing: {:?}\n\
                track_title: {:?}\n\
                track_artist: {:?}\n\
                comment: {:?}\n\
                track_channel_count: {:?}\n\
                Tracks: {} / {}",
            app.track_list.get(app.current_track_index),
            app.is_playing,
            app.track_title,
            app.track_artist,
            app.comment,
            app.track_channel_count,
            app.current_track_index + 1,
            app.track_list.len()
        ))
        .block(
            Block::bordered()
                .title("Template")
                .title_alignment(Alignment::Center)
                .border_type(BorderType::Rounded),
        )
        .style(Style::default().fg(Color::Cyan).bg(Color::Black))
        .centered(),
        frame.size(),
    )
}
