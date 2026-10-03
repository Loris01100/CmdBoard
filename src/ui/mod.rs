pub mod layout;
mod screens;
pub mod theme;
mod widgets;

use ratatui::Frame;

use crate::app::{App, Screen};

/// Pure rendering: reads `app`, never mutates it.
pub fn draw(frame: &mut Frame, app: &App) {
    match app.screen {
        Screen::Dashboard => screens::dashboard::draw(frame, app),
        Screen::Stats | Screen::Rewards | Screen::Help => screens::coming_soon(frame, app),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn every_screen_renders_at_any_size() {
        let mut app = App::with_defaults();
        for screen in [Screen::Dashboard, Screen::Stats, Screen::Rewards, Screen::Help] {
            app.screen = screen;
            for (w, h) in [(120, 30), (70, 20), (10, 3)] {
                let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                terminal.draw(|f| draw(f, &app)).unwrap();
            }
        }
    }

    #[test]
    fn details_panel_hidden_when_narrow() {
        let app = App::with_defaults();
        let render = |w| {
            let mut terminal = Terminal::new(TestBackend::new(w, 30)).unwrap();
            terminal.draw(|f| draw(f, &app)).unwrap();
            let buffer = terminal.backend().buffer();
            buffer.content().iter().map(|c| c.symbol()).collect::<String>()
        };
        assert!(render(120).contains("Détails"));
        assert!(!render(80).contains("Détails"));
    }
}
