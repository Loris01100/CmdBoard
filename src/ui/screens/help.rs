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

fn keys() -> [(String, String); 10] {
    [
        ("j/k ↑↓".into(), t!("help.navigate")),
        ("Tab ←→ h/l".into(), t!("help.panel")),
        (t!("keys.enter"), t!("help.launch")),
        ("a / m / d".into(), t!("help.edit")),
        ("s".into(), t!("help.sort")),
        ("/".into(), t!("help.search")),
        ("Tab s d".into(), t!("help.storage")),
        (":".into(), t!("help.command")),
        ("1 2 3 4 5 ?".into(), t!("help.screens")),
        ("q  Ctrl-C".into(), t!("help.quit")),
    ]
}

pub fn draw(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let (body, command, status) = layout::screen(frame.area(), command_line::height(app));
    let block = theme.panel(&t!("screen.help"), true);
    let inner = block.inner(body);
    frame.render_widget(block, body);

    let [keys_area, commands_area] = ratatui::layout::Layout::vertical([
        Constraint::Length(keys().len() as u16 + 2),
        Constraint::Min(0),
    ])
    .areas(inner);

    let mut lines = vec![Line::styled(t!("help.keys"), theme.title)];
    lines.extend(keys().into_iter().map(|(key, what)| {
        Line::from(vec![
            Span::styled(format!("  {key:<12}"), theme.title),
            Span::raw(what),
        ])
    }));
    frame.render_widget(Paragraph::new(lines), keys_area);

    let commands = COMMANDS.iter().map(|c| {
        let aliases = if c.aliases.is_empty() {
            String::new()
        } else {
            format!(" ({})", c.aliases.join(", "))
        };
        Row::new([
            Cell::from(Span::styled(format!("  :{}", c.usage()), theme.title)),
            Cell::from(Span::styled(aliases, theme.muted())),
            Cell::from(c.summary()),
        ])
    });
    // User aliases from commands.toml, after the built-in commands.
    let user_aliases = app.aliases.iter().map(|(name, body)| {
        Row::new([
            Cell::from(Span::styled(format!("  :{name}"), theme.title)),
            Cell::from(Span::styled(t!("help.alias"), theme.muted())),
            Cell::from(body.to_string()),
        ])
    });
    let rows = commands.chain(user_aliases);
    let table = Table::new(
        rows,
        [
            Constraint::Length(34),
            Constraint::Length(9),
            Constraint::Min(10),
        ],
    )
    .header(Row::new([Line::styled(t!("help.commands"), theme.title)]));
    frame.render_widget(table, commands_area);

    command_line::render(frame, command, app);
    status_bar::render(frame, status, app);
}
