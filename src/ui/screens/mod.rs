pub mod dashboard;
pub mod help;

use ratatui::{Frame, widgets::Paragraph};

use super::{
    layout,
    widgets::{command_line, status_bar},
};
use crate::app::App;

/// Placeholder for the Stats and Rewards screens (steps 9–10).
pub fn coming_soon(frame: &mut Frame, app: &App) {
    let (body, command, status) = layout::screen(frame.area(), command_line::height(app));
    let content = Paragraph::new("Bientôt disponible.")
        .style(app.theme.muted())
        .block(app.theme.panel(app.screen.title(), true));
    frame.render_widget(content, body);
    command_line::render(frame, command, app);
    status_bar::render(frame, status, app);
}
