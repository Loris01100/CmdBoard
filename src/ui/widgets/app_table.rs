use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::Style,
    text::Span,
    widgets::{Cell, Row, Table},
};

use super::{format_duration, xp_bar};
use crate::app::{App, Focus, Mode};
use crate::core::xp;
use crate::ui::layout::cells;

const XP_BAR_WIDTH: usize = 8;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let focused = app.focus == Focus::Apps;

    let title = match app.mode {
        Mode::Search => t!("apps.search_title", count = app.visible_apps().len()),
        _ => t!("apps.title", sort = app.sort.label()),
    };
    let header = Row::new([
        t!("apps.name"),
        t!("apps.level"),
        t!("apps.xp"),
        t!("apps.time"),
    ])
    .style(theme.title);
    let rows = app.visible_apps().into_iter().map(|entry| {
        let (level, xp) = app.shown_app_xp(entry);
        let progress = xp::level_progress(level, xp);
        Row::new([
            Cell::from(entry.name.as_str()),
            Cell::from(level.to_string()),
            Cell::from(Span::styled(
                xp_bar::text(progress, XP_BAR_WIDTH),
                Style::new().fg(theme.xp_fill),
            )),
            Cell::from(format_duration(entry.total_secs)),
        ])
    });
    let widths = [
        Constraint::Min(12),
        Constraint::Length(4),
        Constraint::Length(cells(XP_BAR_WIDTH)),
        Constraint::Length(6),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(theme.panel(&title, focused))
        .row_highlight_style(theme.highlight(focused))
        .highlight_symbol("> ");

    // Rendering needs `&mut TableState`; work on a copy so `draw` stays pure.
    let mut state = app.app_state;
    frame.render_stateful_widget(table, area, &mut state);
}
