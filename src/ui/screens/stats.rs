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
        // Pies on the left, the heatmap (2 cells per week) on the right; narrow: heatmap only.
        // Filtered on one app, the per-app pie would be a single slice: skipped.
        let pies: Vec<(String, &[(String, u64)])> = match charts_area.width {
            0..60 => vec![],
            60..90 => vec![(t!("stats.by_category"), &stats.by_category)],
            _ => [
                Some((t!("stats.by_category"), &stats.by_category[..])),
                app.stats_app
                    .is_none()
                    .then(|| (t!("stats.by_app"), &stats.by_app[..])),
            ]
            .into_iter()
            .flatten()
            .collect(),
        };
        let heatmap_width = if pies.is_empty() {
            Constraint::Fill(1)
        } else {
            Constraint::Length(2 + 2 * (ACTIVITY_DAYS / 7) as u16)
        };
        let areas = Layout::horizontal(
            pies.iter()
                .map(|_| Constraint::Fill(1))
                .chain([heatmap_width]),
        )
        .split(charts_area);
        for ((title, data), area) in pies.iter().zip(areas.iter()) {
            render_pie(frame, *area, app, title, data);
        }
        render_activity(frame, areas[pies.len()], app);
    }
    render_sessions(frame, sessions_area, app);

    command_line::render(frame, command, app);
    status_bar::render(frame, status, app);
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
    let (w, h) = (pie_area.width as i32 * 2, pie_area.height as i32 * 4);
    let (cx, cy, r) = (
        (w - 1) as f64 / 2.0,
        (h - 1) as f64 / 2.0,
        w.min(h) as f64 / 2.0,
    );
    let mut dots = vec![Vec::new(); slices.len()];
    for x in 0..w {
        for y in 0..h {
            let (dx, dy) = (x as f64 - cx, y as f64 - cy);
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
            dots[slice].push((x as f64, y as f64));
        }
    }
    let pie = Canvas::default()
        .marker(Marker::Braille)
        .x_bounds([0.0, (w - 1) as f64])
        .y_bounds([0.0, (h - 1) as f64])
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
                Span::styled("● ", Style::new().fg(color(i))),
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
    let theme = &app.theme;
    let daily = &app.stats.daily;
    let title = t!("stats.activity", weeks = ACTIVITY_DAYS / 7);
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
            "Temps par app",
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
    fn heatmap_levels() {
        assert_eq!(super::level(0, 100), 0);
        assert_eq!(super::level(1, 100), 1);
        assert_eq!(super::level(25, 100), 1);
        assert_eq!(super::level(26, 100), 2);
        assert_eq!(super::level(100, 100), 4);
    }
}
