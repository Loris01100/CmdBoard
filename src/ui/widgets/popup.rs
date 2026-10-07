use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
};

use crate::app::App;
use crate::popup::{Form, LevelUp, Picker, Popup, RewardUnlocked};
use crate::ui::icons;

const MAX_WIDTH: u16 = 64;

/// Draws the popup centered over the current screen.
pub fn render(frame: &mut Frame, popup: &Popup, app: &App) {
    match popup {
        Popup::Confirm { message, .. } => render_confirm(frame, message, app),
        Popup::Picker(picker) => render_picker(frame, picker, app),
        Popup::Form(form) => render_form(frame, form, app),
        Popup::LevelUp(level_up) => render_level_up(frame, level_up, app),
        Popup::RewardUnlocked(reward) => render_reward(frame, reward, app),
    }
}

/// Announces an unlocked reward. Blinks like the level-up popup.
fn render_reward(frame: &mut Frame, reward: &RewardUnlocked, app: &App) {
    let theme = &app.theme;
    let accent = blink(app);
    let accent_style = Style::new().fg(accent).add_modifier(Modifier::BOLD);

    let mut text = vec![
        Line::styled(t!("popup.reward_unlocked"), accent_style).centered(),
        Line::from(""),
        Line::styled(format!("{} {}", icons::TROPHY, reward.name), theme.title).centered(),
        Line::from(reward.description.clone()).centered(),
    ];
    if let Some(app_name) = &reward.app {
        text.push(Line::styled(format!("({app_name})"), theme.muted()).centered());
    }
    text.push(Line::from(""));
    text.push(Line::styled(t!("popup.continue"), theme.muted()).centered());

    let area = centered(
        frame.area(),
        popup_width(frame.area()).min(50),
        text.len() as u16 + 2,
    );
    let block = theme
        .panel(&t!("popup.reward_title"), true)
        .border_style(Style::new().fg(accent));
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(text).block(block), area);
}

/// Gold and green, alternating on each tick: the level-up and reward popups blink.
fn blink(app: &App) -> ratatui::style::Color {
    if app.frame_count.is_multiple_of(2) {
        app.theme.warning
    } else {
        app.theme.success
    }
}

/// Level-up announcement. Its border and title alternate colors on each tick.
fn render_level_up(frame: &mut Frame, level_up: &LevelUp, app: &App) {
    let theme = &app.theme;
    let accent = blink(app);
    let accent_style = Style::new().fg(accent).add_modifier(Modifier::BOLD);

    let mut text = vec![
        Line::styled(t!("popup.level_up", icon = icons::STAR), accent_style).centered(),
        Line::from(""),
    ];
    if let Some(level) = level_up.app_level {
        text.push(
            Line::from(vec![
                Span::styled(level_up.app.clone(), theme.title),
                Span::raw(t!("popup.app_level", level)),
            ])
            .centered(),
        );
    }
    if let Some(level) = level_up.global_level {
        text.push(Line::from(t!("popup.profile_level", level)).centered());
    }
    text.push(
        Line::styled(
            format!("+{} XP", level_up.gained),
            Style::new().fg(theme.xp_fill),
        )
        .centered(),
    );
    text.push(Line::from(""));
    text.push(Line::styled(t!("popup.continue"), theme.muted()).centered());

    let area = centered(
        frame.area(),
        popup_width(frame.area()).min(44),
        text.len() as u16 + 2,
    );
    let block = theme
        .panel(&t!("popup.level_up_title"), true)
        .border_style(Style::new().fg(accent));
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
        .panel(&t!("popup.confirm_title"), true)
        .border_style(Style::new().fg(theme.error));
    let text = vec![
        Line::from(message.to_string()),
        Line::from(""),
        Line::from(vec![
            Span::styled(t!("keys.confirm_keys"), theme.title),
            Span::raw(t!("popup.yes")),
            Span::styled("Esc/n", theme.title),
            Span::raw(t!("popup.no")),
        ]),
    ];
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(text).block(block).wrap(Wrap { trim: true }),
        area,
    );
}

/// Rows of installed apps shown at once.
const PICKER_ROWS: u16 = 10;

/// Search box over the installed apps; the list scrolls to keep the selection visible.
fn render_picker(frame: &mut Frame, picker: &Picker, app: &App) {
    let theme = &app.theme;
    let area = centered(frame.area(), popup_width(frame.area()), PICKER_ROWS + 6);
    let block = theme.panel(&t!("form.add_title"), true);
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    let prompt = t!("picker.search");
    let room = (inner.width as usize).saturating_sub(prompt.chars().count() + 1);
    let (visible, cursor) = picker.query.view(room);
    let search = Line::from(vec![
        Span::styled(prompt.clone(), theme.title),
        Span::raw(visible),
    ]);
    frame.render_widget(Paragraph::new(search), Rect { height: 1, ..inner });
    frame.set_cursor_position(Position::new(
        inner.x + (prompt.chars().count() + cursor) as u16,
        inner.y,
    ));

    let matches = app.picker_matches(picker);
    let rows = PICKER_ROWS.min(inner.height.saturating_sub(3)) as usize;
    let mut lines = Vec::new();
    if matches.is_empty() {
        let text = if app.scan_running {
            t!("picker.scanning")
        } else if picker.query.is_empty() {
            t!("picker.none_installed")
        } else {
            t!("picker.no_match")
        };
        lines.push(Line::styled(format!("  {text}"), theme.muted()));
    }
    let first = picker.selected.saturating_sub(rows.saturating_sub(1));
    for (i, shortcut) in matches.iter().enumerate().skip(first).take(rows) {
        let selected = i == picker.selected;
        let process = shortcut.watch_exe.as_deref().unwrap_or("");
        let line = Line::from(vec![
            Span::raw(if selected { "> " } else { "  " }),
            Span::raw(shortcut.name.clone()),
            Span::styled(format!("  {process}"), theme.muted()),
        ]);
        lines.push(if selected {
            line.style(theme.highlight(true))
        } else {
            line
        });
    }
    let list = Rect::new(inner.x, inner.y + 2, inner.width, rows as u16);
    frame.render_widget(Paragraph::new(lines), list.intersection(inner));

    if inner.height > 1 {
        let footer = Line::styled(t!("picker.footer", count = matches.len()), theme.muted());
        let y = inner.bottom() - 1;
        frame.render_widget(
            Paragraph::new(footer),
            Rect::new(inner.x, y, inner.width, 1),
        );
    }
}

fn render_form(frame: &mut Frame, form: &Form, app: &App) {
    let theme = &app.theme;
    let rows = form.fields.len() as u16;
    // Fields, blank, help (3 rows), footer, borders.
    let area = centered(frame.area(), popup_width(frame.area()), rows + 7);
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
        let label = format!(
            "{marker}{:<w$}",
            format!("{}{required}", field.label),
            w = label_width - 2
        );
        let label_style = if focused { theme.title } else { theme.muted() };

        let room = (inner.width as usize).saturating_sub(label_width);
        let (visible, cursor) = field.input.view(room);
        let value = match form.placeholder(i) {
            Some(placeholder) if field.input.is_empty() => Span::styled(
                placeholder.chars().take(room).collect::<String>(),
                theme.muted(),
            ),
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
        None => Line::styled(t!("form.footer"), theme.muted()),
    };
    if let Some(help) = form.help() {
        let rect = Rect::new(
            inner.x + 2,
            inner.y + rows + 1,
            inner.width.saturating_sub(2),
            3,
        );
        frame.render_widget(
            Paragraph::new(help)
                .style(theme.muted())
                .wrap(Wrap { trim: true }),
            rect.intersection(inner),
        );
    }
    if inner.height > rows + 4 {
        let y = inner.y + rows + 4;
        frame.render_widget(
            Paragraph::new(footer),
            Rect::new(inner.x, y, inner.width, 1),
        );
    }
}

fn popup_width(area: Rect) -> u16 {
    MAX_WIDTH
        .min(area.width.saturating_sub(4))
        .max(area.width.min(20))
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
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
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
        for expected in [
            "Ajouter une app",
            "> Nom*",
            "Catégorie*",
            "Jeux",
            "Nom : champ requis",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in\n{text}");
        }
    }

    #[test]
    fn picker_lists_installed_apps_and_form_explains_process() {
        let mut app = App::with_defaults();
        app.shortcuts = vec![crate::launcher::scan::Shortcut {
            name: "Hades".into(),
            target: r"C:\Start\Hades.lnk".into(),
            watch_exe: Some("Hades.exe".into()),
        }];
        app.mode = Mode::Popup(Popup::Picker(Picker::new("Jeux")));
        let text = screen(&app, 100, 30);
        for expected in ["Chercher :", "> Hades", "Hades.exe", "1 app(s)"] {
            assert!(text.contains(expected), "missing {expected:?} in\n{text}");
        }

        let mut form = Form::add_app("Jeux");
        form.focused = 3;
        app.mode = Mode::Popup(Popup::Form(form));
        let text = screen(&app, 100, 30);
        assert!(text.contains("compter le temps de jeu"), "{text}");
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
        for expected in [
            "Niveau supérieur",
            "Steam passe au niveau 3",
            "Profil : niveau 2",
            "+120 XP",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in\n{text}");
        }
    }

    #[test]
    fn reward_renders_name_and_app() {
        let mut app = App::with_defaults();
        app.mode = Mode::Popup(Popup::RewardUnlocked(RewardUnlocked {
            name: "Marathon".into(),
            description: "Jouer 3 h d'affilée".into(),
            app: Some("Steam".into()),
        }));
        let text = screen(&app, 100, 30);
        for expected in [
            "Récompense débloquée",
            "Marathon",
            "Jouer 3 h d'affilée",
            "(Steam)",
        ] {
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
        let reward = Popup::RewardUnlocked(RewardUnlocked {
            name: "Marathon".into(),
            description: "Jouer 3 h d'affilée".into(),
            app: None,
        });
        let picker = Popup::Picker(Picker::new("Jeux"));
        for popup in [Popup::Form(Form::add_app("Jeux")), picker, level_up, reward] {
            app.mode = Mode::Popup(popup);
            for (w, h) in [(30, 8), (10, 3), (1, 1)] {
                screen(&app, w, h);
            }
        }
    }
}
