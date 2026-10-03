use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::Stylize,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::{App, Mode, MsgKind};

const PLACEHOLDER: &str = "launch <app>, add, move, help…";

/// Rows the command area needs: a bordered box while typing, one line otherwise.
pub fn height(app: &App) -> u16 {
    match app.mode {
        Mode::Command => 3,
        Mode::Normal => 1,
    }
}

/// The `:` input box while typing a command, otherwise the last message.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    match app.mode {
        Mode::Command => render_input(frame, area, app),
        Mode::Normal => render_message(frame, area, app),
    }
}

fn render_input(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let block = theme.panel("Commande", true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width < 2 || inner.height == 0 {
        return;
    }

    let line = &app.command_line;
    // Room left after the ':' prompt; scroll so the cursor always stays visible.
    let room = (inner.width - 1) as usize;
    let offset = line.cursor.saturating_sub(room - 1);
    let visible: String = line.input.chars().skip(offset).take(room).collect();

    let text = if line.input.is_empty() {
        Span::styled(PLACEHOLDER, theme.muted())
    } else {
        Span::raw(visible)
    };
    let prompt = Line::from(vec![Span::styled(":", theme.title), text]);
    frame.render_widget(Paragraph::new(prompt), inner);

    // Placing the terminal cursor is not a state change.
    let x = inner.x + 1 + (line.cursor - offset) as u16;
    frame.set_cursor_position(Position::new(x, inner.y));
}

fn render_message(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let Some((text, kind)) = &app.message else { return };
    let color = match kind {
        MsgKind::Info => theme.info,
        MsgKind::Success => theme.success,
        MsgKind::Error => theme.error,
    };
    frame.render_widget(Paragraph::new(format!(" {text}")).fg(color), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        Terminal,
        backend::{Backend, TestBackend},
    };

    fn render_box(app: &App, width: u16) -> (Vec<String>, Position) {
        let mut terminal = Terminal::new(TestBackend::new(width, 3)).unwrap();
        terminal
            .draw(|f| render(f, f.area(), app))
            .unwrap();
        let backend = terminal.backend_mut();
        let buffer = backend.buffer().clone();
        let rows = (0..3)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        (rows, backend.get_cursor_position().unwrap())
    }

    #[test]
    fn input_box_shows_text_and_cursor() {
        let mut app = App::with_defaults();
        app.mode = Mode::Command;
        assert!(render_box(&app, 40).0[1].contains(PLACEHOLDER));

        "launch Steam".chars().for_each(|c| app.command_line.insert(c));
        let (rows, cursor) = render_box(&app, 40);
        assert!(rows[0].contains("Commande"));
        assert!(rows[1].contains(":launch Steam"));
        assert_eq!(cursor, Position::new(1 + 1 + 12, 1)); // border, ':', 12 chars
    }

    #[test]
    fn long_input_scrolls_to_keep_cursor_visible() {
        let mut app = App::with_defaults();
        app.mode = Mode::Command;
        let long = format!("add Jeu {}end", "x".repeat(40));
        long.chars().for_each(|c| app.command_line.insert(c));
        let (rows, cursor) = render_box(&app, 20);
        assert!(rows[1].contains("end"));
        assert!(cursor.x < 19); // inside the right border
    }
}
