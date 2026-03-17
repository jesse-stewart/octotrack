use crate::app::{App, AppResult};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Handles the key events and updates the state of [`App`].
pub fn handle_key_events(key_event: KeyEvent, app: &mut App) -> AppResult<()> {
    // If quit dialog is showing, handle dialog input
    if app.show_quit_dialog {
        match key_event.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.quit();
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.show_quit_dialog = false;
            }
            _ => {}
        }
        return Ok(());
    }

    // If EQ overlay is showing, handle EQ input
    if app.show_eq {
        match key_event.code {
            KeyCode::Char('e') | KeyCode::Char('E') | KeyCode::Esc => {
                app.toggle_eq_view();
            }
            KeyCode::Char('b') | KeyCode::Char('B') => {
                app.toggle_eq_enabled();
            }
            KeyCode::Left => {
                app.eq_select_prev();
            }
            KeyCode::Right => {
                app.eq_select_next();
            }
            KeyCode::Up => {
                app.eq_increase_band();
            }
            KeyCode::Down => {
                app.eq_decrease_band();
            }
            _ => {}
        }
        return Ok(());
    }

    // Normal key handling
    match key_event.code {
        // Show quit confirmation dialog on `ESC` or `q`
        KeyCode::Esc | KeyCode::Char('q') => {
            app.show_quit_dialog = true;
        }
        // Show quit confirmation dialog on `Ctrl-C`
        KeyCode::Char('c') | KeyCode::Char('C') => {
            if key_event.modifiers == KeyModifiers::CONTROL {
                app.show_quit_dialog = true;
            }
        }
        KeyCode::Char(' ') => app.play(),
        KeyCode::Char('s') => {
            app.stop()?;
        }
        KeyCode::Char('l') => {
            app.toggle_loop_mode();
        }
        KeyCode::Char('a') => {
            app.cycle_auto_mode();
        }
        KeyCode::Char('e') => {
            app.toggle_eq_view();
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            app.toggle_recording();
        }
        KeyCode::Char('m') | KeyCode::Char('M') => {
            app.toggle_monitoring();
        }
        // Volume control
        KeyCode::Up => {
            app.increase_volume();
        }
        KeyCode::Down => {
            app.decrease_volume();
        }
        // Track handlers
        KeyCode::Right => {
            app.increment_track();
        }
        KeyCode::Left => {
            app.decrement_track();
        }
        // Other handlers you could add here.
        _ => {}
    }
    Ok(())
}
