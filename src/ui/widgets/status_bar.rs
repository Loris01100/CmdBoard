use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::{App, Screen};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let hints: &[(&str, &str)] = match app.screen {
        Screen::Dashboard => &[
            ("j/k", "naviguer"),
            ("Tab ←→", "panneau"),
            ("1-4", "écrans"),
            ("q", "quitter"),
        ],
        _ => &[("1-4", "écrans"), ("q", "quitter")],
    };

    let mut spans = vec![Span::raw(" ")];
    for (i, (key, label)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(*key, theme.title));
        spans.push(Span::styled(format!(" {label}"), theme.muted()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
