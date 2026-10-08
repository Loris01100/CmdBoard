use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::App;
use crate::optimize::Bench;
use crate::storage::{models::Activity, unix_now};
use crate::ui::{
    icons, layout,
    screens::optimize::format_score,
    widgets::{
        app_table, category_list, command_line, format_ago, format_clock, format_duration,
        profile_panel, status_bar, xp_bar,
    },
};

pub fn draw(frame: &mut Frame, app: &App) {
    let areas = layout::dashboard(frame.area(), command_line::height(app));

    render_header(frame, areas.header, app);
    category_list::render(frame, areas.categories, app);
    app_table::render(frame, areas.apps, app);
    if let Some(details) = areas.details {
        render_details(frame, details, app);
    }
    if let Some(profile) = areas.profile {
        profile_panel::render(frame, profile, app);
    }
    if let Some(activity) = areas.activity {
        render_activity(frame, activity, app);
    }
    command_line::render(frame, areas.command, app);
    status_bar::render(frame, areas.status, app);
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let title = Line::from(vec![
        Span::styled(" CmdBoard", theme.title),
        Span::styled(format!(" · {}", app.screen.title()), theme.muted()),
    ]);
    frame.render_widget(Paragraph::new(title), area);
    frame.render_widget(Paragraph::new(session_line(app).right_aligned()), area);
}

/// Live timer of each running session, oldest first. Reads the clock, not `app`'s state.
fn session_line(app: &App) -> Line<'static> {
    let theme = &app.theme;
    let mut sessions: Vec<_> = app
        .active_sessions
        .iter()
        .filter_map(|(id, s)| Some((app.apps.iter().find(|a| a.id == *id)?, s)))
        .collect();
    if sessions.is_empty() {
        return Line::styled(t!("session.none_running"), theme.muted());
    }
    sessions.sort_by_key(|(_, session)| session.started);
    let mut spans = Vec::new();
    for (entry, session) in sessions {
        spans.push(Span::styled(
            format!("{} ", icons::SESSION),
            Style::new().fg(theme.success),
        ));
        spans.push(Span::raw(format!("{} ", entry.name)));
        spans.push(Span::styled(
            format!("{}  ", format_clock(session.shown_secs())),
            theme.title,
        ));
        if session.idle {
            spans.push(Span::styled(
                format!("({})  ", t!("session.idle")),
                theme.muted(),
            ));
        }
    }
    Line::from(spans)
}

fn render_details(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let block = theme.panel(&t!("dashboard.details"), false);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(entry) = app.selected_app() else {
        frame.render_widget(
            Paragraph::new(t!("dashboard.no_app")).style(theme.muted()),
            inner,
        );
        return;
    };

    let [head, gauge, rest] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);

    let (level, xp) = app.shown_app_xp(entry);
    let head_text = vec![
        Line::styled(
            entry.name.as_str(),
            theme.title.add_modifier(Modifier::BOLD),
        ),
        Line::from(t!("dashboard.level", level)),
    ];
    frame.render_widget(Paragraph::new(head_text), head);
    frame.render_widget(xp_bar::gauge(level, xp, theme), gauge);

    let last = entry
        .last_played
        .map_or_else(|| t!("never"), |t| format_ago(unix_now() - t));
    let rest_text = vec![
        Line::from(""),
        Line::from(t!(
            "dashboard.total_time",
            time = format_duration(entry.total_secs)
        )),
        Line::from(t!("dashboard.last", last)),
        Line::from(t!(
            "dashboard.rewards",
            icon = icons::TROPHY,
            count = entry.rewards
        )),
        Line::from(""),
        Line::styled(
            t!("dashboard.target", target = entry.launch_target),
            theme.muted(),
        ),
        Line::styled(
            t!(
                "dashboard.process",
                exe = entry.watch_exe.as_deref().unwrap_or("—")
            ),
            theme.muted(),
        ),
    ];
    frame.render_widget(Paragraph::new(rest_text), rest);
}

/// Space between two events of the ticker.
const GAP: &str = "     ";

/// Latest sessions and rewards, then this run's benchmarks. When they do not fit, they
/// scroll right to left in a loop, one cell per tick.
fn render_activity(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let block = theme.panel(&t!("dashboard.activity"), false);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let now = unix_now();
    let mut spans = Vec::new();
    for event in &app.activity {
        let at = match event {
            Activity::Session { app, secs, xp, at } => {
                spans.push(Span::styled(
                    format!("{} ", icons::SESSION),
                    Style::new().fg(theme.success),
                ));
                spans.push(Span::raw(t!(
                    "dashboard.activity_session",
                    name = app,
                    time = format_duration(*secs),
                    xp
                )));
                at
            }
            Activity::Reward { name, at } => {
                spans.push(Span::raw(format!("{} ", icons::TROPHY)));
                spans.push(Span::styled(name.clone(), theme.title));
                at
            }
        };
        spans.push(Span::styled(
            format!(" · {}", format_ago(now - at)),
            theme.muted(),
        ));
        spans.push(Span::raw(GAP));
    }
    for bench in Bench::ALL {
        if let Some((_, Ok(score))) = app.optimize.results.get(&bench) {
            spans.push(Span::raw(format!("{} ", icons::BENCH)));
            spans.push(Span::raw(format!(
                "{} {}",
                bench.label(),
                format_score(*score)
            )));
            spans.push(Span::raw(GAP));
        }
    }
    if spans.is_empty() {
        let empty = Paragraph::new(t!("dashboard.no_activity")).style(theme.muted());
        frame.render_widget(empty, inner);
        return;
    }

    let cycle = Line::from(spans.clone()).width();
    if cycle - GAP.len() <= inner.width as usize {
        frame.render_widget(Paragraph::new(Line::from(spans)), inner);
        return;
    }
    // Enough copies to fill the panel from any offset within one cycle.
    let copies = inner.width as usize / cycle + 2;
    let offset = (app.frame_count % cycle as u64) as u16;
    let looped: Vec<_> = spans
        .iter()
        .cycle()
        .take(spans.len() * copies)
        .cloned()
        .collect();
    let ticker = Paragraph::new(Line::from(looped)).scroll((0, offset));
    frame.render_widget(ticker, inner);
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::optimize::{Bench, Score};
    use crate::storage::{models::Activity, unix_now};
    use ratatui::{Terminal, backend::TestBackend};

    /// The rows of the dashboard, as text.
    fn rows(app: &App) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
        terminal.draw(|f| crate::ui::draw(f, app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..30)
            .map(|y| (0..80).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    /// The row under the "Activité" title.
    fn ticker(app: &App) -> String {
        let rows = rows(app);
        let title = rows.iter().position(|r| r.contains("Activité")).unwrap();
        rows[title + 1].clone()
    }

    #[test]
    fn activity_scrolls_only_when_too_long() {
        let mut app = App::with_defaults();
        app.activity.clear();
        assert!(ticker(&app).contains("Aucune activité"));

        app.activity = vec![Activity::Reward {
            name: "Marathon (Hades)".into(),
            at: unix_now(),
        }];
        app.optimize
            .on_finished(Bench::CpuSingle, false, Ok(Score::Ops(1234.0)));
        let still = ticker(&app);
        assert!(still.contains("Marathon (Hades)") && still.contains("1234 M op/s"));
        app.frame_count = 5;
        assert_eq!(ticker(&app), still, "fits: does not move");

        for i in 0..5 {
            app.activity.push(Activity::Session {
                app: format!("Application numéro {i}"),
                secs: 5_400,
                xp: 90,
                at: unix_now() - 7_200,
            });
        }
        // Past the leading 🏆, which holds for a tick as it is two cells wide.
        app.frame_count = 3;
        let start = ticker(&app);
        app.frame_count = 4;
        let next = ticker(&app);
        assert_ne!(next, start, "too long: scrolls each tick");
        // One cell to the left: the start without its first cell (│ border, then text).
        let inner = |row: &str| row.chars().skip(1).take(70).collect::<String>();
        assert_eq!(
            inner(&next).chars().take(60).collect::<String>(),
            inner(&start).chars().skip(1).take(60).collect::<String>()
        );
    }
}
