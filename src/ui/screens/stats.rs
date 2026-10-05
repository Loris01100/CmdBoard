use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Bar, BarChart, BarGroup, Cell, Paragraph, Row, Table},
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
    // On short terminals the history keeps the room and the charts go.
    let charts_height = if body.height >= 19 { 9 } else { 0 };
    let [summary_area, charts_area, sessions_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(charts_height),
        Constraint::Min(3),
    ])
    .areas(body);

    // Which apps, and the totals.
    let scope = match app
        .stats_app
        .and_then(|id| app.apps.iter().find(|a| a.id == id))
    {
        Some(entry) => t!("stats.one_app", name = entry.name),
        None => t!("stats.all_apps"),
    };
    let summary = t!(
        "stats.summary",
        count = stats.session_count,
        total = format_duration(stats.total_secs),
        longest = format_duration(stats.longest_secs),
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            ratatui::text::Span::styled(scope, theme.title),
            ratatui::text::Span::styled(summary, theme.muted()),
        ])),
        summary_area,
    );

    if charts_height > 0 {
        // Side by side, or only the activity chart when narrow.
        let split = if charts_area.width >= 60 { 45 } else { 0 };
        let [categories_area, activity_area] = Layout::horizontal([
            Constraint::Percentage(split),
            Constraint::Percentage(100 - split),
        ])
        .areas(charts_area);
        if split > 0 {
            render_categories(frame, categories_area, app);
        }
        render_activity(frame, activity_area, app);
    }
    render_sessions(frame, sessions_area, app);

    command_line::render(frame, command, app);
    status_bar::render(frame, status, app);
}

fn render_categories(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let block = theme.panel(&t!("stats.by_category"), false);
    if app.stats.by_category.is_empty() {
        frame.render_widget(
            Paragraph::new(t!("stats.no_sessions"))
                .style(theme.muted())
                .block(block),
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
    let daily = &app.stats.daily;
    let today = daily.last().copied().unwrap_or(0);
    let title = t!(
        "stats.activity",
        weeks = ACTIVITY_DAYS / 7,
        today = format_duration(today)
    );
    // GitHub-style: one column per week (2 cells wide), Monday on top, today bottom right.
    // Too narrow: keep the most recent weeks.
    let weeks = (area.width.saturating_sub(2) as usize / 2).min(ACTIVITY_DAYS / 7);
    let max = daily.iter().copied().max().unwrap_or(0);
    let lines: Vec<Line> = (0..7)
        .map(|row| {
            let spans: Vec<Span> = (0..weeks)
                .map(|col| {
                    // Days back from today; negative is in the future.
                    let back = (weeks - 1 - col) * 7 + row;
                    let back = back.checked_sub(app.stats.today_weekday as usize);
                    match back.and_then(|b| daily.len().checked_sub(b + 1)) {
                        None => Span::raw("  "),
                        Some(i) => match level(daily[i], max) {
                            0 => Span::styled("▁ ", theme.muted()),
                            n => Span::styled(
                                format!("{} ", LEVELS[n]),
                                Style::new().fg(theme.xp_fill),
                            ),
                        },
                    }
                })
                .collect();
            Line::from(spans)
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines).block(theme.panel(&title, false)),
        area,
    );
}

const LEVELS: [&str; 5] = ["▁", "▂", "▃", "▅", "▇"];

/// 0 for no play, else 1..=4 by quarter of the busiest day.
fn level(secs: u64, max: u64) -> usize {
    if secs == 0 {
        0
    } else {
        (secs * 4).div_ceil(max) as usize
    }
}

fn render_sessions(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let title = t!("stats.sessions", count = app.stats.session_count);
    let header = Row::new([
        t!("stats.started"),
        t!("stats.app"),
        t!("stats.duration"),
        t!("stats.xp"),
    ])
    .style(theme.title);
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
        buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
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
        for expected in [
            "Toutes les apps",
            "Temps par catégorie",
            "Jeux",
            "Activité",
            "Sessions (1)",
            "Steam",
            "42m",
            "+47",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in\n{text}");
        }
        assert!(
            text.contains("▇"),
            "today is the busiest day
{text}"
        );
        screen(&app, 20); // narrow: no panic
    }

    #[test]
    fn heatmap_levels() {
        assert_eq!(super::level(0, 100), 0);
        assert_eq!(super::level(1, 100), 1);
        assert_eq!(super::level(25, 100), 1);
        assert_eq!(super::level(26, 100), 2);
        assert_eq!(super::level(100, 100), 4);
    }
}
