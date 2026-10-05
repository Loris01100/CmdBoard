use ratatui::{
    Frame,
    layout::Rect,
    widgets::{List, ListItem},
};

use crate::app::{App, Focus};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Focus::Categories;
    let items: Vec<ListItem> = app
        .categories
        .iter()
        .map(|c| ListItem::new(format!("{} ({})", c.name, app.app_count(c.id))))
        .collect();

    let list = List::new(items)
        .block(app.theme.panel(&t!("apps.categories"), focused))
        .highlight_style(app.theme.highlight(focused))
        .highlight_symbol("> ");

    // Rendering needs `&mut ListState`; work on a copy so `draw` stays pure.
    let mut state = app.cat_state.clone();
    frame.render_stateful_widget(list, area, &mut state);
}
