//! Suggesting a running game that is not added yet (plan section 8): the library games
//! (Steam, Epic, GOG, Ubisoft Connect) are scanned once at startup, the tracker watches
//! the exes of those not added, and the dashboard header names the one running, `n`
//! opening the add form pre-filled.

use super::{App, Message, MsgKind};
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
}

impl App {
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

    /// Library games no app launches or watches, and no app is named after.
    fn unadded_games(&self) -> impl Iterator<Item = &Shortcut> {
        self.suggestions.library.iter().filter(|game| {
            let exe = game.watch_exe.as_deref().map(tracker::exe_key);
            !self.apps.iter().any(|app| {
                app.name.eq_ignore_ascii_case(&game.name)
                    || app.launch_target.eq_ignore_ascii_case(&game.target)
                    || app.watch_exe.as_deref().map(tracker::exe_key) == exe
            })
        })
    }
}
