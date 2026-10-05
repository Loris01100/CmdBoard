use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::App;
use crate::core::xp;
use crate::storage::unix_now;
use crate::ui::{
    layout,
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
    if let Some(rewards) = areas.rewards {
        render_recent_rewards(frame, rewards, app);
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
        .filter_map(|(id, s)| Some((app.apps.iter().find(|a| a.id == *id)?, s.started)))
        .collect();
    if sessions.is_empty() {
        return Line::styled("aucune session en cours ", theme.muted());
    }
    sessions.sort_by_key(|(_, started)| *started);
    let mut spans = Vec::new();
    for (entry, started) in sessions {
        spans.push(Span::styled("▶ ", Style::new().fg(theme.success)));
        spans.push(Span::raw(format!("{} ", entry.name)));
        spans.push(Span::styled(
            format!("{}  ", format_clock(started.elapsed().as_secs())),
            theme.title,
        ));
    }
    Line::from(spans)
}

fn render_details(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let block = theme.panel("Détails", false);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(entry) = app.selected_app() else {
        frame.render_widget(
            Paragraph::new("Aucune application").style(theme.muted()),
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
        Line::from(format!(
            "Niveau {level}  ·  {xp}/{} XP",
            xp::xp_to_next_level(level)
        )),
    ];
    frame.render_widget(Paragraph::new(head_text), head);
    frame.render_widget(xp_bar::gauge(xp::level_progress(level, xp), theme), gauge);

    let last = entry
        .last_played
        .map_or_else(|| "jamais".into(), |t| format_ago(unix_now() - t));
    let rest_text = vec![
        Line::from(""),
        Line::from(format!(
            "Temps total : {}",
            format_duration(entry.total_secs)
        )),
        Line::from(format!("Dernière : {last}")),
        Line::from(format!("Récompenses : 🏆 {}", entry.rewards)),
        Line::from(""),
        Line::styled(format!("Cible : {}", entry.launch_target), theme.muted()),
        Line::styled(
            format!("Process : {}", entry.watch_exe.as_deref().unwrap_or("—")),
            theme.muted(),
        ),
    ];
    frame.render_widget(Paragraph::new(rest_text), rest);
}

fn render_recent_rewards(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let text = if app.recent_rewards.is_empty() {
        Line::styled("Aucune récompense pour le moment", theme.muted())
    } else {
        Line::from(
            app.recent_rewards
                .iter()
                .map(|r| format!("🏆 {r}"))
                .collect::<Vec<_>>()
                .join("   "),
        )
    };
    frame.render_widget(
        Paragraph::new(text).block(theme.panel("Dernières récompenses", false)),
        area,
    );
}
