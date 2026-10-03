use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::Stylize,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::{App, Mode, MsgKind};

/// The `:` prompt while typing a command, otherwise the last message.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    match app.mode {
        Mode::Command => {
            let line = &app.command_line;
            let prompt = Line::from(vec![
                Span::styled(":", theme.title),
                Span::raw(line.input.as_str()),
            ]);
            frame.render_widget(Paragraph::new(prompt), area);
            // Cursor after the ':'. Placing the terminal cursor is not a state change.
            let x = area.x.saturating_add(1 + line.cursor as u16);
            if x < area.right() {
                frame.set_cursor_position(Position::new(x, area.y));
            }
        }
        Mode::Normal => {
            let Some((text, kind)) = &app.message else { return };
            let color = match kind {
                MsgKind::Info => theme.info,
                MsgKind::Success => theme.success,
                MsgKind::Error => theme.error,
            };
            frame.render_widget(Paragraph::new(format!(" {text}")).fg(color), area);
        }
    }
}
