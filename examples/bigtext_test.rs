use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use octotrack::bigtext::BigText;
use ratatui::{backend::CrosstermBackend, layout::Rect, style::Style, Terminal};
use std::io;

const ROWS: &[&str] = &[
    "ABCDEFGHIJKLM",
    "NOPQRSTUVWXYZ",
    "0123456789",
    ".,:;!?-+=()[]",
    "{}|_^~@#$%&*'",
    "\"/\\<>`",
];

fn main() -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    terminal.draw(|f| {
        let area = f.size();
        for (i, row) in ROWS.iter().enumerate() {
            let y = i as u16 * 6;
            if y + 5 > area.height {
                break;
            }
            let widget = BigText::new(*row, Style::default());
            f.render_widget(widget, Rect::new(0, y, area.width, 5));
        }
    })?;

    loop {
        if let Event::Key(key) = event::read()? {
            if key.code == KeyCode::Char('q') || key.code == KeyCode::Esc {
                break;
            }
        }
    }

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}
