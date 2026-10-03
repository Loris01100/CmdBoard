use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    widgets::Paragraph,
};

use super::xp_bar;
use crate::app::App;
use crate::core::xp;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let profile = &app.profile;

    let block = theme.panel("Profil", false);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let level = format!("Niv {} ", profile.level);
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
    frame.render_widget(
        xp_bar::gauge(xp::level_progress(profile.level, profile.xp), theme),
        gauge_area,
    );
    frame.render_widget(Paragraph::new(stats), stats_area);
}
