use ratatui::layout::{Constraint, Layout, Rect};

/// Below this width the details panel is hidden.
const DETAILS_MIN_WIDTH: u16 = 90;

pub struct DashboardLayout {
    pub header: Rect,
    pub categories: Rect,
    pub apps: Rect,
    pub details: Option<Rect>,
    pub profile: Rect,
    pub rewards: Rect,
    pub command: Rect,
    pub status: Rect,
}

pub fn dashboard(area: Rect) -> DashboardLayout {
    let [header, body, profile, rewards, command, status] = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(10),   // body
        Constraint::Length(3), // profile
        Constraint::Length(3), // recent rewards
        Constraint::Length(1), // command line / message
        Constraint::Length(1), // status
    ])
    .areas(area);

    let (categories, apps, details) = if area.width >= DETAILS_MIN_WIDTH {
        let [categories, apps, details] = Layout::horizontal([
            Constraint::Length(20),
            Constraint::Percentage(55),
            Constraint::Min(25),
        ])
        .areas(body);
        (categories, apps, Some(details))
    } else {
        let [categories, apps] =
            Layout::horizontal([Constraint::Length(20), Constraint::Min(0)]).areas(body);
        (categories, apps, None)
    };

    DashboardLayout {
        header,
        categories,
        apps,
        details,
        profile,
        rewards,
        command,
        status,
    }
}

/// Main area, command line row and status bar, for screens other than the dashboard.
pub fn screen(area: Rect) -> (Rect, Rect, Rect) {
    let [body, command, status] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    (body, command, status)
}
