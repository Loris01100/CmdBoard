//! Application layer: the state `App` holds, and how keys, commands and thread events
//! change it. `ui/` only reads it; XP and reward rules live in `core/`, SQLite in
//! `storage/`, scans and launches in `launcher/`.
//!
//! - `keys`: key bindings per mode, mapped to `Command`s
//! - `commands`: the single execution path for every `Command`, aliases and completion
//! - `library`: apps and categories, their selection and changes
//! - `forms`: the "add app" picker and the add/edit/move forms
//! - `sessions`: tracked sessions, XP animations, level-up and reward popups
//! - `background`: sessions handed over to and from the background tracker
//! - `settings`: theme, sort, language, aliases, export/import, updates, history
//! - `storage`, `optimize`: the Storage and Optimization screens
//! - `folders`: the folder browser of the Storage screen

mod background;
mod commands;
mod folders;
mod forms;
mod keys;
mod library;
mod optimize;
mod sessions;
mod settings;
mod sort;
mod storage;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};

use ratatui::{
    Terminal,
    backend::Backend,
    layout::Rect,
    widgets::{ListState, TableState},
};

use crate::command::{alias::Aliases, line::CommandLine};
use crate::config;
use crate::event::AppEvent;
use crate::launcher::scan::Shortcut;
use crate::popup::Popup;
use crate::storage::{
    Database,
    models::{Activity, AppEntry, Category, Profile, RewardView, Stats},
    unix_now,
};
use crate::text_input::TextInput;
use crate::tracker::Watched;
use crate::ui::{
    self,
    theme::{self, Theme},
};
use crate::update;

use optimize::OptimizeScreen;
use sessions::{ActiveSession, XpAnim};
pub use sort::AppSort;
use storage::StorageScreen;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Dashboard,
    Stats,
    Rewards,
    Storage,
    Optimize,
    Help,
}

impl Screen {
    pub fn title(self) -> String {
        match self {
            Screen::Dashboard => t!("screen.dashboard"),
            Screen::Stats => t!("screen.stats"),
            Screen::Rewards => t!("screen.rewards"),
            Screen::Storage => t!("screen.storage"),
            Screen::Optimize => t!("screen.optimize"),
            Screen::Help => t!("screen.help"),
        }
    }
}

/// How keys are interpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// Typing after `:`.
    Command,
    /// Typing after `/`: the apps panel lists the fuzzy matches among all apps.
    Search,
    /// A confirmation or form captures every key.
    Popup(Popup),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Categories,
    Apps,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgKind {
    Info,
    Success,
    Error,
}

type Message = (String, MsgKind);

/// Most queued events handled before drawing again, so a flood cannot freeze the screen.
const MAX_BATCH: usize = 256;

/// What a command reports: a message to show, or nothing.
type Outcome = anyhow::Result<Option<Message>>;

#[expect(
    clippy::struct_excessive_bools,
    clippy::struct_field_names,
    reason = "App holds the state of every screen, named after it"
)]
pub struct App {
    // navigation
    pub screen: Screen,
    pub mode: Mode,
    pub focus: Focus,
    pub cat_state: ListState,
    pub app_state: TableState,
    pub sort: AppSort,

    // cached data, reloaded from the database after each write
    pub categories: Vec<Category>,
    pub apps: Vec<AppEntry>,
    pub profile: Profile,
    /// Latest sessions and rewards, for the dashboard ticker.
    pub activity: Vec<Activity>,
    /// Every reward, for the Rewards screen.
    pub rewards: Vec<RewardView>,
    pub reward_state: TableState,
    /// Stats screen data, for `stats_app` or every app.
    pub stats: Stats,
    pub stats_app: Option<i64>,
    pub stats_state: TableState,
    /// The Stats pie shows the time per app instead of per category.
    pub stats_by_app: bool,

    // `/` search
    pub search: TextInput,
    /// Focus and app selection to restore when the search is cancelled.
    search_restore: Option<(Focus, Option<usize>)>,

    // sessions and animations, driven by `Tick`
    pub active_sessions: HashMap<i64, ActiveSession>,
    /// Incremented on each tick.
    pub frame_count: u64,
    /// Something on screen changed since the last draw.
    redraw: bool,
    /// Session clock (seconds played, every session) and minute of the last tick: a tick
    /// redraws when one of them changes.
    shown_clock: (u64, i64),
    /// XP bars still filling up, by app id.
    xp_anims: HashMap<i64, XpAnim>,
    profile_anim: Option<XpAnim>,
    /// Popups waiting for the user to be back in normal mode (e.g. a level-up while typing).
    pending_popups: VecDeque<Popup>,

    // command line
    pub command_line: CommandLine,
    /// From `commands.toml`.
    pub aliases: Aliases,
    /// Feedback from the last command, shown on the command line row.
    pub message: Option<Message>,

    pub theme: Theme,
    /// Name `:theme` loads it by (file stem), shown by `:theme`.
    pub theme_name: String,
    /// User themes and `config.toml`. `None` in tests: built-in themes, nothing saved.
    themes_dir: Option<PathBuf>,
    config_path: Option<PathBuf>,
    /// Only `App` writes: `ui/` cannot reach the database.
    db: Database,
    /// Receives the watch list after each reload. `None` in tests.
    tracker: Option<Sender<Vec<Watched>>>,
    /// Lets short-lived threads report back. `None` in tests: they never start.
    events: Option<Sender<AppEvent>>,
    update_running: bool,
    /// Newer version found by a check, shown in the status bar.
    pub update_available: Option<String>,
    /// Installed apps for the "add app" picker, rescanned each time it opens.
    pub shortcuts: Vec<Shortcut>,
    pub scan_running: bool,

    /// Storage screen, rescanned each time it opens.
    pub storage: StorageScreen,
    /// Optimization screen, nothing saved.
    pub optimize: OptimizeScreen,
    pub should_quit: bool,
}

impl App {
    pub fn new(db: Database) -> anyhow::Result<Self> {
        let mut app = Self {
            screen: Screen::Dashboard,
            mode: Mode::Normal,
            focus: Focus::Categories,
            cat_state: ListState::default(),
            app_state: TableState::default(),
            sort: AppSort::default(),
            categories: Vec::new(),
            apps: Vec::new(),
            profile: Profile::default(),
            activity: Vec::new(),
            rewards: Vec::new(),
            reward_state: TableState::default(),
            stats: Stats::default(),
            stats_app: None,
            stats_state: TableState::default(),
            stats_by_app: false,
            search: TextInput::default(),
            search_restore: None,
            active_sessions: HashMap::new(),
            frame_count: 0,
            redraw: true,
            shown_clock: (0, 0),
            xp_anims: HashMap::new(),
            profile_anim: None,
            pending_popups: VecDeque::new(),
            command_line: CommandLine::default(),
            aliases: Aliases::default(),
            message: None,
            theme: Theme::default(),
            theme_name: theme::FALLBACK.into(),
            themes_dir: None,
            config_path: None,
            db,
            tracker: None,
            events: None,
            update_running: false,
            update_available: None,
            shortcuts: Vec::new(),
            scan_running: false,
            storage: StorageScreen::default(),
            optimize: OptimizeScreen::default(),
            should_quit: false,
        };
        app.reload()?;
        Ok(app)
    }

    /// Refreshes the cached data and keeps every selection in range.
    pub fn reload(&mut self) -> anyhow::Result<()> {
        self.categories = self.db.categories()?;
        self.apps = self.db.apps()?;
        self.sort.apply(&mut self.apps);
        self.profile = self.db.profile()?;
        self.activity = self.db.activity(10)?;
        if self
            .stats_app
            .is_some_and(|id| self.find_app_by_id(id).is_none())
        {
            self.stats_app = None; // the app was removed
        }
        self.stats = self.db.stats(self.stats_app)?;
        self.stats_state.select(clamp(
            self.stats_state.selected(),
            self.stats.sessions.len(),
        ));
        self.rewards = self.db.reward_views()?;
        self.reward_state
            .select(clamp(self.reward_state.selected(), self.rewards.len()));
        self.cat_state
            .select(clamp(self.cat_state.selected(), self.categories.len()));
        let visible = self.visible_apps().len();
        self.app_state
            .select(clamp(self.app_state.selected(), visible));
        self.send_watch_list();
        Ok(())
    }

    /// At startup: the configured theme, else one picked from the terminal's colors.
    /// If the configured theme cannot load, falls back and returns the error to show.
    pub fn init_theme(&mut self, data_dir: &Path, configured: Option<String>) -> Option<String> {
        self.themes_dir = Some(data_dir.join("themes"));
        self.config_path = Some(data_dir.join("config.toml"));
        let wanted = configured.unwrap_or_else(|| theme::default_name().into());
        match theme::load(&wanted, self.themes_dir.as_deref()) {
            Ok(loaded) => {
                self.theme = loaded;
                self.theme_name = wanted.trim().to_lowercase();
                None
            }
            Err(e) => {
                let fallback = theme::default_name();
                if let Ok(loaded) = theme::load(fallback, None) {
                    self.theme = loaded;
                    self.theme_name = fallback.into();
                }
                Some(e)
            }
        }
    }

    /// At startup: the order saved in `config.toml`. Returns an error to show if unknown.
    pub fn init_sort(&mut self, configured: Option<&str>) -> Option<String> {
        let name = configured?;
        let Some(sort) = AppSort::parse(name) else {
            return Some(t!("sort.unknown_config", name));
        };
        self.sort = sort;
        self.sort.apply(&mut self.apps);
        None
    }

    /// Connects the session tracker and sends it the current watch list.
    pub fn attach_tracker(&mut self, tracker: Sender<Vec<Watched>>) {
        self.tracker = Some(tracker);
        self.send_watch_list();
    }

    /// Starts the daily passive update check, when due and not turned off in `config.toml`.
    pub fn attach_events(&mut self, events: Sender<AppEvent>, config: &config::Config) {
        let now = unix_now();
        if config.update_check && update::check_due(config.last_update_check, now) {
            if let Some(path) = &self.config_path {
                // Saved before checking, so being offline doesn't retry at every launch.
                let _ = config::save_value(path, "last_update_check", now);
            }
            self.update_running = true;
            update::spawn(events.clone(), update::Action::Check);
        }
        self.events = Some(events);
        // So that `:uninstall` completes from any screen.
        self.start_storage_scan();
    }

    /// Apps with a process to watch.
    fn watch_list(&self) -> Vec<Watched> {
        self.apps
            .iter()
            .filter_map(|a| {
                let exe = a.watch_exe.as_deref()?.trim();
                (!exe.is_empty()).then(|| Watched {
                    app_id: a.id,
                    exe: exe.into(),
                })
            })
            .collect()
    }

    fn send_watch_list(&self) {
        let Some(tracker) = &self.tracker else { return };
        // A closed channel means the tracker died: sessions just stop being tracked.
        let _ = tracker.send(self.watch_list());
    }

    /// Runs `work` in a short-lived thread that reports through the event channel.
    /// Returns false, running nothing, without a channel (in tests).
    fn spawn(&self, work: impl FnOnce() -> AppEvent + Send + 'static) -> bool {
        let Some(events) = self.events.clone() else {
            return false;
        };
        std::thread::spawn(move || {
            let _ = events.send(work());
        });
        true
    }

    /// Main loop: waits for an event, handles it with every one already queued, then
    /// draws once if the screen changed. Idle, a tick draws nothing; a burst of events (a
    /// folder being measured) costs one draw. Key events are already filtered on `Press`
    /// by the event thread. Running sessions are left to `quit_sessions`.
    pub fn run<B: Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        events: &Receiver<AppEvent>,
    ) -> anyhow::Result<()>
    where
        B::Error: Send + Sync + 'static,
    {
        while !self.should_quit {
            if std::mem::take(&mut self.redraw) {
                terminal.draw(|f| ui::draw(f, self))?;
            }
            let first = events.recv()?;
            for event in std::iter::once(first).chain(events.try_iter().take(MAX_BATCH)) {
                if matches!(event, AppEvent::Tick) && !self.redraw {
                    let size = terminal.size()?;
                    self.redraw = ui::animates(self, Rect::new(0, 0, size.width, size.height));
                }
                self.handle(event);
                if self.should_quit {
                    break;
                }
            }
        }
        Ok(())
    }

    /// Routes one event from the event, tracker or a short-lived thread.
    pub fn handle(&mut self, event: AppEvent) {
        // A tick redraws only when something moves (`on_tick`); any other event may
        // change the screen.
        self.redraw |= !matches!(event, AppEvent::Tick);
        match event {
            AppEvent::Key(key) => self.on_key(key),
            AppEvent::Resize => {}
            AppEvent::Tick => self.on_tick(),
            AppEvent::SessionStarted { app_id } => self.on_session_start(app_id),
            AppEvent::SessionProgress {
                app_id,
                played,
                idle,
            } => self.on_session_progress(app_id, played, idle),
            AppEvent::SessionEnded { app_id, secs } => self.on_session_end(app_id, secs),
            AppEvent::UpdateFinished { action, result } => self.on_update_finished(action, result),
            AppEvent::StorageScanned { disks, programs } => {
                self.storage.on_scanned(disks, programs);
            }
            AppEvent::FolderListed { dir, entries } => self.on_folder_listed(&dir, entries),
            AppEvent::FolderProgress { path, percent } => {
                self.storage.on_folder_progress(path, percent);
            }
            AppEvent::FolderSized { path, size } => self.storage.on_folder_sized(path, size),
            AppEvent::Trashed { path, result } => self.on_trashed(&path, result),
            AppEvent::BenchFinished {
                bench,
                heavy,
                result,
            } => self.optimize.on_finished(bench, heavy, result),
            AppEvent::ShortcutsScanned(found) => self.on_shortcuts_scanned(found),
        }
    }
}

/// Next index in a list of `len` items, wrapping at both ends.
fn step(current: Option<usize>, len: usize, forward: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(match current {
        None => 0,
        Some(i) if forward => (i + 1) % len,
        Some(i) => (i + len - 1) % len,
    })
}

/// Keeps a selection inside a list of `len` items, selecting the first one by default.
fn clamp(selected: Option<usize>, len: usize) -> Option<usize> {
    (len > 0).then(|| selected.unwrap_or(0).min(len - 1))
}

#[cfg(test)]
impl App {
    /// App on an in-memory database holding the starter content.
    pub fn with_defaults() -> Self {
        let db = Database::open_in_memory().unwrap();
        db.seed_defaults(|_| true).unwrap();
        App::new(db).unwrap()
    }
}
