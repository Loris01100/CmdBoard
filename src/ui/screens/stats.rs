use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    symbols::Marker,
    text::{Line, Span},
    widgets::{
        Cell, Paragraph, Row, Table,
        canvas::{Canvas, Points},
    },
};

use crate::app::App;
use crate::storage::models::ACTIVITY_DAYS;
use crate::ui::{
    icons, layout,
    widgets::{command_line, format_duration, status_bar},
};

/// Time per category, activity over the last days, then the session history.
pub fn draw(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let stats = &app.stats;
    let (body, command, status_area) = layout::screen(frame.area(), command_line::height(app));
    // On short terminals the history keeps the room and the charts go.
    let charts_height = if body.height >= 20 { 10 } else { 0 };
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
        today = format_duration(stats.daily.last().copied().unwrap_or(0)),
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            ratatui::text::Span::styled(scope, theme.title),
            ratatui::text::Span::styled(summary, theme.muted()),
        ])),
        summary_area,
    );

    if charts_height > 0 {
        // The pie (`s`: per category or per app), then the heatmap on the right half;
        // narrow: heatmap only.
        let pie = u16::from(charts_area.width >= 60);
        let [pie_area, activity_area] =
            Layout::horizontal([Constraint::Fill(pie), Constraint::Percentage(50)])
                .areas(charts_area);
        if pie > 0 {
            let (title, data) = if app.stats_by_app {
                (t!("stats.by_app"), &stats.by_app)
            } else {
                (t!("stats.by_category"), &stats.by_category)
            };
            render_pie(frame, pie_area, app, &title, data);
        }
        render_activity(frame, activity_area, app);
    }
    render_sessions(frame, sessions_area, app);

    command_line::render(frame, command, app);
    status_bar::render(frame, status_area, app);
}

/// Share of the play time per name, as a braille pie with its legend.
fn render_pie(frame: &mut Frame, area: Rect, app: &App, title: &str, data: &[(String, u64)]) {
    let theme = &app.theme;
    let block = theme.panel(title, false);
    if data.is_empty() {
        frame.render_widget(
            Paragraph::new(t!("stats.no_sessions"))
                .style(theme.muted())
                .block(block),
            area,
        );
        return;
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Distinct theme colors; names past them share the muted "others" slice.
    let colors = [theme.xp_fill, theme.info, theme.warning, theme.error];
    let slices = pie_slices(data, colors.len());
    let color = |i: usize| colors.get(i).copied().unwrap_or(theme.muted);
    let total: u64 = slices.iter().map(|(_, secs)| secs).sum::<u64>().max(1);

    // A cell is about twice as tall as wide, and holds 2×4 braille dots: square dots.
    let [pie_area, legend_area] =
        Layout::horizontal([Constraint::Length(inner.height * 2 + 1), Constraint::Min(0)])
            .areas(inner);
    let (w, h) = (
        i32::from(pie_area.width) * 2,
        i32::from(pie_area.height) * 4,
    );
    let (cx, cy, r) = (
        f64::from(w - 1) / 2.0,
        f64::from(h - 1) / 2.0,
        f64::from(w.min(h)) / 2.0,
    );
    let mut dots = vec![Vec::new(); slices.len()];
    for x in 0..w {
        for y in 0..h {
            let (dx, dy) = (f64::from(x) - cx, f64::from(y) - cy);
            if dx.hypot(dy) > r {
                continue;
            }
            // Clockwise from the top, as a fraction of the turn.
            let turn = dx.atan2(dy).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU;
            let mut start = 0.0;
            let slice = slices
                .iter()
                .position(|(_, secs)| {
                    start += *secs as f64 / total as f64;
                    turn < start
                })
                .unwrap_or(slices.len() - 1);
            dots[slice].push((f64::from(x), f64::from(y)));
        }
    }
    let pie = Canvas::default()
        .marker(Marker::Braille)
        .x_bounds([0.0, f64::from(w - 1)])
        .y_bounds([0.0, f64::from(h - 1)])
        .paint(|ctx| {
            for (i, coords) in dots.iter().enumerate() {
                ctx.draw(&Points {
                    coords,
                    color: color(i),
                });
            }
        });
    frame.render_widget(pie, pie_area);

    let names: Vec<String> = slices
        .iter()
        .map(|(name, _)| name.clone().unwrap_or_else(|| t!("stats.others")))
        .collect();
    // "● " + name + " 42m  58%": shorten names rather than lose the numbers.
    let room = (legend_area.width as usize).saturating_sub(12).max(1);
    let pad = names
        .iter()
        .map(|n| n.chars().count())
        .max()
        .unwrap_or(0)
        .min(room);
    let legend: Vec<Line> = slices
        .iter()
        .zip(&names)
        .enumerate()
        .map(|(i, ((_, secs), name))| {
            Line::from(vec![
                Span::styled(format!("{} ", icons::DOT), Style::new().fg(color(i))),
                Span::raw(format!("{:pad$}", ellipsis(name, pad))),
                Span::styled(
                    format!(
                        " {:>4} {:>3}%",
                        format_duration(*secs),
                        (secs * 100 + total / 2) / total
                    ),
                    Style::new().fg(theme.info),
                ),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(legend), legend_area);
}

/// `text` cut to `width` characters, the last one an ellipsis when cut.
fn ellipsis(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(width.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// The `max - 1` most played names, then the rest summed as `None` ("others").
/// Fits in `max` slices as is.
fn pie_slices(by_category: &[(String, u64)], max: usize) -> Vec<(Option<String>, u64)> {
    if by_category.len() <= max {
        return by_category
            .iter()
            .map(|(n, s)| (Some(n.clone()), *s))
            .collect();
    }
    let (top, rest) = by_category.split_at(max - 1);
    let mut slices: Vec<_> = top.iter().map(|(n, s)| (Some(n.clone()), *s)).collect();
    slices.push((None, rest.iter().map(|(_, s)| s).sum()));
    slices
}

fn render_activity(frame: &mut Frame, area: Rect, app: &App) {
    const LABELS: usize = 3; // weekday names column
    let theme = &app.theme;
    let (daily, today) = (&app.stats.daily, app.stats.today);
    let weekday = usize::try_from((today + 3).rem_euclid(7)).unwrap_or(0); // 0 = Monday
    // Same square everywhere, the color tells the time played.
    let cell = |secs: u64| (icons::SQUARE, theme.heat(level(secs)));

    // Bottom border: what each color means, in time played.
    let mut legend = vec![Span::raw(" ")];
    for n in 0..=HEAT_STEPS.len() + 1 {
        let label = match n {
            0 => "0".to_string(),
            n if n <= HEAT_STEPS.len() => format!("<{}", format_duration(HEAT_STEPS[n - 1])),
            _ => format!("≥{}", format_duration(HEAT_STEPS[HEAT_STEPS.len() - 1])),
        };
        legend.push(Span::styled(format!("{} ", icons::SQUARE), theme.heat(n)));
        legend.push(Span::styled(format!("{label}  "), theme.muted()));
    }
    let block = theme
        .panel(&t!("stats.activity", weeks = ACTIVITY_DAYS / 7), false)
        .title_bottom(Line::from(legend));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // GitHub-style: one column per week, Monday on top, today bottom right, the date of
    // each Monday above. Too narrow: keep the most recent weeks.
    let room = (inner.width as usize).saturating_sub(LABELS);
    let width = (room / (ACTIVITY_DAYS / 7)).clamp(2, 4);
    let weeks = (room / width).min(ACTIVITY_DAYS / 7);
    // Days back from today of a cell; `None` in the future.
    let back = |col: usize, row: usize| ((weeks - 1 - col) * 7 + weekday).checked_sub(row);

    let mut header = vec![' '; LABELS + weeks * width + 5];
    let step = 6usize.div_ceil(width); // "dd/mm" and a space
    for col in (0..weeks).filter(|col| (weeks - 1 - col).is_multiple_of(step)) {
        let (day, month) = day_month(today - i64::try_from(back(col, 0).unwrap_or(0)).unwrap_or(0));
        let start = LABELS + col * width;
        header.splice(start..start + 5, format!("{day:02}/{month:02}").chars());
    }
    let mut lines = vec![Line::styled(
        header.into_iter().collect::<String>(),
        theme.muted(),
    )];

    let names = t!("stats.weekdays");
    for (row, name) in names.split_whitespace().take(7).enumerate() {
        let mut spans = vec![Span::styled(format!("{name:<LABELS$}"), theme.muted())];
        for col in 0..weeks {
            let index = back(col, row).and_then(|b| daily.len().checked_sub(b + 1));
            spans.push(match index {
                None => Span::raw(" ".repeat(width)),
                Some(i) => {
                    let (glyph, style) = cell(daily[i]);
                    Span::styled(format!("{glyph:<width$}"), style)
                }
            });
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// `(day, month)` of a count of days since 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn day_month(days: i64) -> (i64, i64) {
    let z = days + 719_468;
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (doy - (153 * mp + 2) / 5 + 1, month)
}

/// Upper bounds (seconds) of heat levels 1 to 3; level 4 is the rest.
const HEAT_STEPS: [u64; 3] = [22 * 60, 45 * 60, 60 * 60];

/// 0 for no play, else 1..=4 by fixed steps of time played.
fn level(secs: u64) -> usize {
    match secs {
        0 => 0,
        _ => 1 + HEAT_STEPS.iter().take_while(|&&step| secs >= step).count(),
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
    let mut state = app.stats_state;
    frame.render_stateful_widget(table, area, &mut state);
}

#[cfg(test)]
mod tests {
    use crate::app::{App, Screen};
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    };

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
        assert!(text.contains("■"), "heatmap squares\n{text}");

        app.on_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        let text = screen(&app, 110);
        assert!(text.contains("Temps par app"), "{text}");
        assert!(!text.contains("Jeux"), "{text}");
        screen(&app, 20); // narrow: no panic
    }

    #[test]
    fn pie_groups_the_tail() {
        let cats: Vec<_> = [("a", 5), ("b", 4), ("c", 3)]
            .map(|(n, s)| (n.to_string(), s))
            .into();
        assert_eq!(super::pie_slices(&cats, 3).len(), 3);
        assert_eq!(
            super::pie_slices(&cats, 2),
            [(Some("a".to_string()), 5), (None, 7)]
        );
    }

    #[test]
    fn ellipsis_cuts_long_names() {
        assert_eq!(super::ellipsis("Steam", 8), "Steam");
        assert_eq!(super::ellipsis("Windows Terminal", 8), "Windows…");
    }

    #[test]
    fn day_month_from_epoch_days() {
        assert_eq!(super::day_month(0), (1, 1));
        assert_eq!(super::day_month(19_782), (29, 2)); // 2024, leap year
        assert_eq!(super::day_month(20_000), (4, 10));
    }

    #[test]
    fn heatmap_levels() {
        let level = |minutes: u64| super::level(minutes * 60);
        assert_eq!(super::level(0), 0);
        assert_eq!(super::level(1), 1);
        assert_eq!((level(21), level(22)), (1, 2));
        assert_eq!((level(44), level(45)), (2, 3));
        assert_eq!((level(59), level(60)), (3, 4));
        assert_eq!(level(600), 4);
    }
}
