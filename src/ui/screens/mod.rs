pub mod dashboard;

use ratatui::{Frame, widgets::Paragraph};

use super::{layout, widgets::status_bar};
use crate::app::App;

/// Placeholder for the Stats, Rewards and Help screens (steps 9–10).
pub fn coming_soon(frame: &mut Frame, app: &App) {
    let (body, status) = layout::with_status(frame.area());
    let content = Paragraph::new("Bientôt disponible.")
        .style(app.theme.muted())
        .block(app.theme.panel(app.screen.title(), true));
    frame.render_widget(content, body);
    status_bar::render(frame, status, app);
}
