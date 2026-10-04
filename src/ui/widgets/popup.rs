use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
};

use crate::app::App;
use crate::popup::{Form, LevelUp, Popup};

const MAX_WIDTH: u16 = 64;

/// Draws the popup centered over the current screen.
pub fn render(frame: &mut Frame, popup: &Popup, app: &App) {
    match popup {
        Popup::Confirm { message, .. } => render_confirm(frame, message, app),
        Popup::Form(form) => render_form(frame, form, app),
        Popup::LevelUp(level_up) => render_level_up(frame, level_up, app),
    }
}

/// Level-up announcement. Its border and title alternate colors on each tick.
fn render_level_up(frame: &mut Frame, level_up: &LevelUp, app: &App) {
    let theme = &app.theme;
    let accent = if app.frame_count % 2 == 0 { theme.success } else { theme.info };
    let accent_style = Style::new().fg(accent).add_modifier(Modifier::BOLD);

    let mut text = vec![Line::styled("★  Niveau supérieur !  ★", accent_style).centered(), Line::from("")];
    if let Some(level) = level_up.app_level {
        text.push(Line::from(vec![
            Span::styled(level_up.app.clone(), theme.title),
            Span::raw(format!(" passe au niveau {level}")),
        ]).centered());
    }
    if let Some(level) = level_up.global_level {
        text.push(Line::from(format!("Profil : niveau {level}")).centered());
    }
    text.push(Line::styled(format!("+{} XP", level_up.gained), Style::new().fg(theme.xp_fill)).centered());
    text.push(Line::from(""));
    text.push(Line::styled("Entrée pour continuer", theme.muted()).centered());

    let area = centered(frame.area(), popup_width(frame.area()).min(44), text.len() as u16 + 2);
    let block = theme.panel("Level-up", true).border_style(Style::new().fg(accent));
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(text).block(block), area);
}

fn render_confirm(frame: &mut Frame, message: &str, app: &App) {
    let theme = &app.theme;
    let width = popup_width(frame.area());
    // Rough wrapped height: the message is plain prose.
    let text_width = width.saturating_sub(2).max(1) as usize;
    let message_lines = message.chars().count().div_ceil(text_width) as u16;
    let area = centered(frame.area(), width, message_lines + 4);

    let block = theme
        .panel("Confirmer", true)
        .border_style(Style::new().fg(theme.error));
    let text = vec![
        Line::from(message.to_string()),
        Line::from(""),
        Line::from(vec![
            Span::styled("Entrée/o", theme.title),
            Span::raw(" oui   "),
            Span::styled("Esc/n", theme.title),
            Span::raw(" non"),
        ]),
    ];
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(text).block(block).wrap(Wrap { trim: true }),
        area,
    );
}

fn render_form(frame: &mut Frame, form: &Form, app: &App) {
    let theme = &app.theme;
    let rows = form.fields.len() as u16;
    let area = centered(frame.area(), popup_width(frame.area()), rows + 4);
    let block = theme.panel(&form.title(), true);
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    let label_width = form
        .fields
        .iter()
        .map(|f| f.label.chars().count())
        .max()
        .unwrap_or(0)
        + 4; // "> " marker and " " padding, plus "*" for required fields

    for (i, field) in form.fields.iter().enumerate() {
        let y = inner.y + i as u16;
        if y >= inner.bottom() {
            break;
        }
        let focused = i == form.focused;
        let marker = if focused { "> " } else { "  " };
        let required = if field.required { "*" } else { " " };
        let label = format!("{marker}{:<w$}", format!("{}{required}", field.label), w = label_width - 2);
        let label_style = if focused { theme.title } else { theme.muted() };

        let room = (inner.width as usize).saturating_sub(label_width);
        let (visible, cursor) = field.input.view(room);
        let value = match form.placeholder(i) {
            Some(placeholder) if field.input.is_empty() => {
                Span::styled(placeholder.chars().take(room).collect::<String>(), theme.muted())
            }
            _ => Span::raw(visible),
        };
        let row = Rect::new(inner.x, y, inner.width, 1);
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(label, label_style), value])),
            row,
        );
        if focused && room > 0 {
            let x = inner.x + (label_width + cursor) as u16;
            frame.set_cursor_position(Position::new(x, y));
        }
    }

    // Last row: the error, or how to submit.
    let footer = match &form.error {
        Some(error) => Line::styled(format!("  {error}"), Style::new().fg(theme.error)),
        None => Line::styled("  * requis · Entrée sur le dernier champ : valider", theme.muted()),
    };
    if inner.height > rows + 1 {
        let y = inner.y + rows + 1;
        frame.render_widget(Paragraph::new(footer), Rect::new(inner.x, y, inner.width, 1));
    }
}

fn popup_width(area: Rect) -> u16 {
    MAX_WIDTH.min(area.width.saturating_sub(4)).max(area.width.min(20))
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [row] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [rect] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(row);
    rect
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Mode;
    use ratatui::{Terminal, backend::TestBackend};

    fn screen(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| crate::ui::draw(f, app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn form_renders_fields_and_error() {
        let mut app = App::with_defaults();
        let mut form = Form::add_app("Jeux");
        form.error = Some("Nom : champ requis".into());
        app.mode = Mode::Popup(Popup::Form(form));
        let text = screen(&app, 100, 30);
        for expected in ["Ajouter une app", "> Nom*", "Catégorie*", "Jeux", "Nom : champ requis"] {
            assert!(text.contains(expected), "missing {expected:?} in\n{text}");
        }
    }

    #[test]
    fn confirm_renders_question() {
        let mut app = App::with_defaults();
        app.execute(crate::command::Command::RemoveApp {
            app: "Steam".into(),
            confirmed: false,
        });
        let text = screen(&app, 100, 30);
        assert!(text.contains("Confirmer"));
        assert!(text.contains("Supprimer « Steam »"));
    }

    #[test]
    fn level_up_renders_both_levels() {
        let mut app = App::with_defaults();
        app.mode = Mode::Popup(Popup::LevelUp(LevelUp {
            app: "Steam".into(),
            app_level: Some(3),
            global_level: Some(2),
            gained: 120,
        }));
        let text = screen(&app, 100, 30);
        for expected in ["Niveau supérieur", "Steam passe au niveau 3", "Profil : niveau 2", "+120 XP"] {
            assert!(text.contains(expected), "missing {expected:?} in\n{text}");
        }
    }

    #[test]
    fn popups_fit_tiny_terminals() {
        let mut app = App::with_defaults();
        let level_up = Popup::LevelUp(LevelUp {
            app: "Steam".into(),
            app_level: Some(2),
            global_level: None,
            gained: 100,
        });
        for popup in [Popup::Form(Form::add_app("Jeux")), level_up] {
            app.mode = Mode::Popup(popup);
            for (w, h) in [(30, 8), (10, 3), (1, 1)] {
                screen(&app, w, h);
            }
        }
    }
}
