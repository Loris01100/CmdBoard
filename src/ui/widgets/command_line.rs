use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::Stylize,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::{App, Mode, MsgKind};

const PLACEHOLDER: &str = "launch <app>, stats, help… (Tab)";
const SEARCH_PLACEHOLDER: &str = "nom d'une app, même approximatif";

/// Rows the command area needs: a bordered box while typing, one line otherwise.
pub fn height(app: &App) -> u16 {
    match &app.mode {
        Mode::Command | Mode::Search => 3,
        Mode::Normal | Mode::Popup(_) => 1,
    }
}

/// The `:` input box while typing a command, otherwise the last message.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    match &app.mode {
        Mode::Command => {
            let input = &app.command_line.input;
            let block = app.theme.panel("Commande", true);
            let block = match completion_hint(app) {
                Some(hint) => block.title_bottom(hint),
                None => block,
            };
            render_input(frame, area, app, block, (":", input, PLACEHOLDER));
        }
        Mode::Search => {
            let block = app.theme.panel("Recherche", true);
            render_input(
                frame,
                area,
                app,
                block,
                ("/", &app.search, SEARCH_PLACEHOLDER),
            );
        }
        Mode::Normal | Mode::Popup(_) => render_message(frame, area, app),
    }
}

/// After Tab, the candidates on the box's bottom border, the shown one highlighted.
fn completion_hint(app: &App) -> Option<Line<'static>> {
    let (completion, index) = app.command_line.completion.as_ref()?;
    if completion.candidates.len() < 2 {
        return None;
    }
    let theme = &app.theme;
    let mut spans = vec![Span::raw(" ")];
    for (i, candidate) in completion.candidates.iter().enumerate().take(8) {
        let style = if i == *index {
            theme.title
        } else {
            theme.muted()
        };
        spans.push(Span::styled(candidate.trim_end().to_string(), style));
        spans.push(Span::raw(" "));
    }
    if completion.candidates.len() > 8 {
        spans.push(Span::styled("… ", theme.muted()));
    }
    Some(Line::from(spans))
}

fn render_input(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    block: ratatui::widgets::Block<'static>,
    (prompt, input, placeholder): (&str, &crate::text_input::TextInput, &str),
) {
    let theme = &app.theme;
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width < 2 || inner.height == 0 {
        return;
    }

    // Room left after the prompt.
    let (visible, cursor) = input.view((inner.width - 1) as usize);
    let text = if input.is_empty() {
        Span::styled(placeholder.to_string(), theme.muted())
    } else {
        Span::raw(visible)
    };
    let prompt = Line::from(vec![Span::styled(prompt.to_string(), theme.title), text]);
    frame.render_widget(Paragraph::new(prompt), inner);

    // Placing the terminal cursor is not a state change.
    frame.set_cursor_position(Position::new(inner.x + 1 + cursor as u16, inner.y));
}

fn render_message(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let Some((text, kind)) = &app.message else {
        return;
    };
    let color = match kind {
        MsgKind::Info => theme.info,
        MsgKind::Success => theme.success,
        MsgKind::Error => theme.error,
    };
    frame.render_widget(Paragraph::new(format!(" {text}")).fg(color), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        Terminal,
        backend::{Backend, TestBackend},
    };

    fn render_box(app: &App, width: u16) -> (Vec<String>, Position) {
        let mut terminal = Terminal::new(TestBackend::new(width, 3)).unwrap();
        terminal.draw(|f| render(f, f.area(), app)).unwrap();
        let backend = terminal.backend_mut();
        let buffer = backend.buffer().clone();
        let rows = (0..3)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        (rows, backend.get_cursor_position().unwrap())
    }

    #[test]
    fn input_box_shows_text_and_cursor() {
        let mut app = App::with_defaults();
        app.mode = Mode::Command;
        assert!(render_box(&app, 40).0[1].contains(PLACEHOLDER));

        "launch Steam"
            .chars()
            .for_each(|c| app.command_line.input.insert(c));
        let (rows, cursor) = render_box(&app, 40);
        assert!(rows[0].contains("Commande"));
        assert!(rows[1].contains(":launch Steam"));
        assert_eq!(cursor, Position::new(1 + 1 + 12, 1)); // border, ':', 12 chars
    }

    #[test]
    fn search_box_and_completion_hint() {
        let mut app = App::with_defaults();
        app.mode = Mode::Search;
        "ste".chars().for_each(|c| app.search.insert(c));
        let (rows, _) = render_box(&app, 40);
        assert!(rows[0].contains("Recherche"));
        assert!(rows[1].contains("/ste"));

        app.mode = Mode::Command;
        app.command_line.input.set("move Steam ");
        app.command_line.completion = Some((
            crate::command::complete::Completion {
                base: "move Steam ".into(),
                candidates: vec!["Dev".into(), "Jeux".into()],
            },
            1,
        ));
        let (rows, _) = render_box(&app, 40);
        assert!(rows[2].contains("Dev Jeux"), "{rows:?}");
    }

    #[test]
    fn long_input_scrolls_to_keep_cursor_visible() {
        let mut app = App::with_defaults();
        app.mode = Mode::Command;
        let long = format!("add Jeu {}end", "x".repeat(40));
        long.chars().for_each(|c| app.command_line.input.insert(c));
        let (rows, cursor) = render_box(&app, 20);
        assert!(rows[1].contains("end"));
        assert!(cursor.x < 19); // inside the right border
    }
}
