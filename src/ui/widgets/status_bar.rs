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
    let hints: &[(&str, &str)] = match (&app.mode, app.screen) {
        (Mode::Command, _) => &[
            ("Entrée", "valider"),
            ("Esc", "annuler"),
            ("↑↓", "historique"),
            ("Tab", "compléter"),
        ],
        (Mode::Search, _) => &[
            ("↑↓", "choisir"),
            ("Entrée", "aller à l'app"),
            ("Esc", "annuler"),
        ],
        (Mode::Popup(Popup::Confirm { .. }), _) => {
            &[("Entrée/o", "confirmer"), ("Esc/n", "annuler")]
        }
        (Mode::Popup(Popup::LevelUp(_) | Popup::RewardUnlocked(_)), _) => {
            &[("Entrée/Esc", "continuer")]
        }
        (Mode::Popup(Popup::Form(_)), _) => &[
            ("Tab ↑↓", "champ"),
            ("Entrée", "suivant / valider"),
            ("Esc", "annuler"),
        ],
        (Mode::Normal, Screen::Dashboard) => &[
            ("j/k", "naviguer"),
            ("Tab", "panneau"),
            ("Entrée", "lancer"),
            ("a/m/d", "ajouter/déplacer/suppr."),
            ("/", "chercher"),
            (":", "commande"),
            ("q", "quitter"),
        ],
        (Mode::Normal, Screen::Rewards | Screen::Stats) => &[
            ("j/k", "naviguer"),
            (":", "commande"),
            ("1-4", "écrans"),
            ("q", "quitter"),
        ],
        (Mode::Normal, _) => &[(":", "commande"), ("1-4", "écrans"), ("q", "quitter")],
    };

    let mut spans = vec![Span::raw(" ")];
    for (i, (key, label)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(*key, theme.title));
        spans.push(Span::styled(format!(" {label}"), theme.muted()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    if let Some(version) = &app.update_available {
        let notice = Line::from(vec![
            Span::styled(
                format!("v{version} disponible "),
                Style::new().fg(theme.info),
            ),
            Span::styled(":update ", theme.muted()),
        ]);
        frame.render_widget(Paragraph::new(notice).alignment(Alignment::Right), area);
    }
}
