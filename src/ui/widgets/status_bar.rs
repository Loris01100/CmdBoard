use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::{App, Mode, Screen};
use crate::popup::Popup;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let mut spans = vec![Span::raw(" ")];
    for (i, (key, label)) in hints(app).into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(key, theme.title));
        let label = crate::i18n::tr(label, &[]);
        spans.push(Span::styled(format!(" {label}"), theme.muted()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    if let Some(version) = &app.update_available {
        let notice = Line::from(vec![
            Span::styled(t!("update.available", version), Style::new().fg(theme.info)),
            Span::styled(":update ", theme.muted()),
        ]);
        frame.render_widget(Paragraph::new(notice).alignment(Alignment::Right), area);
    }
}

/// Keys of the current mode and screen: `(key, i18n key of what it does)`.
fn hints(app: &App) -> Vec<(String, &'static str)> {
    let enter = t!("keys.enter");
    match (&app.mode, app.screen) {
        (Mode::Command, _) => vec![
            (enter, "hint.confirm"),
            ("Esc".into(), "hint.cancel"),
            ("↑↓".into(), "hint.history"),
            ("Tab".into(), "hint.complete"),
        ],
        (Mode::Search, _) => vec![
            ("↑↓".into(), "hint.choose"),
            (enter, "hint.go_to_app"),
            ("Esc".into(), "hint.cancel"),
        ],
        (Mode::Popup(Popup::Confirm { .. }), _) => vec![
            (t!("keys.confirm_keys"), "hint.yes"),
            ("Esc/n".into(), "hint.cancel"),
        ],
        (Mode::Popup(Popup::LevelUp(_) | Popup::RewardUnlocked(_)), _) => {
            vec![(format!("{enter}/Esc"), "hint.continue")]
        }
        (Mode::Popup(Popup::Picker(_)), _) => vec![
            ("↑↓".into(), "hint.choose"),
            (enter, "hint.fill"),
            ("Tab".into(), "hint.manual"),
            ("Esc".into(), "hint.cancel"),
        ],
        (Mode::Popup(Popup::Form(_)), _) => vec![
            ("Tab ↑↓".into(), "hint.field"),
            (enter, "hint.next_or_submit"),
            ("Esc".into(), "hint.cancel"),
        ],
        (Mode::Normal, Screen::Dashboard) => vec![
            ("j/k".into(), "hint.navigate"),
            ("Tab".into(), "hint.panel"),
            (enter, "hint.launch"),
            ("a/e/m/d".into(), "hint.edit"),
            ("/".into(), "hint.search"),
            (":".into(), "hint.command"),
            ("q".into(), "hint.quit"),
        ],
        (Mode::Normal, Screen::Storage) if app.storage.folders.is_some() => vec![
            ("j/k".into(), "hint.navigate"),
            (format!("{enter}/→"), "hint.open"),
            ("←".into(), "hint.parent"),
            ("s".into(), "hint.order"),
            ("d".into(), "hint.delete"),
            ("f".into(), "hint.programs"),
            ("q".into(), "hint.quit"),
        ],
        (Mode::Normal, Screen::Storage) => vec![
            ("j/k".into(), "hint.navigate"),
            ("Tab".into(), "hint.disk"),
            ("s".into(), "hint.order"),
            ("d".into(), "hint.uninstall"),
            ("f".into(), "hint.folders"),
            (":".into(), "hint.command"),
            ("0-5".into(), "hint.screens"),
            ("q".into(), "hint.quit"),
        ],
        (Mode::Normal, Screen::Optimize) if app.optimize.gaming_focus => vec![
            ("j/k".into(), "hint.navigate"),
            ("Tab".into(), "hint.panel"),
            (enter, "hint.switch"),
            ("o".into(), "hint.windows_page"),
            ("0-5".into(), "hint.screens"),
            ("q".into(), "hint.quit"),
        ],
        (Mode::Normal, Screen::Optimize) => vec![
            ("j/k".into(), "hint.navigate"),
            ("Tab".into(), "hint.panel"),
            (enter, "hint.run"),
            ("n".into(), "hint.level"),
            ("0-5".into(), "hint.screens"),
            ("q".into(), "hint.quit"),
        ],
        (Mode::Normal, Screen::Rewards | Screen::Stats) => vec![
            ("j/k".into(), "hint.navigate"),
            (":".into(), "hint.command"),
            ("0-5".into(), "hint.screens"),
            ("q".into(), "hint.quit"),
        ],
        (Mode::Normal, _) => vec![
            (":".into(), "hint.command"),
            ("0-5".into(), "hint.screens"),
            ("q".into(), "hint.quit"),
        ],
    }
}
