//! Apps and categories: lookups, the dashboard selection, and the commands that change
//! them. Every write goes through `Database`, then `reload()` refreshes the cache.

use anyhow::{Context, bail};
use ratatui::widgets::TableState;

use super::{App, Focus, Mode, MsgKind, Outcome, Screen};
use crate::command::Command;
use crate::fuzzy;
use crate::launcher::launch;
use crate::popup::Popup;
use crate::storage::models::{AppEntry, Category, NewApp};

/// Apps listed in the "Recent" row.
const RECENT_APPS: usize = 8;

impl App {
    pub(super) fn find_app_by_id(&self, id: i64) -> Option<&AppEntry> {
        self.apps.iter().find(|a| a.id == id)
    }

    pub(super) fn app_name(&self, id: i64) -> Option<String> {
        self.find_app_by_id(id).map(|a| a.name.clone())
    }

    /// Case-insensitive lookup by name.
    pub fn find_app(&self, name: &str) -> Option<&AppEntry> {
        let name = name.to_lowercase();
        self.apps.iter().find(|a| a.name.to_lowercase() == name)
    }

    pub(super) fn app_named(&self, name: &str) -> anyhow::Result<&AppEntry> {
        self.find_app(name)
            .with_context(|| t!("error.unknown_app", name))
    }

    fn category_named(&self, name: &str) -> anyhow::Result<&Category> {
        let lower = name.to_lowercase();
        self.categories
            .iter()
            .find(|c| c.name.to_lowercase() == lower)
            .with_context(|| t!("error.unknown_category", name))
    }

    /// Id of the category with this name (ignoring case), created if missing.
    fn category_or_create(&mut self, name: &str) -> anyhow::Result<(i64, bool)> {
        match self.category_named(name) {
            Ok(category) => Ok((category.id, false)),
            Err(_) => Ok((self.db.add_category(name)?, true)),
        }
    }

    /// The category selected in the list, `None` on the "Recent" row above them.
    pub fn selected_category(&self) -> Option<&Category> {
        let row = self.cat_state.selected()?;
        self.categories.get(row.checked_sub(1)?)
    }

    /// The first row of the categories list: the apps played last.
    pub fn recents_selected(&self) -> bool {
        self.cat_state.selected() == Some(0)
    }

    /// Rows of the categories list: "Recent", then every category.
    pub fn category_rows(&self) -> usize {
        self.categories.len() + 1
    }

    /// The `RECENT_APPS` apps whose last session ended last, most recent first.
    pub fn recent_apps(&self) -> Vec<&AppEntry> {
        let mut played: Vec<&AppEntry> = self
            .apps
            .iter()
            .filter(|a| a.last_played.is_some())
            .collect();
        played.sort_by_key(|a| std::cmp::Reverse(a.last_played));
        played.truncate(RECENT_APPS);
        played
    }

    /// Keeps the categories row in range. At first, "Recent" if anything was played,
    /// else the first category.
    pub(super) fn clamp_category_row(&mut self) {
        let first = usize::from(self.recent_apps().is_empty());
        let row = self.cat_state.selected().unwrap_or(first);
        self.cat_state
            .select(Some(row.min(self.category_rows() - 1)));
    }

    /// Apps of the selected category, in display order. While searching: the matches
    /// among all apps, best first.
    pub fn visible_apps(&self) -> Vec<&AppEntry> {
        if self.mode == Mode::Search {
            let names = self.apps.iter().map(|a| a.name.as_str());
            return fuzzy::rank(self.search.text(), names)
                .into_iter()
                .map(|i| &self.apps[i])
                .collect();
        }
        if self.recents_selected() {
            return self.recent_apps();
        }
        match self.selected_category() {
            Some(cat) => self
                .apps
                .iter()
                .filter(|a| a.category_id == cat.id)
                .collect(),
            None => Vec::new(),
        }
    }

    pub fn selected_app(&self) -> Option<&AppEntry> {
        let i = self.app_state.selected()?;
        self.visible_apps().get(i).copied()
    }

    pub fn app_count(&self, category_id: i64) -> usize {
        self.apps
            .iter()
            .filter(|a| a.category_id == category_id)
            .count()
    }

    /// Selects an app and its category, and focuses the apps panel.
    pub(super) fn select_app(&mut self, id: i64) {
        let Some(category_id) = self.find_app_by_id(id).map(|a| a.category_id) else {
            return;
        };
        let cat_index = self
            .categories
            .iter()
            .position(|c| c.id == category_id)
            .map(|i| i + 1);
        self.cat_state.select(cat_index);
        let app_index = self.visible_apps().iter().position(|a| a.id == id);
        self.app_state.select(app_index);
        self.focus = Focus::Apps;
    }

    /// `:select`, or Enter in the search: shows the app on the dashboard.
    pub(super) fn select_by_name(&mut self, app: &str) -> anyhow::Result<()> {
        let id = self.app_named(app)?.id;
        self.screen = Screen::Dashboard;
        self.select_app(id);
        Ok(())
    }

    pub(super) fn reset_app_selection(&mut self) {
        let selected = (!self.visible_apps().is_empty()).then_some(0);
        self.app_state = TableState::default().with_selected(selected);
    }

    pub(super) fn launch(&self, app: &str) -> Outcome {
        let entry = self.app_named(app)?;
        launch::launch(&entry.launch_target, entry.launch_args.as_deref())?;
        Ok(Some((
            t!("action.launched", name = entry.name),
            MsgKind::Success,
        )))
    }

    pub(super) fn add_app(
        &mut self,
        name: &str,
        target: String,
        category: Option<String>,
        watch_exe: Option<String>,
        args: Option<String>,
    ) -> Outcome {
        if self.find_app(name).is_some() {
            bail!(t!("error.app_exists", name));
        }
        launch::check_target(&target)?;
        let (category_id, created) = if let Some(category) = category {
            self.category_or_create(&category)?
        } else {
            let selected = self
                .selected_category()
                .with_context(|| t!("error.no_category"))?;
            (selected.id, false)
        };
        let id = self.db.add_app(&new_app(
            name.to_owned(),
            target,
            category_id,
            watch_exe,
            args,
        ))?;
        self.reload()?;
        self.select_app(id);
        let text = t!("action.added", name, note = created_note(created));
        Ok(Some((text, MsgKind::Success)))
    }

    /// Same checks as `add_app`. The app keeps its id, so its history stays.
    pub(super) fn edit_app(
        &mut self,
        app: &str,
        name: &str,
        target: String,
        category: &str,
        watch_exe: Option<String>,
        args: Option<String>,
    ) -> Outcome {
        let id = self.app_named(app)?.id;
        // Another app with that name; changing only the case of its own name is fine.
        if self.find_app(name).is_some_and(|other| other.id != id) {
            bail!(t!("error.app_exists", name));
        }
        launch::check_target(&target)?;
        let (category_id, created) = self.category_or_create(category)?;
        self.db.update_app(
            id,
            &new_app(name.to_owned(), target, category_id, watch_exe, args),
        )?;
        self.reload()?;
        self.select_app(id);
        let text = t!("action.edited", name, note = created_note(created));
        Ok(Some((text, MsgKind::Success)))
    }

    pub(super) fn move_app(&mut self, app: &str, category: &str) -> Outcome {
        let (id, name) = {
            let entry = self.app_named(app)?;
            (entry.id, entry.name.clone())
        };
        let (category_id, created) = self.category_or_create(category)?;
        self.db.move_app(id, category_id)?;
        self.reload()?;
        self.select_app(id);
        let category = self
            .selected_category()
            .map_or(category, |c| c.name.as_str());
        let text = t!("action.moved", name, category, note = created_note(created));
        Ok(Some((text, MsgKind::Success)))
    }

    pub(super) fn remove_app(&mut self, app: &str, confirmed: bool) -> Outcome {
        let (id, name) = {
            let entry = self.app_named(app)?;
            (entry.id, entry.name.clone())
        };
        if !confirmed {
            let message = t!("action.confirm_remove_app", name);
            let app = name;
            return self.confirm(
                message,
                Command::RemoveApp {
                    app,
                    confirmed: true,
                },
            );
        }
        self.db.delete_app(id)?;
        self.reload()?;
        Ok(Some((t!("action.removed", name), MsgKind::Success)))
    }

    /// Only an empty category can be removed.
    pub(super) fn remove_category(&mut self, category: &str, confirmed: bool) -> Outcome {
        let (id, name) = {
            let found = self.category_named(category)?;
            (found.id, found.name.clone())
        };
        let count = self.app_count(id);
        if count > 0 {
            bail!(t!("error.category_not_empty", name, count));
        }
        if !confirmed {
            let message = t!("action.confirm_remove_category", name);
            let category = name;
            return self.confirm(
                message,
                Command::RemoveCategory {
                    category,
                    confirmed: true,
                },
            );
        }
        self.db.delete_category(id)?;
        self.reload()?;
        let text = t!("action.category_removed", name);
        Ok(Some((text, MsgKind::Success)))
    }

    /// Asks before running `command`, its `confirmed: true` version.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "callers return it as their own Outcome"
    )]
    pub(super) fn confirm(&mut self, message: String, command: Command) -> Outcome {
        self.mode = Mode::Popup(Popup::Confirm { message, command });
        Ok(None)
    }
}

/// Without a `watch_exe`, the one `target` launches, if any.
fn new_app(
    name: String,
    target: String,
    category_id: i64,
    watch_exe: Option<String>,
    launch_args: Option<String>,
) -> NewApp {
    NewApp {
        launch_args,
        watch_exe: watch_exe.or_else(|| launch::watch_exe_for(&target)),
        name,
        launch_target: target,
        category_id,
    }
}

fn created_note(created: bool) -> String {
    if created {
        t!("new_category")
    } else {
        String::new()
    }
}
