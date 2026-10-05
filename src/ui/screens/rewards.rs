use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::Style,
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, Wrap},
};

use crate::app::App;
use crate::storage::models::RewardView;
use crate::ui::{
    layout,
    widgets::{command_line, status_bar},
};

/// Every reward: unlocked ones in color, locked ones greyed out. The selected one is
/// detailed below (scope, rule, who unlocked it and when).
pub fn draw(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let (body, command, status) = layout::screen(frame.area(), command_line::height(app));
    // On short terminals the list keeps the room and the detail panel goes.
    let detail_height = if body.height >= 12 { 5 } else { 0 };
    let [list_area, detail_area] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(detail_height)]).areas(body);

    let unlocked = app.rewards.iter().filter(|r| !r.unlocks.is_empty()).count();
    let title = format!("Récompenses ({unlocked}/{})", app.rewards.len());

    let header = Row::new(["", "Nom", "Description", "Débloquée"]).style(theme.title);
    let rows = app.rewards.iter().map(|reward| {
        let (icon, style) = if reward.unlocks.is_empty() {
            ("🔒", theme.muted())
        } else {
            ("🏆", Style::new())
        };
        Row::new([
            Cell::from(icon),
            Cell::from(reward.name.as_str()),
            Cell::from(reward.description.as_str()),
            Cell::from(unlocked_summary(reward)),
        ])
        .style(style)
    });
    let widths = [
        Constraint::Length(2),
        Constraint::Length(16),
        Constraint::Min(10),
        Constraint::Length(12),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(theme.panel(&title, true))
        .row_highlight_style(theme.highlight(true))
        .highlight_symbol("> ");
    // Rendering needs `&mut TableState`; work on a copy so `draw` stays pure.
    let mut state = app.reward_state.clone();
    frame.render_stateful_widget(table, list_area, &mut state);

    if detail_height > 0 {
        let selected = app.reward_state.selected().and_then(|i| app.rewards.get(i));
        let detail = match selected {
            Some(reward) => detail_lines(reward, app),
            None => vec![Line::styled("Aucune récompense définie", theme.muted())],
        };
        frame.render_widget(
            Paragraph::new(detail)
                .wrap(Wrap { trim: true })
                .block(theme.panel("Détail", false)),
            detail_area,
        );
    }

    command_line::render(frame, command, app);
    status_bar::render(frame, status, app);
}

/// Last column: the date, how many apps unlocked a per-app reward, or "—".
fn unlocked_summary(reward: &RewardView) -> String {
    match reward.unlocks.as_slice() {
        [] => "—".into(),
        [only] if !reward.per_app || reward.app.is_some() => only.date.clone(),
        unlocks => format!("{} app(s)", unlocks.len()),
    }
}

fn detail_lines(reward: &RewardView, app: &App) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let scope = match (&reward.app, reward.per_app) {
        (Some(app), _) => format!("pour {app}"),
        (None, true) => "une fois par app".into(),
        (None, false) => "globale".into(),
    };
    let unlocks = if reward.unlocks.is_empty() {
        Span::styled("verrouillée", theme.muted())
    } else {
        let list = reward
            .unlocks
            .iter()
            .map(|u| match &u.app {
                Some(app) => format!("{app} ({})", u.date),
                None => u.date.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        Span::styled(list, Style::new().fg(theme.success))
    };
    vec![
        Line::from(vec![
            Span::styled(reward.name.clone(), theme.title),
            Span::styled(format!("  ·  {scope}"), theme.muted()),
        ]),
        Line::from(vec![
            Span::styled("Condition : ", theme.muted()),
            Span::raw(reward.rule.clone()),
        ]),
        Line::from(vec![Span::styled("Débloquée : ", theme.muted()), unlocks]),
    ]
}

#[cfg(test)]
mod tests {
    use crate::app::{App, Screen};
    use ratatui::{Terminal, backend::TestBackend};

    fn screen(app: &App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(110, 30)).unwrap();
        terminal.draw(|f| crate::ui::draw(f, app)).unwrap();
        let buffer = terminal.backend().buffer();
        buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    #[test]
    fn lists_locked_and_unlocked_rewards() {
        let mut app = App::with_defaults();
        app.screen = Screen::Rewards;
        let total = app.rewards.len();
        let text = screen(&app);
        assert!(text.contains(&format!("Récompenses (0/{total})")), "{text}");
        assert!(text.contains("Premiers pas"));
        assert!(text.contains("Condition : total_sessions >= 1"));
        assert!(text.contains("verrouillée"));

        let steam = app.find_app("Steam").unwrap().id;
        app.on_session_start(steam);
        app.on_session_end(steam, 600);
        app.mode = crate::app::Mode::Normal; // dismiss the popups
        let text = screen(&app);
        // "Premiers pas" (plus "Noctambule" when the test runs at night).
        assert!(
            !text.contains(&format!("Récompenses (0/{total})")),
            "{text}"
        );
        assert!(!text.contains("verrouillée")); // "Premiers pas" is selected and unlocked
    }
}
