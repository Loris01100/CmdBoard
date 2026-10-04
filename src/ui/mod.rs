pub mod layout;
mod screens;
pub mod theme;
mod widgets;

use ratatui::Frame;

use crate::app::{App, Mode, Screen};

/// Pure rendering: reads `app`, never mutates it.
pub fn draw(frame: &mut Frame, app: &App) {
    match app.screen {
        Screen::Dashboard => screens::dashboard::draw(frame, app),
        Screen::Help => screens::help::draw(frame, app),
        Screen::Stats | Screen::Rewards => screens::coming_soon(frame, app),
    }
    if let Mode::Popup(popup) = &app.mode {
        widgets::popup::render(frame, popup, app);
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
    fn command_line_shows_prompt_then_message() {
        use crate::app::Mode;
        let mut app = App::with_defaults();
        let render = |app: &App| {
            let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
            terminal.draw(|f| draw(f, app)).unwrap();
            let buffer = terminal.backend().buffer();
            buffer.content().iter().map(|c| c.symbol()).collect::<String>()
        };
        app.mode = Mode::Command;
        "help add".chars().for_each(|c| app.command_line.input.insert(c));
        assert!(render(&app).contains(":help add"));

        let text = app.command_line.submit();
        app.mode = Mode::Normal;
        app.execute(crate::command::parser::parse(&text).unwrap());
        assert!(render(&app).contains("add [<nom> <cible>"));
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
