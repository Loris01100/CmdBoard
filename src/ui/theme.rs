use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, BorderType},
};

/// Every color used by the UI. Widgets must never hardcode colors.
#[derive(Debug, Clone)]
pub struct Theme {
    pub border: Style,
    pub border_focused: Style,
    pub title: Style,
    pub selected: Style,
    pub selected_unfocused: Style,
    pub xp_fill: Color,
    pub info: Color,
    pub success: Color,
    pub error: Color,
    pub muted: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            border: Style::new().fg(Color::DarkGray),
            border_focused: Style::new().fg(Color::Cyan),
            title: Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            selected: Style::new()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            selected_unfocused: Style::new().add_modifier(Modifier::BOLD),
            xp_fill: Color::Green,
            info: Color::Cyan,
            success: Color::Green,
            error: Color::Red,
            muted: Color::DarkGray,
        }
    }
}

impl Theme {
    /// Bordered panel with its title embedded in the frame.
    pub fn panel(&self, title: &str, focused: bool) -> Block<'static> {
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(if focused { self.border_focused } else { self.border })
            .title(Span::styled(format!(" {title} "), self.title))
    }

    pub fn highlight(&self, focused: bool) -> Style {
        if focused { self.selected } else { self.selected_unfocused }
    }

    pub fn muted(&self) -> Style {
        Style::new().fg(self.muted)
    }
}
