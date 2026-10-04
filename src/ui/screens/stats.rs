use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::Line,
    widgets::{Bar, BarChart, BarGroup, Cell, Paragraph, Row, Sparkline, Table},
};

use crate::app::App;
use crate::storage::models::ACTIVITY_DAYS;
use crate::ui::{
    layout,
    widgets::{command_line, format_duration, status_bar},
};

/// Time per category, activity over the last days, then the session history.
pub fn draw(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let stats = &app.stats;
    let (body, command, status) = layout::screen(frame.area(), command_line::height(app));
    let [summary_area, charts_area, sessions_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(8),
        Constraint::Min(3),
    ])
    .areas(body);

    // Which apps, and the totals.
    let scope = match app.stats_app.and_then(|id| app.apps.iter().find(|a| a.id == id)) {
        Some(entry) => format!(" {} (:stats pour tout voir)", entry.name),
        None => " Toutes les apps".into(),
    };
    let summary = format!(
        "  ·  {} session(s)  ·  {} au total  ·  plus longue : {}",
        stats.session_count,
        format_duration(stats.total_secs),
        format_duration(stats.longest_secs),
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            ratatui::text::Span::styled(scope, theme.title),
            ratatui::text::Span::styled(summary, theme.muted()),
        ])),
        summary_area,
    );

    let [categories_area, activity_area] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)])
            .areas(charts_area);
    render_categories(frame, categories_area, app);
    render_activity(frame, activity_area, app);
    render_sessions(frame, sessions_area, app);

    command_line::render(frame, command, app);
    status_bar::render(frame, status, app);
}

fn render_categories(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let block = theme.panel("Temps par catégorie", false);
    if app.stats.by_category.is_empty() {
        frame.render_widget(
            Paragraph::new("Aucune session").style(theme.muted()).block(block),
            area,
        );
        return;
    }
    let bars: Vec<Bar> = app
        .stats
        .by_category
        .iter()
        .map(|(name, secs)| {
            Bar::default()
                .label(Line::from(name.clone()))
                .value(*secs / 60)
                .text_value(format_duration(*secs))
        })
        .collect();
    let chart = BarChart::default()
        .block(block)
        .direction(Direction::Horizontal)
        .bar_width(1)
        .bar_gap(0)
        .bar_style(Style::new().fg(theme.xp_fill))
        .value_style(Style::new().fg(theme.info))
        .data(BarGroup::default().bars(&bars));
    frame.render_widget(chart, area);
}

fn render_activity(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let today = app.stats.daily.last().copied().unwrap_or(0);
    let title = format!("Activité ({ACTIVITY_DAYS} jours) · aujourd'hui : {}", format_duration(today));
    // One column per day, today on the right: keep the most recent days if too narrow.
    let width = area.width.saturating_sub(2) as usize;
    let daily = &app.stats.daily;
    let shown = &daily[daily.len().saturating_sub(width)..];
    let sparkline = Sparkline::default()
        .block(theme.panel(&title, false))
        .data(shown)
        .style(Style::new().fg(theme.xp_fill));
    frame.render_widget(sparkline, area);
}

fn render_sessions(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let title = format!("Sessions ({})", app.stats.session_count);
    let header = Row::new(["Début", "App", "Durée", "XP"]).style(theme.title);
    let rows = app.stats.sessions.iter().map(|s| {
        Row::new([
            Cell::from(s.started.as_str()),
            Cell::from(s.app.as_str()),
            Cell::from(format_duration(s.duration_secs)),
            Cell::from(format!("+{}", s.xp)),
        ])
    });
    let widths = [
        Constraint::Length(17),
        Constraint::Min(12),
        Constraint::Length(6),
        Constraint::Length(6),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(theme.panel(&title, true))
        .row_highlight_style(theme.highlight(true))
        .highlight_symbol("> ");
    // Rendering needs `&mut TableState`; work on a copy so `draw` stays pure.
    let mut state = app.stats_state.clone();
    frame.render_stateful_widget(table, area, &mut state);
}

#[cfg(test)]
mod tests {
    use crate::app::{App, Screen};
    use ratatui::{Terminal, backend::TestBackend};

    fn screen(app: &App, width: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
        terminal.draw(|f| crate::ui::draw(f, app)).unwrap();
        let buffer = terminal.backend().buffer();
        buffer.content().iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn shows_charts_and_history() {
        let mut app = App::with_defaults();
        app.screen = Screen::Stats;
        assert!(screen(&app, 110).contains("Aucune session"));

        let steam = app.find_app("Steam").unwrap().id;
        app.on_session_start(steam);
        app.on_session_end(steam, 42 * 60);
        app.mode = crate::app::Mode::Normal; // dismiss the popups
        let text = screen(&app, 110);
        for expected in ["Toutes les apps", "Temps par catégorie", "Jeux", "Activité", "Sessions (1)", "Steam", "42m", "+47"] {
            assert!(text.contains(expected), "missing {expected:?} in\n{text}");
        }
        screen(&app, 20); // narrow: no panic
    }
}
