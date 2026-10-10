use ratatui::{
    Frame,
    layout::Rect,
    widgets::{List, ListItem},
};

use crate::app::{App, Focus};
use crate::ui::icons;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Focus::Categories;
    // "Recent" first: the apps played last, whatever their category.
    let recent = t!(
        "apps.recent",
        icon = icons::STAR,
        count = app.recent_apps().len()
    );
    let items: Vec<ListItem> = std::iter::once(ListItem::new(recent))
        .chain(
            app.categories
                .iter()
                .map(|c| ListItem::new(format!("{} ({})", c.name, app.app_count(c.id)))),
        )
        .collect();

    let list = List::new(items)
        .block(app.theme.panel(&t!("apps.categories"), focused))
        .highlight_style(app.theme.highlight(focused))
        .highlight_symbol("> ");

    // Rendering needs `&mut ListState`; work on a copy so `draw` stays pure.
    let mut state = app.cat_state;
    frame.render_stateful_widget(list, area, &mut state);
}
