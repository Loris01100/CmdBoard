use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::App;
use crate::core::xp;
use crate::ui::{
    layout,
    widgets::{app_table, category_list, format_duration, profile_panel, status_bar, xp_bar},
};

pub fn draw(frame: &mut Frame, app: &App) {
    let areas = layout::dashboard(frame.area());

    render_header(frame, areas.header, app);
    category_list::render(frame, areas.categories, app);
    app_table::render(frame, areas.apps, app);
    if let Some(details) = areas.details {
        render_details(frame, details, app);
    }
    profile_panel::render(frame, areas.profile, app);
    render_recent_rewards(frame, areas.rewards, app);
    // `areas.command` stays empty until the command line exists (step 5).
    status_bar::render(frame, areas.status, app);
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let title = Line::from(vec![
        Span::styled(" CmdBoard", theme.title),
        Span::styled(format!(" · {}", app.screen.title()), theme.muted()),
    ]);
    // Live session timer comes with the tracker (step 7).
    let session = Line::styled("aucune session en cours ", theme.muted()).right_aligned();
    frame.render_widget(Paragraph::new(title), area);
    frame.render_widget(Paragraph::new(session), area);
}

fn render_details(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let block = theme.panel("Détails", false);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(entry) = app.selected_app() else {
        frame.render_widget(Paragraph::new("Aucune application").style(theme.muted()), inner);
        return;
    };

    let [head, gauge, rest] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);

    let head_text = vec![
        Line::styled(entry.name.as_str(), theme.title.add_modifier(Modifier::BOLD)),
        Line::from(format!("Niveau {}", entry.level)),
    ];
    frame.render_widget(Paragraph::new(head_text), head);
    frame.render_widget(xp_bar::gauge(xp::level_progress(entry.level, entry.xp), theme), gauge);

    let last = entry.last_played.as_deref().unwrap_or("jamais");
    let rest_text = vec![
        Line::from(""),
        Line::from(format!("Temps total : {}", format_duration(entry.total_secs))),
        Line::from(format!("Dernière : {last}")),
        Line::from(format!("Récompenses : 🏆 {}", entry.rewards)),
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
