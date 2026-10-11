//! Suggesting a running game that is not added yet (plan section 8): the library games
//! (Steam, Epic, GOG, Ubisoft Connect) are scanned once at startup, the tracker watches
//! the exes of those not added, and the dashboard header names the one running, `n`
//! opening the add form pre-filled. `x` ignores it for good (`ignored_games` in
//! `config.toml`), `:ignore off <game>` suggests it again.

use anyhow::bail;

use super::{App, Message, MsgKind, Outcome};
use crate::event::AppEvent;
use crate::launcher::scan::{self, Shortcut};
use crate::popup::{Form, Popup};
use crate::tracker::{self, WatchList};

/// Library games and which of the ones not added are running.
#[derive(Debug, Default)]
pub struct Suggestions {
    /// Library games with an exe to watch, scanned at startup.
    pub(super) library: Vec<Shortcut>,
    /// `exe_key`s of the games not added that run, as last reported by the tracker.
    pub(super) running: Vec<String>,
    /// Names of the games never to suggest, as saved in `config.toml`.
    pub(super) ignored: Vec<String>,
}

impl App {
    /// At startup: the games ignored in `config.toml`.
    pub fn init_ignored_games(&mut self, games: Vec<String>) {
        self.suggestions.ignored = games;
    }

    /// At startup, in the UI only: scans the game libraries in a short-lived thread.
    pub fn start_library_scan(&mut self) {
        self.spawn(|| AppEvent::LibraryScanned(scan::library_games()));
    }

    pub(super) fn on_library_scanned(&mut self, games: Vec<Shortcut>) {
        self.suggestions.library = games
            .into_iter()
            .filter(|g| g.watch_exe.as_deref().is_some_and(|e| !e.trim().is_empty()))
            .collect();
        self.send_watch_list();
    }

    pub(super) fn on_unadded_running(&mut self, exes: Vec<String>) {
        self.suggestions.running = exes;
    }

    /// What the tracker looks for: the apps, and the exes of library games not added.
    pub(super) fn full_watch_list(&self) -> WatchList {
        WatchList {
            apps: self.watch_list(),
            unadded: self
                .unadded_games()
                .filter_map(|g| g.watch_exe.clone())
                .collect(),
        }
    }

    /// The running library game to suggest adding, if any: the first by name.
    pub fn suggestion(&self) -> Option<&Shortcut> {
        self.unadded_games().find(|g| {
            let key = g.watch_exe.as_deref().map(tracker::exe_key);
            key.is_some_and(|key| self.suggestions.running.contains(&key))
        })
    }

    /// `n` on the dashboard: the add form, filled in like the picker would.
    pub(super) fn add_running(&mut self) -> Option<Message> {
        let Some(game) = self.suggestion().cloned() else {
            return Some((t!("suggestion.none"), MsgKind::Info));
        };
        let category = self.selected_category().map_or("", |c| c.name.as_str());
        let form = Form::add_app_from(
            category,
            &game.name,
            &game.target,
            game.watch_exe.as_deref(),
        );
        self.mode = super::Mode::Popup(Popup::Form(form));
        None
    }

    /// `x` on the dashboard: never suggests the running game again.
    pub(super) fn ignore_running(&mut self) -> Outcome {
        let Some(name) = self.suggestion().map(|g| g.name.clone()) else {
            return Ok(Some((t!("suggestion.none"), MsgKind::Info)));
        };
        let mut ignored = self.suggestions.ignored.clone();
        ignored.push(name.clone());
        self.save_ignored(ignored)?;
        Ok(Some((t!("ignore.added", name), MsgKind::Success)))
    }

    /// `:ignore`: lists the ignored games, or (`show_again`) suggests one again.
    pub(super) fn ignore_command(&mut self, show_again: Option<&str>) -> Outcome {
        let Some(name) = show_again else {
            let text = if self.suggestions.ignored.is_empty() {
                t!("ignore.empty")
            } else {
                t!("ignore.list", list = self.suggestions.ignored.join(", "))
            };
            return Ok(Some((text, MsgKind::Info)));
        };
        let name = name.trim();
        let mut ignored = self.suggestions.ignored.clone();
        let before = ignored.len();
        ignored.retain(|g| !g.eq_ignore_ascii_case(name));
        if ignored.len() == before {
            bail!(t!("ignore.unknown", name));
        }
        self.save_ignored(ignored)?;
        Ok(Some((t!("ignore.removed", name), MsgKind::Success)))
    }

    /// Saves the list first, so a broken `config.toml` changes nothing, then tells the
    /// tracker what to look for now.
    fn save_ignored(&mut self, ignored: Vec<String>) -> anyhow::Result<()> {
        self.save_setting("ignored_games", ignored.clone(), &t!("ignore.not_saved"))?;
        self.suggestions.ignored = ignored;
        self.send_watch_list();
        Ok(())
    }

    /// Library games no app launches or watches, no app is named after, not ignored.
    fn unadded_games(&self) -> impl Iterator<Item = &Shortcut> {
        self.suggestions.library.iter().filter(|game| {
            let exe = game.watch_exe.as_deref().map(tracker::exe_key);
            let ignored = |name: &String| name.eq_ignore_ascii_case(&game.name);
            !self.suggestions.ignored.iter().any(ignored)
                && !self.apps.iter().any(|app| {
                    app.name.eq_ignore_ascii_case(&game.name)
                        || app.launch_target.eq_ignore_ascii_case(&game.target)
                        || app.watch_exe.as_deref().map(tracker::exe_key) == exe
                })
        })
    }
}
