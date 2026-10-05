use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    widgets::Paragraph,
};

use super::xp_bar;
use crate::app::App;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let profile = &app.profile;

    let block = theme.panel("Profil", false);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let (shown_level, shown_xp) = app.shown_profile_xp();
    let level = format!("Niv {shown_level} ");
    let stats = format!(
        " │ Streak {}j │ XP du jour : +{}",
        profile.streak_days, profile.xp_today
    );
    let [level_area, gauge_area, stats_area] = Layout::horizontal([
        Constraint::Length(level.chars().count() as u16),
        Constraint::Min(10),
        Constraint::Length(stats.chars().count() as u16),
    ])
    .areas(inner);

    frame.render_widget(Paragraph::new(level).style(theme.title), level_area);
    frame.render_widget(xp_bar::gauge(shown_level, shown_xp, theme), gauge_area);
    frame.render_widget(Paragraph::new(stats), stats_area);
}
