use ratatui::{
    Frame,
    layout::Constraint,
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table},
};

use crate::app::App;
use crate::command::COMMANDS;
use crate::ui::{
    layout,
    widgets::{command_line, status_bar},
};

const KEYS: &[(&str, &str)] = &[
    ("j/k ↑↓", "naviguer"),
    ("Tab ←→ h/l", "changer de panneau"),
    ("Entrée", "lancer l'app sélectionnée"),
    ("a / m / d", "ajouter / déplacer / supprimer (app ou catégorie vide)"),
    ("/", "chercher une app dans toutes les catégories (Entrée : y aller)"),
    (":", "ligne de commande (↑↓ historique, Tab compléter, Esc annuler)"),
    ("1 2 3 4", "Dashboard, Stats, Récompenses, Aide"),
    ("q  Ctrl-C", "quitter"),
];

pub fn draw(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let (body, command, status) = layout::screen(frame.area(), command_line::height(app));
    let block = theme.panel("Aide", true);
    let inner = block.inner(body);
    frame.render_widget(block, body);

    let [keys_area, commands_area] = ratatui::layout::Layout::vertical([
        Constraint::Length(KEYS.len() as u16 + 2),
        Constraint::Min(0),
    ])
    .areas(inner);

    let mut keys = vec![Line::styled("Touches", theme.title)];
    keys.extend(KEYS.iter().map(|(key, what)| {
        Line::from(vec![
            Span::styled(format!("  {key:<12}"), theme.title),
            Span::raw(*what),
        ])
    }));
    frame.render_widget(Paragraph::new(keys), keys_area);

    let commands = COMMANDS.iter().map(|c| {
        let aliases = if c.aliases.is_empty() {
            String::new()
        } else {
            format!(" ({})", c.aliases.join(", "))
        };
        Row::new([
            Cell::from(Span::styled(format!("  :{}", c.usage), theme.title)),
            Cell::from(Span::styled(aliases, theme.muted())),
            Cell::from(c.summary),
        ])
    });
    // User aliases from commands.toml, after the built-in commands.
    let user_aliases = app.aliases.iter().map(|(name, body)| {
        Row::new([
            Cell::from(Span::styled(format!("  :{name}"), theme.title)),
            Cell::from(Span::styled(" alias", theme.muted())),
            Cell::from(body.to_string()),
        ])
    });
    let rows = commands.chain(user_aliases);
    let table = Table::new(
        rows,
        [Constraint::Length(34), Constraint::Length(9), Constraint::Min(10)],
    )
    .header(Row::new([Line::styled("Commandes", theme.title)]));
    frame.render_widget(table, commands_area);

    command_line::render(frame, command, app);
    status_bar::render(frame, status, app);
}
