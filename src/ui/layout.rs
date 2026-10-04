use ratatui::layout::{Constraint, Layout, Rect};

/// Below this width the details panel is hidden.
const DETAILS_MIN_WIDTH: u16 = 90;
/// Below this width the categories sit above the apps instead of beside them.
const STACK_WIDTH: u16 = 60;
/// Below these heights the recent rewards, then the profile, are hidden.
const REWARDS_MIN_HEIGHT: u16 = 24;
const PROFILE_MIN_HEIGHT: u16 = 18;
/// Rows of the categories list when stacked (borders included).
const STACKED_CATEGORIES_HEIGHT: u16 = 5;

pub struct DashboardLayout {
    pub header: Rect,
    pub categories: Rect,
    pub apps: Rect,
    pub details: Option<Rect>,
    pub profile: Option<Rect>,
    pub rewards: Option<Rect>,
    pub command: Rect,
    pub status: Rect,
}

/// `command_height`: 1 row for messages, more while the command box is open.
/// The body (lists) keeps the room: secondary panels go first on small terminals.
pub fn dashboard(area: Rect, command_height: u16) -> DashboardLayout {
    let panel = |min_height: u16| if area.height >= min_height { 3 } else { 0 };
    let [header, body, profile, rewards, command, status] = Layout::vertical([
        Constraint::Length(1),                         // header
        Constraint::Min(5),                            // body
        Constraint::Length(panel(PROFILE_MIN_HEIGHT)), // profile
        Constraint::Length(panel(REWARDS_MIN_HEIGHT)), // recent rewards
        Constraint::Length(command_height),            // command box / message
        Constraint::Length(1),                         // status
    ])
    .areas(area);
    let shown = |rect: Rect| (rect.height > 0).then_some(rect);

    let (categories, apps, details) = if area.width < STACK_WIDTH {
        let [categories, apps] = Layout::vertical([
            Constraint::Length(STACKED_CATEGORIES_HEIGHT),
            Constraint::Min(0),
        ])
        .areas(body);
        (categories, apps, None)
    } else if area.width >= DETAILS_MIN_WIDTH {
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
        profile: shown(profile),
        rewards: shown(rewards),
        command,
        status,
    }
}

/// Main area, command area and status bar, for screens other than the dashboard.
pub fn screen(area: Rect, command_height: u16) -> (Rect, Rect, Rect) {
    let [body, command, status] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(command_height),
        Constraint::Length(1),
    ])
    .areas(area);
    (body, command, status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panels_adapt_to_size() {
        let wide = dashboard(Rect::new(0, 0, 120, 30), 1);
        assert!(wide.details.is_some() && wide.profile.is_some() && wide.rewards.is_some());
        assert_eq!(wide.categories.y, wide.apps.y); // side by side

        let narrow = dashboard(Rect::new(0, 0, 50, 30), 1);
        assert!(narrow.details.is_none());
        assert_eq!(narrow.categories.x, narrow.apps.x); // stacked
        assert!(narrow.apps.y > narrow.categories.y);

        let short = dashboard(Rect::new(0, 0, 120, 20), 1);
        assert!(short.profile.is_some() && short.rewards.is_none());
        let shorter = dashboard(Rect::new(0, 0, 120, 12), 1);
        assert!(shorter.profile.is_none() && shorter.rewards.is_none());
    }
}
