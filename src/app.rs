use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{self, AtomicBool},
    mpsc::{Receiver, Sender},
};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    DefaultTerminal,
    widgets::{ListState, TableState},
};

use crate::command::{
    Command,
    alias::Aliases,
    complete::{self, Sources},
    find_help,
    line::CommandLine,
    parser,
};
use crate::config;
use crate::core::{rewards, xp};
use crate::event::AppEvent;
use crate::fuzzy;
use crate::i18n;
use crate::launcher::{
    folders::{self, Entry},
    launch,
    programs::{self, Disk, Program},
    scan::{self, Shortcut},
};
use crate::optimize::{self, Bench, Gaming, Score};
use crate::popup::{Form, FormKind, LevelUp, Picker, Popup, RewardUnlocked};
use crate::storage::{
    Database,
    models::{Activity, AppEntry, Category, NewApp, Profile, RewardView, Stats},
    unix_now,
};
use crate::text_input::TextInput;
use crate::tracker::Watched;
use crate::ui::{
    self,
    theme::{self, Theme},
};
use crate::update;

/// How often running sessions save their time played, in case of a crash.
const CHECKPOINT_EVERY: Duration = Duration::from_secs(60);

/// How many ticks an XP bar takes to fill up to its new value (2 s at 250 ms).
pub const XP_ANIM_FRAMES: u64 = 8;

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

/// Order of the apps panel, chosen with `:sort` or `s`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppSort {
    #[default]
    Name,
    Xp,
    Recent,
    Time,
}

impl AppSort {
    pub const ALL: [AppSort; 4] = [AppSort::Name, AppSort::Xp, AppSort::Recent, AppSort::Time];

    pub fn name(self) -> &'static str {
        match self {
            AppSort::Name => "name",
            AppSort::Xp => "xp",
            AppSort::Recent => "recent",
            AppSort::Time => "time",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        let name = name.to_lowercase();
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    pub fn label(self) -> String {
        match self {
            AppSort::Name => t!("sort.name"),
            AppSort::Xp => t!("sort.xp"),
            AppSort::Recent => t!("sort.recent"),
            AppSort::Time => t!("sort.time"),
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&s| s == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// Name ascending; the others biggest or latest first, ties by name.
    pub fn apply(self, apps: &mut [AppEntry]) {
        apps.sort_by_cached_key(|a| a.name.to_lowercase());
        match self {
            AppSort::Name => {}
            AppSort::Xp => apps.sort_by_key(|a| std::cmp::Reverse(a.total_xp)),
            AppSort::Recent => apps.sort_by_key(|a| std::cmp::Reverse(a.last_played)),
            AppSort::Time => apps.sort_by_key(|a| std::cmp::Reverse(a.total_secs)),
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

/// A session in progress, keyed by app id in `App::active_sessions`.
#[derive(Debug, Clone, Copy)]
pub struct ActiveSession {
    /// Row in `sessions`, open until the session ends.
    pub session_id: i64,
    pub started: Instant,
    last_checkpoint: Instant,
}

/// An XP total moving from `from` to `to`, started at tick `start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XpAnim {
    pub from: u32,
    pub to: u32,
    pub start: u64,
}

/// Folder browser of the Storage screen.
#[derive(Debug)]
pub struct Folders {
    /// `None`: the list of drives.
    pub dir: Option<PathBuf>,
    pub entries: Vec<Entry>,
    pub state: TableState,
    /// Still reading `dir`.
    pub listing: bool,
    /// Entry to select once listed (the folder we came back from).
    select: Option<PathBuf>,
    /// Set when leaving `dir`: its measures stop.
    cancel: Arc<AtomicBool>,
    /// Subfolders being measured: percentage of their children done.
    pub progress: HashMap<PathBuf, u8>,
}

/// What a recorded session earned.
#[derive(Debug, Clone, Default)]
struct SessionOutcome {
    xp: u32,
    rewards: Vec<RewardUnlocked>,
    /// Rewards whose rule could not be evaluated, as "rule « code » : error".
    rule_errors: Vec<String>,
}

impl XpAnim {
    /// The total to show at tick `frame`.
    pub fn value(&self, frame: u64) -> u32 {
        xp::animate(
            self.from,
            self.to,
            frame.saturating_sub(self.start),
            XP_ANIM_FRAMES,
        )
    }

    fn is_done(&self, frame: u64) -> bool {
        frame.saturating_sub(self.start) >= XP_ANIM_FRAMES
    }
}

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
    pub active_sessions: HashMap<i64, ActiveSession>,

    // animations, driven by `Tick`
    /// Incremented on each tick.
    pub frame_count: u64,
    /// XP bars still filling up, by app id.
    pub xp_anims: HashMap<i64, XpAnim>,
    pub profile_anim: Option<XpAnim>,
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
    pub db: Database,
    /// Receives the watch list after each reload. `None` in tests.
    tracker: Option<Sender<Vec<Watched>>>,
    /// Lets `:update` start its thread. `None` in tests.
    events: Option<Sender<AppEvent>>,
    update_running: bool,
    /// Newer version found by a check, shown in the status bar.
    pub update_available: Option<String>,
    /// Installed apps for the "add app" picker, rescanned each time it opens.
    pub shortcuts: Vec<Shortcut>,
    pub scan_running: bool,

    // Storage screen, rescanned each time it opens
    pub disks: Vec<Disk>,
    pub programs: Vec<Program>,
    pub storage_state: TableState,
    /// Programs of this drive only; `None`: every drive.
    pub storage_disk: Option<char>,
    /// Smallest programs first instead of biggest.
    pub storage_ascending: bool,
    pub storage_scanning: bool,
    /// `Some`: the folder browser replaces the programs.
    pub folders: Option<Folders>,
    /// Folder sizes measured so far, kept while browsing back and forth.
    folder_sizes: HashMap<PathBuf, u64>,

    // Optimization screen, nothing saved
    /// Read when the screen first opens.
    pub system: Option<optimize::System>,
    /// On or off, read each time the screen opens.
    pub gaming: Vec<(Gaming, bool)>,
    pub bench_state: TableState,
    pub gaming_state: TableState,
    /// The gaming settings have the focus instead of the benchmarks.
    pub gaming_focus: bool,
    pub bench_heavy: bool,
    pub bench_running: Option<Bench>,
    /// Last result of each benchmark, with whether it ran heavy.
    pub bench_results: HashMap<Bench, (bool, Result<Score, String>)>,
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
            aliases: Aliases::default(),
            active_sessions: HashMap::new(),
            frame_count: 0,
            xp_anims: HashMap::new(),
            profile_anim: None,
            pending_popups: VecDeque::new(),
            command_line: CommandLine::default(),
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
            disks: Vec::new(),
            programs: Vec::new(),
            storage_state: TableState::default(),
            storage_disk: None,
            storage_ascending: false,
            storage_scanning: false,
            folders: None,
            folder_sizes: HashMap::new(),
            system: None,
            gaming: Vec::new(),
            bench_state: TableState::default().with_selected(0),
            gaming_state: TableState::default().with_selected(0),
            gaming_focus: false,
            bench_heavy: false,
            bench_running: None,
            bench_results: HashMap::new(),
            should_quit: false,
        };
        app.reload()?;
        Ok(app)
    }

    /// Refreshes the cached data and keeps both selections in range.
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

    fn send_watch_list(&self) {
        let Some(tracker) = &self.tracker else { return };
        let list = self
            .apps
            .iter()
            .filter_map(|a| {
                let exe = a.watch_exe.as_deref()?.trim();
                (!exe.is_empty()).then(|| Watched {
                    app_id: a.id,
                    exe: exe.into(),
                })
            })
            .collect();
        // A closed channel means the tracker died: sessions just stop being tracked.
        let _ = tracker.send(list);
    }

    /// Main loop: redraw, then handle the next event from the event or tracker thread.
    /// Key events are already filtered on `Press` by the event thread.
    pub fn run(
        &mut self,
        terminal: &mut DefaultTerminal,
        events: &Receiver<AppEvent>,
    ) -> anyhow::Result<()> {
        while !self.should_quit {
            terminal.draw(|f| ui::draw(f, self))?;
            self.handle(events.recv()?);
        }
        self.end_all_sessions()
    }

    /// Routes one event from the event, tracker or a short-lived thread.
    pub fn handle(&mut self, event: AppEvent) {
        match event {
            AppEvent::Key(key) => self.on_key(key),
            AppEvent::Tick => self.on_tick(),
            AppEvent::SessionStarted { app_id } => self.on_session_start(app_id),
            AppEvent::SessionEnded { app_id, secs } => self.on_session_end(app_id, secs),
            AppEvent::UpdateFinished { action, result } => self.on_update_finished(action, result),
            AppEvent::StorageScanned { disks, programs } => {
                self.on_storage_scanned(disks, programs)
            }
            AppEvent::FolderListed { dir, entries } => self.on_folder_listed(&dir, entries),
            AppEvent::FolderProgress { path, percent } => self.on_folder_progress(path, percent),
            AppEvent::FolderSized { path, size } => self.on_folder_sized(path, size),
            AppEvent::Trashed { path, result } => self.on_trashed(&path, result),
            AppEvent::BenchFinished {
                bench,
                heavy,
                result,
            } => self.on_bench_finished(bench, heavy, result),
            AppEvent::ShortcutsScanned(found) => {
                self.scan_running = false;
                self.shortcuts = found;
                // The list changed: back to the best match.
                if let Mode::Popup(Popup::Picker(picker)) = &mut self.mode {
                    picker.selected = 0;
                }
            }
        }
    }

    pub fn on_update_finished(
        &mut self,
        action: update::Action,
        result: Result<update::Outcome, String>,
    ) {
        use update::{Action, Outcome};
        self.update_running = false;
        let message = match (action, result) {
            // The passive check stays silent, apart from the status bar.
            (Action::Check, Ok(Outcome::Available { version, .. })) => {
                self.update_available = Some(version);
                return;
            }
            (Action::Check, _) => return,
            (Action::Install, Err(error)) => (t!("update.failed", error), MsgKind::Error),
            (Action::Install, Ok(Outcome::UpToDate)) => {
                self.update_available = None;
                (
                    t!("update.up_to_date", version = update::CURRENT),
                    MsgKind::Info,
                )
            }
            (Action::Install, Ok(Outcome::Available { version, .. })) => {
                let text = t!("update.use_winget", version);
                self.update_available = Some(version);
                (text, MsgKind::Info)
            }
            (Action::Install, Ok(Outcome::Installed { version })) => {
                self.update_available = None;
                (t!("update.installed", version), MsgKind::Success)
            }
        };
        self.message = Some(message);
    }

    /// Each tick redraws (live timer, animations); running sessions also checkpoint here.
    pub fn on_tick(&mut self) {
        self.frame_count += 1;
        let frame = self.frame_count;
        self.xp_anims.retain(|_, anim| !anim.is_done(frame));
        if self.profile_anim.is_some_and(|anim| anim.is_done(frame)) {
            self.profile_anim = None;
        }
        self.show_pending_popup();

        for session in self.active_sessions.values_mut() {
            if session.last_checkpoint.elapsed() < CHECKPOINT_EVERY {
                continue;
            }
            session.last_checkpoint = Instant::now();
            let secs = session.started.elapsed().as_secs();
            if let Err(e) = self.db.checkpoint_session(session.session_id, secs) {
                self.message = Some((format!("{e:#}"), MsgKind::Error));
            }
        }
    }

    pub fn on_session_start(&mut self, app_id: i64) {
        if self.active_sessions.contains_key(&app_id) {
            return;
        }
        // The app may have been removed since the tracker's last poll.
        let Some(name) = self.app_name(app_id) else {
            return;
        };
        match self.db.start_session(app_id, unix_now()) {
            Ok(session_id) => {
                let now = Instant::now();
                self.active_sessions.insert(
                    app_id,
                    ActiveSession {
                        session_id,
                        started: now,
                        last_checkpoint: now,
                    },
                );
                self.message = Some((t!("session.started", name), MsgKind::Info));
            }
            Err(e) => self.message = Some((format!("{e:#}"), MsgKind::Error)),
        }
    }

    /// `secs` is measured by the tracker, from detection to disappearance.
    pub fn on_session_end(&mut self, app_id: i64, secs: u64) {
        let Some(session) = self.active_sessions.remove(&app_id) else {
            return;
        };
        let Some(before) = self.find_app_by_id(app_id).map(|a| a.total_xp) else {
            return; // removed meanwhile
        };
        let profile_before = self.profile.total_xp;
        let result = self
            .finish_session(session.session_id, secs)
            .and_then(|outcome| self.reload().map(|()| outcome));
        let Some(name) = self.app_name(app_id) else {
            return;
        };
        let message = match result {
            Ok(Some(outcome)) => {
                let text = t!("session.ended", name, minutes = secs / 60, xp = outcome.xp);
                let message = match outcome.rule_errors.first() {
                    Some(error) => (format!("{text} · {error}"), MsgKind::Error),
                    None => (text, MsgKind::Success),
                };
                self.on_xp_changed(app_id, before, profile_before, outcome.xp);
                self.queue_rewards(outcome.rewards);
                message
            }
            Ok(None) => (t!("session.too_short", name), MsgKind::Info),
            Err(e) => (format!("{e:#}"), MsgKind::Error),
        };
        self.message = Some(message);
    }

    /// Closes a session, then rewards it. Returns `None` if the session was too short to
    /// be recorded.
    fn finish_session(&self, session_id: i64, secs: u64) -> anyhow::Result<Option<SessionOutcome>> {
        if !self.db.end_session(session_id, unix_now(), secs)? {
            return Ok(None);
        }
        self.reward_session(session_id, secs).map(Some)
    }

    /// XP first, so rules see the new levels, then rewards.
    fn reward_session(&self, session_id: i64, secs: u64) -> anyhow::Result<SessionOutcome> {
        let xp = self.award_session_xp(session_id, secs)?;
        let (rewards, rule_errors) = self.unlock_rewards(session_id)?;
        Ok(SessionOutcome {
            xp,
            rewards,
            rule_errors,
        })
    }

    /// XP of a closed session, with the streak bonus. The streak includes today, now that
    /// the session is closed.
    fn award_session_xp(&self, session_id: i64, secs: u64) -> anyhow::Result<u32> {
        let streak = self.db.profile()?.streak_days;
        let gained = xp::xp_for_session((secs / 60) as u32, streak);
        if gained > 0 {
            self.db.add_session_xp(session_id, gained)?;
        }
        Ok(gained)
    }

    /// Evaluates the rewards the session's app can still unlock, and unlocks those whose
    /// rule passes. A broken rule is reported, not fatal: the other rewards still count.
    fn unlock_rewards(
        &self,
        session_id: i64,
    ) -> anyhow::Result<(Vec<RewardUnlocked>, Vec<String>)> {
        let (app_id, facts) = self.db.session_facts(session_id)?;
        let app_name = self.app_name(app_id);
        let (mut unlocked, mut errors) = (Vec::new(), Vec::new());
        for reward in self.db.pending_rewards(app_id)? {
            match rewards::evaluate(&reward.rule, &facts) {
                Ok(true) => {
                    let for_app = reward.per_app.then_some(app_id);
                    self.db
                        .unlock_reward(reward.id, for_app, session_id, unix_now())?;
                    unlocked.push(RewardUnlocked {
                        name: reward.name,
                        description: reward.description,
                        app: if reward.per_app {
                            app_name.clone()
                        } else {
                            None
                        },
                    });
                }
                Ok(false) => {}
                Err(error) => errors.push(t!("session.rule_error", code = reward.code, error)),
            }
        }
        Ok((unlocked, errors))
    }

    /// At startup, closes the sessions a crash left open and rewards them.
    pub fn close_orphan_sessions(&mut self) -> anyhow::Result<()> {
        let closed = self.db.close_orphan_sessions()?;
        if closed.is_empty() {
            return Ok(());
        }
        let mut errors = Vec::new();
        let mut unlocked = Vec::new();
        for session in &closed {
            let outcome = self.reward_session(session.session_id, session.secs)?;
            unlocked.extend(outcome.rewards);
            errors.extend(outcome.rule_errors);
        }
        self.reload()?;
        self.queue_rewards(unlocked);
        let text = t!("session.recovered", count = closed.len());
        self.message = Some(match errors.first() {
            Some(error) => (format!("{text} · {error}"), MsgKind::Error),
            None => (text, MsgKind::Info),
        });
        Ok(())
    }

    fn queue_rewards(&mut self, unlocked: Vec<RewardUnlocked>) {
        self.pending_popups
            .extend(unlocked.into_iter().map(Popup::RewardUnlocked));
        self.show_pending_popup();
    }

    /// On quit, closes running sessions as if their apps had stopped.
    fn end_all_sessions(&mut self) -> anyhow::Result<()> {
        let sessions: Vec<_> = self.active_sessions.drain().collect();
        for (_, session) in sessions {
            self.finish_session(session.session_id, session.started.elapsed().as_secs())?;
        }
        Ok(())
    }

    /// After an app's XP changed (data already reloaded): animates its bar and the
    /// profile's, and queues a level-up popup if a level went up.
    fn on_xp_changed(&mut self, app_id: i64, app_before: u32, profile_before: u32, gained: u32) {
        let Some(entry) = self.find_app_by_id(app_id) else {
            return;
        };
        let (name, app_after, app_level) = (entry.name.clone(), entry.total_xp, entry.level);
        let frame = self.frame_count;

        // Start from what is on screen, in case a previous animation is still running.
        let from = self
            .xp_anims
            .get(&app_id)
            .map_or(app_before, |a| a.value(frame));
        self.xp_anims.insert(
            app_id,
            XpAnim {
                from,
                to: app_after,
                start: frame,
            },
        );
        let from = self.profile_anim.map_or(profile_before, |a| a.value(frame));
        self.profile_anim = Some(XpAnim {
            from,
            to: self.profile.total_xp,
            start: frame,
        });

        let went_up =
            |before: u32, level: u32| (level > xp::level_from_total(before).0).then_some(level);
        let level_up = LevelUp {
            app: name,
            app_level: went_up(app_before, app_level),
            global_level: went_up(profile_before, self.profile.level),
            gained,
        };
        if level_up.app_level.is_some() || level_up.global_level.is_some() {
            self.pending_popups.push_back(Popup::LevelUp(level_up));
            self.show_pending_popup();
        }
    }

    /// Opens the next queued popup, unless the user is busy typing or answering another.
    fn show_pending_popup(&mut self) {
        if self.mode == Mode::Normal
            && let Some(popup) = self.pending_popups.pop_front()
        {
            self.mode = Mode::Popup(popup);
        }
    }

    /// `(level, xp within that level)` to display for an app, following its animation.
    pub fn shown_app_xp(&self, entry: &AppEntry) -> (u32, u32) {
        match self.xp_anims.get(&entry.id) {
            Some(anim) => xp::level_from_total(anim.value(self.frame_count)),
            None => (entry.level, entry.xp),
        }
    }

    /// `(level, xp within that level)` to display for the profile, following its animation.
    pub fn shown_profile_xp(&self) -> (u32, u32) {
        match self.profile_anim {
            Some(anim) => xp::level_from_total(anim.value(self.frame_count)),
            None => (self.profile.level, self.profile.xp),
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.execute(Command::Quit);
            return;
        }
        match &self.mode {
            Mode::Normal => self.on_normal_key(key),
            Mode::Command => self.on_command_key(key),
            Mode::Search => self.on_search_key(key),
            Mode::Popup(_) => self.on_popup_key(key),
        }
    }

    fn on_normal_key(&mut self, key: KeyEvent) {
        self.message = None;
        match key.code {
            KeyCode::Char(':') => self.mode = Mode::Command,
            KeyCode::Char('/') => self.start_search(),
            _ => {
                if let Some(command) = self.key_to_command(key) {
                    self.execute(command);
                }
            }
        }
    }

    /// `/`: searches every app from the dashboard. Esc puts the selection back.
    fn start_search(&mut self) {
        self.screen = Screen::Dashboard;
        self.search_restore = Some((self.focus, self.app_state.selected()));
        self.search = TextInput::default();
        self.mode = Mode::Search;
        self.focus = Focus::Apps;
        self.app_state
            .select(clamp(Some(0), self.visible_apps().len()));
    }

    fn on_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.cancel_search(),
            KeyCode::Backspace if self.search.is_empty() => self.cancel_search(),
            KeyCode::Enter => match self.selected_app().map(|a| a.name.clone()) {
                Some(app) => {
                    self.mode = Mode::Normal;
                    self.search_restore = None;
                    self.execute(Command::Select { app });
                }
                None => self.cancel_search(), // no match
            },
            KeyCode::Down | KeyCode::Tab => self.move_selection(true),
            KeyCode::Up | KeyCode::BackTab => self.move_selection(false),
            _ => {
                if self.search.handle_key(key) {
                    // The best match comes first: select it again after each edit.
                    self.app_state
                        .select(clamp(Some(0), self.visible_apps().len()));
                }
            }
        }
    }

    fn cancel_search(&mut self) {
        self.mode = Mode::Normal;
        if let Some((focus, selected)) = self.search_restore.take() {
            self.focus = focus;
            self.app_state.select(selected);
        }
        let visible = self.visible_apps().len();
        self.app_state
            .select(clamp(self.app_state.selected(), visible));
    }

    /// Normal-mode key bindings (plan section 7).
    fn key_to_command(&self, key: KeyEvent) -> Option<Command> {
        Some(match key.code {
            KeyCode::Char('q') => Command::Quit,
            KeyCode::Char('1') => Command::Show(Screen::Dashboard),
            KeyCode::Char('2') => Command::Show(Screen::Stats),
            KeyCode::Char('3') => Command::Show(Screen::Rewards),
            KeyCode::Char('4') => Command::Show(Screen::Storage),
            KeyCode::Char('5') => Command::Show(Screen::Optimize),
            KeyCode::Char('0') | KeyCode::Char('?') => Command::Show(Screen::Help),
            KeyCode::Down | KeyCode::Char('j') => Command::SelectNext,
            KeyCode::Up | KeyCode::Char('k') => Command::SelectPrev,
            _ if self.screen == Screen::Storage => self.storage_key(key)?,
            _ if self.screen == Screen::Optimize => self.optimize_key(key)?,
            KeyCode::Tab | KeyCode::BackTab => Command::ToggleFocus,
            KeyCode::Left | KeyCode::Char('h') => Command::FocusPanel(Focus::Categories),
            KeyCode::Right | KeyCode::Char('l') => Command::FocusPanel(Focus::Apps),
            KeyCode::Char('s') if self.screen == Screen::Stats => Command::ToggleStatsPie,
            _ if self.screen != Screen::Dashboard => return None,
            KeyCode::Enter => match self.focus {
                Focus::Categories => Command::FocusPanel(Focus::Apps),
                Focus::Apps => Command::Launch {
                    app: self.selected_app()?.name.clone(),
                },
            },
            KeyCode::Char('a') => Command::OpenForm(FormKind::AddApp),
            KeyCode::Char('s') => Command::Sort {
                by: Some(self.sort.next()),
            },
            KeyCode::Char('m') => Command::OpenForm(FormKind::MoveApp {
                app: self.selected_app()?.name.clone(),
            }),
            KeyCode::Char('d') => match self.focus {
                Focus::Categories => Command::RemoveCategory {
                    category: self.selected_category()?.name.clone(),
                    confirmed: false,
                },
                Focus::Apps => Command::RemoveApp {
                    app: self.selected_app()?.name.clone(),
                    confirmed: false,
                },
            },
            _ => return None,
        })
    }

    /// Storage screen: Tab/←→ pick the drive, `s` the order, `d`/Del uninstall,
    /// `f` the folder browser.
    fn storage_key(&self, key: KeyEvent) -> Option<Command> {
        if self.folders.is_some() {
            return self.folder_key(key);
        }
        Some(match key.code {
            KeyCode::Char('f') => Command::ToggleFolders,
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                Command::CycleDisk { forward: true }
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                Command::CycleDisk { forward: false }
            }
            KeyCode::Char('s') => Command::ToggleStorageOrder,
            KeyCode::Char('d') | KeyCode::Delete => {
                let i = self.storage_state.selected()?;
                Command::Uninstall {
                    program: self.visible_programs().get(i)?.name.clone(),
                    confirmed: false,
                }
            }
            _ => return None,
        })
    }

    /// Folder browser: Enter/→ open, Backspace/← back, `d`/Del to the Recycle Bin (or
    /// uninstall, for a program's folder).
    fn folder_key(&self, key: KeyEvent) -> Option<Command> {
        Some(match key.code {
            KeyCode::Char('f') => Command::ToggleFolders,
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => Command::OpenFolder,
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => Command::ParentFolder,
            KeyCode::Char('s') => Command::ToggleStorageOrder,
            KeyCode::Char('d') | KeyCode::Delete => {
                self.folders.as_ref()?.dir.as_ref()?; // drives cannot be deleted
                let entry = self.selected_entry()?;
                match programs::installed_in(&self.programs, &entry.path) {
                    Some(program) => Command::Uninstall {
                        program: program.name.clone(),
                        confirmed: false,
                    },
                    None => Command::Trash {
                        path: entry.path.clone(),
                        confirmed: false,
                    },
                }
            }
            _ => return None,
        })
    }

    /// Optimization screen: Tab switches between benchmarks and gaming settings, Enter runs
    /// or switches, `n` light/heavy, `o` the setting's Windows page.
    fn optimize_key(&self, key: KeyEvent) -> Option<Command> {
        Some(match key.code {
            KeyCode::Tab | KeyCode::BackTab => Command::ToggleFocus,
            KeyCode::Char('n') => Command::ToggleBenchLevel,
            KeyCode::Enter if self.gaming_focus => {
                Command::ToggleGaming(self.gaming.get(self.gaming_state.selected()?)?.0)
            }
            KeyCode::Char('o') if self.gaming_focus => {
                Command::OpenGamingPage(self.gaming.get(self.gaming_state.selected()?)?.0)
            }
            KeyCode::Enter => Command::Bench(*Bench::ALL.get(self.bench_state.selected()?)?),
            _ => return None,
        })
    }

    fn on_command_key(&mut self, key: KeyEvent) {
        if let KeyCode::Tab | KeyCode::BackTab = key.code {
            self.complete_command(key.code == KeyCode::Tab);
            return;
        }
        let line = &mut self.command_line;
        line.completion = None;
        match key.code {
            KeyCode::Esc => {
                line.clear();
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => {
                let text = line.submit();
                self.mode = Mode::Normal;
                self.run_line(&text);
            }
            // Backspace on an empty line leaves command mode, like Vim.
            KeyCode::Backspace if line.input.is_empty() => self.mode = Mode::Normal,
            KeyCode::Up => line.history_prev(),
            KeyCode::Down => line.history_next(),
            _ => {
                line.input.handle_key(key);
            }
        }
    }

    fn on_popup_key(&mut self, key: KeyEvent) {
        let Mode::Popup(popup) = &mut self.mode else {
            return;
        };
        match popup {
            Popup::Picker(_) => self.on_picker_key(key),
            Popup::Confirm { command, .. } => match key.code {
                KeyCode::Enter | KeyCode::Char('o' | 'O' | 'y' | 'Y') => {
                    let command = command.clone();
                    self.mode = Mode::Normal;
                    self.execute(command);
                }
                KeyCode::Esc | KeyCode::Char('n' | 'N') => self.cancel_popup(),
                _ => {}
            },
            Popup::Form(form) => match key.code {
                KeyCode::Esc => self.cancel_popup(),
                KeyCode::Tab | KeyCode::Down => form.next_field(),
                KeyCode::BackTab | KeyCode::Up => form.prev_field(),
                KeyCode::Enter if !form.is_last_field() => form.next_field(),
                KeyCode::Enter => self.submit_form(),
                _ => {
                    if form.focused_input().handle_key(key) {
                        form.error = None;
                    }
                }
            },
            Popup::LevelUp(_) | Popup::RewardUnlocked(_) => {
                if matches!(key.code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char(' ')) {
                    self.mode = Mode::Normal;
                    self.show_pending_popup();
                }
            }
        }
    }

    /// Scans the installed apps in a short-lived thread (about 0.1 s of disk reads).
    fn start_shortcut_scan(&mut self) {
        let Some(events) = self.events.clone() else {
            return;
        };
        if self.scan_running {
            return;
        }
        self.scan_running = true;
        std::thread::spawn(move || {
            let _ = events.send(AppEvent::ShortcutsScanned(scan::scan()));
        });
    }

    /// Reads the disks and installed programs in a short-lived thread.
    fn start_storage_scan(&mut self) {
        let Some(events) = self.events.clone() else {
            return;
        };
        if self.storage_scanning {
            return;
        }
        self.storage_scanning = true;
        std::thread::spawn(move || {
            let (disks, programs) = rayon::join(programs::disks, programs::installed);
            let _ = events.send(AppEvent::StorageScanned { disks, programs });
        });
    }

    pub fn on_storage_scanned(&mut self, disks: Vec<Disk>, programs: Vec<Program>) {
        self.storage_scanning = false;
        self.disks = disks;
        self.programs = programs;
        if self
            .storage_disk
            .is_some_and(|letter| !self.disks.iter().any(|d| d.letter == letter))
        {
            self.storage_disk = None; // drive unplugged
        }
        let visible = self.visible_programs().len();
        self.storage_state
            .select(clamp(self.storage_state.selected(), visible));
    }

    /// Programs of the chosen drive, by size (unknown sizes last), ties by name.
    pub fn visible_programs(&self) -> Vec<&Program> {
        let mut list: Vec<&Program> = self
            .programs
            .iter()
            .filter(|p| self.storage_disk.is_none() || p.drive == self.storage_disk)
            .collect();
        // `programs` is sorted by name and the sort is stable.
        list.sort_by(|a, b| by_size(a.size, b.size, self.storage_ascending));
        list
    }

    /// Folder browser entries, by size like the programs (unmeasured last).
    pub fn visible_entries(&self) -> Vec<&Entry> {
        let Some(folders) = &self.folders else {
            return Vec::new();
        };
        let mut list: Vec<&Entry> = folders.entries.iter().collect();
        list.sort_by(|a, b| by_size(a.size, b.size, self.storage_ascending));
        list
    }

    fn selected_entry(&self) -> Option<&Entry> {
        let i = self.folders.as_ref()?.state.selected()?;
        self.visible_entries().get(i).copied()
    }

    /// Selects the entry at `path`, else the first one.
    fn select_entry(&mut self, path: Option<&Path>) {
        let entries = self.visible_entries();
        let index = path
            .and_then(|p| entries.iter().position(|e| e.path == p))
            .or((!entries.is_empty()).then_some(0));
        if let Some(folders) = &mut self.folders {
            folders.state.select(index);
        }
    }

    /// Shows `dir` (`None`: the drives) and reads it in a short-lived thread. Stops the
    /// measures of the folder being left.
    fn open_folder(&mut self, dir: Option<PathBuf>, select: Option<PathBuf>) {
        self.stop_measures();
        let entries = match &dir {
            Some(_) => Vec::new(),
            None => self
                .disks
                .iter()
                .map(|d| Entry {
                    name: format!("{}:", d.letter),
                    path: PathBuf::from(format!("{}:\\", d.letter)),
                    is_dir: true,
                    size: Some(d.total - d.free.min(d.total)),
                })
                .collect(),
        };
        self.folders = Some(Folders {
            listing: dir.is_some(),
            dir: dir.clone(),
            entries,
            state: TableState::default(),
            select: select.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
            progress: HashMap::new(),
        });
        self.select_entry(select.as_deref());
        if let (Some(dir), Some(events)) = (dir, self.events.clone()) {
            std::thread::spawn(move || {
                let entries = folders::list(&dir);
                let _ = events.send(AppEvent::FolderListed { dir, entries });
            });
        }
    }

    fn stop_measures(&self) {
        if let Some(folders) = &self.folders {
            folders.cancel.store(true, atomic::Ordering::Relaxed);
        }
    }

    /// Fills in the sizes already known, then measures the other subfolders in a
    /// short-lived thread, one `FolderSized` each.
    pub fn on_folder_listed(&mut self, dir: &Path, mut entries: Vec<Entry>) {
        let Some(folders) = &mut self.folders else {
            return;
        };
        if folders.dir.as_deref() != Some(dir) {
            return; // left meanwhile
        }
        entries.sort_by_cached_key(|e| e.name.to_lowercase());
        for entry in entries.iter_mut().filter(|e| e.is_dir) {
            entry.size = self.folder_sizes.get(&entry.path).copied();
        }
        let todo: Vec<PathBuf> = entries
            .iter()
            .filter(|e| e.is_dir && e.size.is_none())
            .map(|e| e.path.clone())
            .collect();
        folders.entries = entries;
        folders.listing = false;
        let (select, cancel) = (folders.select.take(), folders.cancel.clone());
        self.select_entry(select.as_deref());

        let Some(events) = self.events.clone() else {
            return;
        };
        std::thread::spawn(move || {
            use rayon::prelude::*;
            todo.into_par_iter().for_each(|path| {
                let progress = |percent| {
                    let path = path.clone();
                    let _ = events.send(AppEvent::FolderProgress { path, percent });
                };
                if let Some(size) = folders::dir_size_with_progress(&path, &cancel, progress) {
                    let _ = events.send(AppEvent::FolderSized { path, size });
                }
            });
        });
    }

    pub fn on_folder_progress(&mut self, path: PathBuf, percent: u8) {
        if let Some(folders) = &mut self.folders {
            folders.progress.insert(path, percent);
        }
    }

    /// Keeps the selection on the same entry while the list reorders.
    pub fn on_folder_sized(&mut self, path: PathBuf, size: u64) {
        let selected = self.selected_entry().map(|e| e.path.clone());
        if let Some(folders) = &mut self.folders
            && let Some(entry) = folders.entries.iter_mut().find(|e| e.path == path)
        {
            entry.size = Some(size);
        }
        if let Some(folders) = &mut self.folders {
            folders.progress.remove(&path);
        }
        self.folder_sizes.insert(path, size);
        self.select_entry(selected.as_deref());
    }

    pub fn on_trashed(&mut self, path: &Path, result: Result<(), String>) {
        let name = path.display();
        self.message = Some(match result {
            Ok(()) => {
                // Its size, and the sizes of the folders holding it, are stale.
                self.folder_sizes
                    .retain(|p, _| !path.starts_with(p) && !p.starts_with(path));
                let selected = self.selected_entry().map(|e| e.path.clone());
                if let Some(folders) = &mut self.folders {
                    folders.entries.retain(|e| e.path != path);
                }
                self.select_entry(selected.as_deref());
                (t!("storage.trashed", path = name), MsgKind::Success)
            }
            Err(error) => (error, MsgKind::Error),
        });
    }

    /// Installed apps matching the picker's query, best first, minus the ones already added.
    pub fn picker_matches(&self, picker: &Picker) -> Vec<&Shortcut> {
        let candidates: Vec<&Shortcut> = self
            .shortcuts
            .iter()
            .filter(|s| {
                !self
                    .apps
                    .iter()
                    .any(|a| a.launch_target.eq_ignore_ascii_case(&s.target))
            })
            .collect();
        fuzzy::rank(
            picker.query.text(),
            candidates.iter().map(|s| s.name.as_str()),
        )
        .into_iter()
        .map(|i| candidates[i])
        .collect()
    }

    fn on_picker_key(&mut self, key: KeyEvent) {
        let Mode::Popup(Popup::Picker(picker)) = &self.mode else {
            return;
        };
        let matches = self.picker_matches(picker);
        let count = matches.len();
        let chosen = matches.get(picker.selected).map(|s| (*s).clone());
        let (query, category) = (
            picker.query.text().trim().to_string(),
            picker.category.clone(),
        );
        let Mode::Popup(Popup::Picker(picker)) = &mut self.mode else {
            return;
        };
        match (key.code, chosen) {
            (KeyCode::Esc, _) => self.cancel_popup(),
            (KeyCode::Down, _) if count > 0 => picker.selected = (picker.selected + 1) % count,
            (KeyCode::Up, _) if count > 0 => {
                picker.selected = (picker.selected + count - 1) % count
            }
            (KeyCode::Enter, Some(s)) => {
                let form =
                    Form::add_app_from(&category, &s.name, &s.target, s.watch_exe.as_deref());
                self.mode = Mode::Popup(Popup::Form(form));
            }
            // Tab, or Enter without a match: fill in by hand, keeping what was typed as the name.
            (KeyCode::Tab | KeyCode::Enter, _) => {
                let form = Form::add_app_from(&category, &query, "", None);
                self.mode = Mode::Popup(Popup::Form(form));
            }
            _ => {
                if picker.query.handle_key(key) {
                    picker.selected = 0;
                }
            }
        }
    }

    /// Runs the form's command. On error the form stays open and shows it.
    fn submit_form(&mut self) {
        let Mode::Popup(Popup::Form(form)) = &mut self.mode else {
            return;
        };
        let command = match form.build_command() {
            Ok(command) => command,
            Err(e) => {
                form.error = Some(e);
                return;
            }
        };
        match self.run_command(command) {
            Ok(message) => {
                self.mode = Mode::Normal;
                self.message = message;
            }
            Err(e) => {
                if let Mode::Popup(Popup::Form(form)) = &mut self.mode {
                    form.error = Some(format!("{e:#}"));
                }
            }
        }
    }

    fn cancel_popup(&mut self) {
        self.mode = Mode::Normal;
        self.message = Some((t!("cancelled"), MsgKind::Info));
    }

    /// Tab in the command line: completes commands, apps and categories.
    fn complete_command(&mut self, forward: bool) {
        let themes = theme::available(self.themes_dir.as_deref());
        let sources = Sources {
            themes: themes.iter().map(String::as_str).collect(),
            aliases: self.aliases.names().collect(),
            apps: self.apps.iter().map(|a| a.name.as_str()).collect(),
            categories: self.categories.iter().map(|c| c.name.as_str()).collect(),
            programs: self.programs.iter().map(|p| p.name.as_str()).collect(),
        };
        self.command_line
            .complete(|text| complete::complete(text, &sources), forward);
    }

    /// Runs a typed line: an alias (its commands one after the other) or one command.
    pub fn run_line(&mut self, line: &str) {
        let lines = match self.aliases.expand(line) {
            None => {
                match parser::parse(line) {
                    Ok(command) => self.execute(command),
                    Err(e) => self.message = Some((e, MsgKind::Error)),
                }
                return;
            }
            Some(Err(e)) => {
                self.message = Some((t!("pair", label = "alias", value = e), MsgKind::Error));
                return;
            }
            Some(Ok(lines)) => lines,
        };
        for sub in lines {
            let result = parser::parse(&sub)
                .map_err(anyhow::Error::msg)
                .and_then(|command| self.run_command(command));
            match result {
                Ok(Some(message)) => self.message = Some(message),
                Ok(None) => {}
                Err(e) => {
                    let value = format!("{e:#}");
                    self.message = Some((t!("pair", label = sub, value), MsgKind::Error));
                    return;
                }
            }
            // A confirmation or a form waits for the user: stop there.
            if self.mode != Mode::Normal {
                return;
            }
        }
    }

    /// Single execution path for every `Command`. Shows the outcome as a message.
    /// Reads what the Optimization screen shows: the PC once, the gaming settings each time.
    fn open_optimize(&mut self) {
        if self.system.is_none() {
            self.system = Some(optimize::system());
        }
        self.gaming = Gaming::ALL.iter().map(|&g| (g, g.enabled())).collect();
    }

    pub fn on_bench_finished(&mut self, bench: Bench, heavy: bool, result: Result<Score, String>) {
        self.bench_running = None;
        self.bench_results.insert(bench, (heavy, result));
    }

    pub fn execute(&mut self, command: Command) {
        match self.run_command(command) {
            Ok(Some(message)) => self.message = Some(message),
            Ok(None) => {}
            Err(e) => self.message = Some((format!("{e:#}"), MsgKind::Error)),
        }
    }

    fn run_command(&mut self, command: Command) -> anyhow::Result<Option<Message>> {
        let success = |text: String| Ok(Some((text, MsgKind::Success)));
        match command {
            Command::Show(screen) => self.show(screen),
            Command::SelectNext => self.move_selection(true),
            Command::SelectPrev => self.move_selection(false),
            Command::FocusPanel(focus) => self.focus = focus,
            Command::ToggleFocus if self.screen == Screen::Optimize => {
                self.gaming_focus = !self.gaming_focus
            }
            Command::ToggleFocus => {
                self.focus = match self.focus {
                    Focus::Categories => Focus::Apps,
                    Focus::Apps => Focus::Categories,
                }
            }
            Command::ToggleStatsPie => self.stats_by_app = !self.stats_by_app,
            Command::ToggleBenchLevel => self.bench_heavy = !self.bench_heavy,
            Command::Bench(bench) => self.bench(bench)?,
            Command::ToggleGaming(setting) if !setting.switchable() => {
                return self.run_command(Command::OpenGamingPage(setting));
            }
            Command::ToggleGaming(setting) => return self.toggle_gaming(setting),
            Command::OpenGamingPage(setting) => {
                opener::open(setting.page()).with_context(|| t!("optimize.open_failed"))?;
            }
            Command::CycleDisk { forward } => {
                // `None` (every drive) sits before the first drive.
                let mut choices = vec![None];
                choices.extend(self.disks.iter().map(|d| Some(d.letter)));
                let current = choices.iter().position(|&c| c == self.storage_disk);
                let next = step(current, choices.len(), forward).unwrap_or(0);
                self.storage_disk = choices[next];
                self.reset_storage_selection();
            }
            Command::ToggleStorageOrder => {
                self.storage_ascending = !self.storage_ascending;
                self.reset_storage_selection();
                self.select_entry(None);
            }
            Command::ToggleFolders => self.toggle_folders(),
            Command::OpenFolder => {
                if let Some(entry) = self.selected_entry().filter(|e| e.is_dir) {
                    let path = entry.path.clone();
                    self.open_folder(Some(path), None);
                }
            }
            Command::ParentFolder => {
                if let Some(dir) = self.folders.as_ref().and_then(|f| f.dir.clone()) {
                    // `C:\` has no parent: back to the drives.
                    self.open_folder(dir.parent().map(Path::to_path_buf), Some(dir));
                }
            }
            Command::Trash { path, confirmed } => return self.trash(path, confirmed),
            Command::Quit => self.should_quit = true,

            Command::Launch { app } => {
                let entry = self.app_named(&app)?;
                launch::launch(&entry.launch_target)?;
                return success(t!("action.launched", name = entry.name));
            }
            Command::Add {
                name,
                target,
                category,
                watch_exe,
            } => return self.add_app(name, target, category, watch_exe),
            Command::Move { app, category } => {
                let (id, name) = {
                    let entry = self.app_named(&app)?;
                    (entry.id, entry.name.clone())
                };
                let (category_id, created) = self.category_or_create(&category)?;
                self.db.move_app(id, category_id)?;
                self.reload()?;
                self.select_app(id);
                let category = &self
                    .selected_category()
                    .map_or(category, |c| c.name.clone());
                return success(t!(
                    "action.moved",
                    name,
                    category,
                    note = created_note(created)
                ));
            }
            Command::RemoveApp { app, confirmed } => {
                let (id, name) = {
                    let entry = self.app_named(&app)?;
                    (entry.id, entry.name.clone())
                };
                if !confirmed {
                    self.mode = Mode::Popup(Popup::Confirm {
                        message: t!("action.confirm_remove_app", name),
                        command: Command::RemoveApp {
                            app: name,
                            confirmed: true,
                        },
                    });
                    return Ok(None);
                }
                self.db.delete_app(id)?;
                self.reload()?;
                return success(t!("action.removed", name));
            }
            Command::RemoveCategory {
                category,
                confirmed,
            } => return self.remove_category(&category, confirmed),
            Command::Uninstall { program, confirmed } => {
                let lower = program.to_lowercase();
                let found = self
                    .programs
                    .iter()
                    .find(|p| p.name.to_lowercase() == lower)
                    .with_context(|| t!("storage.unknown_program", name = program))?;
                let name = found.name.clone();
                if !confirmed {
                    self.mode = Mode::Popup(Popup::Confirm {
                        message: t!("storage.confirm_uninstall", name),
                        command: Command::Uninstall {
                            program: name,
                            confirmed: true,
                        },
                    });
                    return Ok(None);
                }
                programs::uninstall(found)?;
                return Ok(Some((t!("storage.uninstalling", name), MsgKind::Info)));
            }
            Command::ClearSessions { confirmed } => {
                if !confirmed {
                    self.mode = Mode::Popup(Popup::Confirm {
                        message: t!("action.confirm_clear_sessions"),
                        command: Command::ClearSessions { confirmed: true },
                    });
                    return Ok(None);
                }
                let count = self.db.hide_sessions()?;
                self.reload()?;
                return success(t!("action.sessions_cleared", count));
            }
            Command::ClearStats { confirmed } => {
                if !confirmed {
                    self.mode = Mode::Popup(Popup::Confirm {
                        message: t!("action.confirm_clear_stats"),
                        command: Command::ClearStats { confirmed: true },
                    });
                    return Ok(None);
                }
                let count = self.db.clear_sessions()?;
                self.reload()?;
                return success(t!("action.stats_cleared", count));
            }
            Command::OpenForm(kind) => self.open_form(kind)?,
            Command::Select { app } => {
                let id = self.app_named(&app)?.id;
                self.screen = Screen::Dashboard;
                self.select_app(id);
            }
            Command::Stats { app } => {
                self.stats_app = match app {
                    Some(app) => Some(self.app_named(&app)?.id),
                    None => None,
                };
                self.stats = self.db.stats(self.stats_app)?;
                self.stats_state = TableState::default()
                    .with_selected((!self.stats.sessions.is_empty()).then_some(0));
                self.screen = Screen::Stats;
            }
            Command::Theme { name: None } => {
                let names = theme::available(self.themes_dir.as_deref());
                let text = t!(
                    "theme.list",
                    list = names.join(", "),
                    current = self.theme_name
                );
                return Ok(Some((text, MsgKind::Info)));
            }
            Command::Theme { name: Some(name) } => {
                // A broken theme file is an error message; the current theme stays.
                self.theme =
                    theme::load(&name, self.themes_dir.as_deref()).map_err(anyhow::Error::msg)?;
                self.theme_name = name.trim().to_lowercase();
                if let Some(path) = &self.config_path {
                    config::save_value(path, "theme", self.theme_name.as_str())
                        .with_context(|| t!("theme.not_saved"))?;
                }
                return success(t!("theme.set", name = self.theme.name));
            }
            Command::Sort { by: None } => {
                let names: Vec<_> = AppSort::ALL.iter().map(|s| s.name()).collect();
                let text = t!(
                    "sort.list",
                    list = names.join(", "),
                    current = self.sort.name()
                );
                return Ok(Some((text, MsgKind::Info)));
            }
            Command::Sort { by: Some(sort) } => return self.set_sort(sort),
            Command::Lang { code: None } => {
                let codes: Vec<_> = i18n::LANGS.iter().map(|(code, _)| *code).collect();
                let text = t!(
                    "lang.list",
                    list = codes.join(", "),
                    current = i18n::current()
                );
                return Ok(Some((text, MsgKind::Info)));
            }
            Command::Lang { code: Some(code) } => return self.set_lang(&code),
            Command::Group { name, apps } => {
                let names = apps
                    .iter()
                    .map(|app| self.app_named(app).map(|entry| entry.name.clone()))
                    .collect::<anyhow::Result<Vec<_>>>()?;
                let body: Vec<String> = names.iter().map(|n| format!("launch {n}")).collect();
                let path = self
                    .config_path
                    .as_deref()
                    .and_then(Path::parent)
                    .with_context(|| t!("error.no_appdata"))?
                    .join("commands.toml");
                self.aliases = Aliases::save(&path, &name, &body.join("; "))?;
                return success(t!(
                    "action.group_saved",
                    name = name.to_lowercase(),
                    list = names.join(", ")
                ));
            }
            Command::Export { path } => {
                let path = match path {
                    Some(path) => PathBuf::from(path),
                    None => self.db.default_export_path()?,
                };
                let (apps, sessions) = self.db.export_to(&path)?;
                return success(t!("action.exported", apps, sessions, path = path.display()));
            }
            Command::Import { path } => {
                let imported = self.db.import_from(Path::new(&path))?;
                self.reload()?;
                return success(t!(
                    "action.imported",
                    apps = imported.apps,
                    sessions = imported.sessions
                ));
            }
            Command::Update => {
                if self.update_running {
                    bail!(t!("update.running"));
                }
                let events = self
                    .events
                    .clone()
                    .with_context(|| t!("update.unavailable"))?;
                self.update_running = true;
                update::spawn(events, update::Action::Install);
                return Ok(Some((t!("update.searching"), MsgKind::Info)));
            }
            Command::Help { command: None } => self.screen = Screen::Help,
            Command::Help {
                command: Some(name),
            } => return self.help(&name),
        }
        Ok(None)
    }

    fn show(&mut self, screen: Screen) {
        self.screen = screen;
        if screen == Screen::Storage {
            self.start_storage_scan();
        }
        if screen == Screen::Optimize {
            self.open_optimize();
        }
    }

    fn bench(&mut self, bench: Bench) -> anyhow::Result<()> {
        if self.bench_running.is_some() {
            bail!(t!("optimize.busy"));
        }
        let Some(events) = self.events.clone() else {
            return Ok(());
        };
        self.bench_running = Some(bench);
        let heavy = self.bench_heavy;
        std::thread::spawn(move || {
            let result = optimize::run(bench, heavy);
            let _ = events.send(AppEvent::BenchFinished {
                bench,
                heavy,
                result,
            });
        });
        Ok(())
    }

    fn toggle_gaming(&mut self, setting: Gaming) -> anyhow::Result<Option<Message>> {
        let on = !setting.enabled();
        setting.set(on).with_context(|| t!("optimize.set_failed"))?;
        self.open_optimize();
        let state = if on {
            t!("optimize.on")
        } else {
            t!("optimize.off")
        };
        Ok(Some((
            t!("optimize.switched", name = setting.label(), state),
            MsgKind::Success,
        )))
    }

    fn toggle_folders(&mut self) {
        if self.folders.is_some() {
            self.stop_measures();
            self.folders = None;
            return;
        }
        // Starts on the chosen drive, else on the list of drives.
        let root = self
            .storage_disk
            .map(|letter| PathBuf::from(format!("{letter}:\\")));
        self.open_folder(root, None);
    }

    fn trash(&mut self, path: PathBuf, confirmed: bool) -> anyhow::Result<Option<Message>> {
        if !confirmed {
            self.mode = Mode::Popup(Popup::Confirm {
                message: t!("storage.confirm_trash", path = path.display()),
                command: Command::Trash {
                    path,
                    confirmed: true,
                },
            });
            return Ok(None);
        }
        let events = self
            .events
            .clone()
            .with_context(|| t!("storage.unavailable"))?;
        let text = t!("storage.trashing", path = path.display());
        std::thread::spawn(move || {
            let result = folders::trash(&path).map_err(|e| format!("{e:#}"));
            let _ = events.send(AppEvent::Trashed { path, result });
        });
        Ok(Some((text, MsgKind::Info)))
    }

    fn add_app(
        &mut self,
        name: String,
        target: String,
        category: Option<String>,
        watch_exe: Option<String>,
    ) -> anyhow::Result<Option<Message>> {
        if self.find_app(&name).is_some() {
            bail!(t!("error.app_exists", name));
        }
        launch::check_target(&target)?;
        let (category_id, created) = match category {
            Some(category) => self.category_or_create(&category)?,
            None => {
                let selected = self
                    .selected_category()
                    .with_context(|| t!("error.no_category"))?;
                (selected.id, false)
            }
        };
        let id = self.db.add_app(&NewApp {
            watch_exe: watch_exe.or_else(|| launch::watch_exe_for(&target)),
            name: name.clone(),
            launch_target: target,
            category_id,
        })?;
        self.reload()?;
        self.select_app(id);
        Ok(Some((
            t!("action.added", name, note = created_note(created)),
            MsgKind::Success,
        )))
    }

    fn remove_category(
        &mut self,
        category: &str,
        confirmed: bool,
    ) -> anyhow::Result<Option<Message>> {
        let (id, name) = {
            let found = self.category_named(category)?;
            (found.id, found.name.clone())
        };
        let count = self.app_count(id);
        if count > 0 {
            bail!(t!("error.category_not_empty", name, count));
        }
        if !confirmed {
            self.mode = Mode::Popup(Popup::Confirm {
                message: t!("action.confirm_remove_category", name),
                command: Command::RemoveCategory {
                    category: name,
                    confirmed: true,
                },
            });
            return Ok(None);
        }
        self.db.delete_category(id)?;
        self.reload()?;
        Ok(Some((
            t!("action.category_removed", name),
            MsgKind::Success,
        )))
    }

    fn open_form(&mut self, kind: FormKind) -> anyhow::Result<()> {
        let form = match kind {
            FormKind::AddApp => {
                let category = self.selected_category().map_or("", |c| c.name.as_str());
                let picker = Picker::new(category);
                self.mode = Mode::Popup(Popup::Picker(picker));
                self.start_shortcut_scan();
                return Ok(());
            }
            FormKind::MoveApp { app } => {
                let entry = self.app_named(&app)?;
                let category = self
                    .categories
                    .iter()
                    .find(|c| c.id == entry.category_id)
                    .map_or("", |c| c.name.as_str());
                Form::move_app(&entry.name, category)
            }
        };
        self.mode = Mode::Popup(Popup::Form(form));
        Ok(())
    }

    fn set_sort(&mut self, sort: AppSort) -> anyhow::Result<Option<Message>> {
        let selected = self.selected_app().map(|a| a.id);
        self.sort = sort;
        self.sort.apply(&mut self.apps);
        if let Some(id) = selected {
            self.select_app(id);
        }
        if let Some(path) = &self.config_path {
            config::save_value(path, "sort", sort.name()).with_context(|| t!("sort.not_saved"))?;
        }
        Ok(Some((
            t!("sort.set", label = sort.label()),
            MsgKind::Success,
        )))
    }

    fn set_lang(&mut self, code: &str) -> anyhow::Result<Option<Message>> {
        if !i18n::set(code) {
            bail!(t!("lang.unknown", code));
        }
        if let Some(path) = &self.config_path {
            config::save_value(path, "lang", i18n::current())
                .with_context(|| t!("lang.not_saved"))?;
        }
        Ok(Some((
            t!("lang.set", name = t!("language")),
            MsgKind::Success,
        )))
    }

    fn help(&self, name: &str) -> anyhow::Result<Option<Message>> {
        if let Some(body) = self.aliases.get(name) {
            let label = format!("alias {name}");
            return Ok(Some((t!("pair", label, value = body), MsgKind::Info)));
        }
        let help = find_help(name).with_context(|| t!("error.unknown_command", name))?;
        Ok(Some((
            t!("pair", label = help.usage(), value = help.summary()),
            MsgKind::Info,
        )))
    }

    fn find_app_by_id(&self, id: i64) -> Option<&AppEntry> {
        self.apps.iter().find(|a| a.id == id)
    }

    fn app_name(&self, id: i64) -> Option<String> {
        self.find_app_by_id(id).map(|a| a.name.clone())
    }

    /// Case-insensitive lookup by name.
    pub fn find_app(&self, name: &str) -> Option<&AppEntry> {
        let name = name.to_lowercase();
        self.apps.iter().find(|a| a.name.to_lowercase() == name)
    }

    fn app_named(&self, name: &str) -> anyhow::Result<&AppEntry> {
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

    pub fn selected_category(&self) -> Option<&Category> {
        self.cat_state
            .selected()
            .and_then(|i| self.categories.get(i))
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
    fn select_app(&mut self, id: i64) {
        let Some(category_id) = self.apps.iter().find(|a| a.id == id).map(|a| a.category_id) else {
            return;
        };
        let cat_index = self.categories.iter().position(|c| c.id == category_id);
        self.cat_state.select(cat_index);
        let app_index = self.visible_apps().iter().position(|a| a.id == id);
        self.app_state.select(app_index);
        self.focus = Focus::Apps;
    }

    fn move_selection(&mut self, forward: bool) {
        match self.screen {
            Screen::Rewards => {
                let next = step(self.reward_state.selected(), self.rewards.len(), forward);
                self.reward_state.select(next);
                return;
            }
            Screen::Stats => {
                let next = step(
                    self.stats_state.selected(),
                    self.stats.sessions.len(),
                    forward,
                );
                self.stats_state.select(next);
                return;
            }
            Screen::Storage if self.folders.is_some() => {
                let len = self.visible_entries().len();
                if let Some(folders) = &mut self.folders {
                    let next = step(folders.state.selected(), len, forward);
                    folders.state.select(next);
                }
                return;
            }
            Screen::Storage => {
                let len = self.visible_programs().len();
                let next = step(self.storage_state.selected(), len, forward);
                self.storage_state.select(next);
                return;
            }
            Screen::Optimize => {
                let (state, len) = if self.gaming_focus {
                    (&mut self.gaming_state, self.gaming.len())
                } else {
                    (&mut self.bench_state, Bench::ALL.len())
                };
                state.select(step(state.selected(), len, forward));
                return;
            }
            Screen::Dashboard | Screen::Help => {}
        }
        match self.focus {
            Focus::Categories => {
                let next = step(self.cat_state.selected(), self.categories.len(), forward);
                if next != self.cat_state.selected() {
                    self.cat_state.select(next);
                    self.reset_app_selection();
                }
            }
            Focus::Apps => {
                let next = step(
                    self.app_state.selected(),
                    self.visible_apps().len(),
                    forward,
                );
                self.app_state.select(next);
            }
        }
    }

    fn reset_storage_selection(&mut self) {
        let selected = (!self.visible_programs().is_empty()).then_some(0);
        self.storage_state = TableState::default().with_selected(selected);
    }

    fn reset_app_selection(&mut self) {
        let selected = (!self.visible_apps().is_empty()).then_some(0);
        self.app_state = TableState::default().with_selected(selected);
    }
}

fn created_note(created: bool) -> String {
    if created {
        t!("new_category")
    } else {
        String::new()
    }
}

/// Biggest first (or smallest, if `ascending`), unknown sizes always last.
fn by_size(a: Option<u64>, b: Option<u64>, ascending: bool) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) if ascending => x.cmp(&y),
        (Some(x), Some(y)) => y.cmp(&x),
        (a, b) => b.is_some().cmp(&a.is_some()),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    /// Types `:` + `text` + Enter.
    fn run(app: &mut App, text: &str) {
        press(app, KeyCode::Char(':'));
        text.chars().for_each(|c| press(app, KeyCode::Char(c)));
        press(app, KeyCode::Enter);
    }

    fn message_kind(app: &App) -> Option<MsgKind> {
        app.message.as_ref().map(|(_, kind)| *kind)
    }

    #[test]
    fn step_wraps_both_ways() {
        assert_eq!(step(Some(2), 3, true), Some(0));
        assert_eq!(step(Some(0), 3, false), Some(2));
        assert_eq!(step(None, 3, true), Some(0));
        assert_eq!(step(Some(0), 0, true), None);
    }

    #[test]
    fn clamp_keeps_selection_in_range() {
        assert_eq!(clamp(None, 3), Some(0));
        assert_eq!(clamp(Some(5), 3), Some(2));
        assert_eq!(clamp(Some(1), 0), None);
    }

    #[test]
    fn loads_from_database() {
        let app = App::with_defaults();
        assert_eq!(app.selected_category().unwrap().name, "Jeux");
        assert_eq!(app.selected_app().unwrap().name, "Steam");
    }

    #[test]
    fn sort_orders_apps_and_keeps_selection() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char('k')); // "Outils"
        press(&mut app, KeyCode::Char('l'));
        let names = |app: &App| -> Vec<String> {
            app.visible_apps().iter().map(|a| a.name.clone()).collect()
        };
        assert_eq!(names(&app), ["Bloc-notes", "Calculatrice", "Explorateur"]);

        let set_xp = |app: &mut App, name: &str, xp: u32| {
            let id = app.find_app(name).unwrap().id;
            app.db.set_app_xp(id, xp).unwrap();
            app.reload().unwrap();
        };
        set_xp(&mut app, "Explorateur", 100);
        set_xp(&mut app, "Calculatrice", 50);
        run(&mut app, "sort xp");
        assert_eq!(names(&app), ["Explorateur", "Calculatrice", "Bloc-notes"]);
        assert_eq!(app.selected_app().unwrap().name, "Bloc-notes"); // followed it
        set_xp(&mut app, "Bloc-notes", 500); // reload keeps the order
        assert_eq!(names(&app)[0], "Bloc-notes");

        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.sort, AppSort::Recent);
        run(&mut app, "sort name");
        assert_eq!(names(&app), ["Bloc-notes", "Calculatrice", "Explorateur"]);
        assert_eq!(
            app.init_sort(Some("size")).unwrap(),
            "config.toml : tri inconnu : size"
        );
    }

    #[test]
    fn storage_sorts_filters_and_confirms_uninstall() {
        let mut app = App::with_defaults();
        let program = |name: &str, size: Option<u64>, drive: char| Program {
            name: name.into(),
            publisher: None,
            size,
            drive: Some(drive),
            location: None,
            uninstall: "x.exe".into(),
        };
        let disk = |letter| Disk {
            letter,
            total: 100,
            free: 50,
        };
        app.on_storage_scanned(
            vec![disk('C'), disk('D')],
            vec![
                program("Alpha", Some(10), 'C'),
                program("Beta", None, 'C'),
                program("Gamma", Some(30), 'D'),
                program("Zeta", Some(20), 'C'),
            ],
        );
        let names = |app: &App| -> Vec<String> {
            app.visible_programs()
                .iter()
                .map(|p| p.name.clone())
                .collect()
        };
        press(&mut app, KeyCode::Char('4'));
        assert_eq!(app.screen, Screen::Storage);
        assert_eq!(names(&app), ["Gamma", "Zeta", "Alpha", "Beta"]);
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(names(&app), ["Alpha", "Zeta", "Gamma", "Beta"]); // unknown stays last
        press(&mut app, KeyCode::Tab);
        assert_eq!(names(&app), ["Alpha", "Zeta", "Beta"]); // C:
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.storage_disk, None); // D:, then every disk again
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.storage_disk, Some('D'));

        press(&mut app, KeyCode::Char('d'));
        let Mode::Popup(Popup::Confirm { command, .. }) = &app.mode else {
            panic!("expected a confirmation, got {:?}", app.mode);
        };
        assert_eq!(
            *command,
            Command::Uninstall {
                program: "Gamma".into(),
                confirmed: true
            }
        );
        press(&mut app, KeyCode::Esc); // never run a real uninstaller in tests
        run(&mut app, "uninstall nope");
        assert_eq!(message_kind(&app), Some(MsgKind::Error));
        press(&mut app, KeyCode::Char('0'));
        assert_eq!(app.screen, Screen::Help);
    }

    #[test]
    fn changing_category_resets_app_selection() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char('k')); // wraps to "Outils"
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected_app().unwrap().name, "Calculatrice");

        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.selected_category().unwrap().name, "Dev");
        assert_eq!(app.selected_app().unwrap().name, "Windows Terminal");
    }

    #[test]
    fn enter_on_categories_focuses_apps() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.focus, Focus::Apps);
    }

    #[test]
    fn colon_opens_and_esc_cancels() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char(':'));
        assert_eq!(app.mode, Mode::Command);
        press(&mut app, KeyCode::Char('q')); // typed, not quit
        assert!(!app.should_quit);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.command_line.input.is_empty());
    }

    #[test]
    fn add_creates_category_and_selects_app() {
        let mut app = App::with_defaults();
        run(
            &mut app,
            r#"add "Hollow Knight" steam://rungameid/367520 Metroidvania"#,
        );
        assert_eq!(
            message_kind(&app),
            Some(MsgKind::Success),
            "{:?}",
            app.message
        );
        assert_eq!(app.selected_category().unwrap().name, "Metroidvania");
        assert_eq!(app.selected_app().unwrap().name, "Hollow Knight");
        assert_eq!(app.focus, Focus::Apps);

        run(&mut app, r#"add "hollow knight" x.exe"#); // same name, other case
        assert_eq!(message_kind(&app), Some(MsgKind::Error));
    }

    #[test]
    fn add_without_category_uses_selected_one() {
        let mut app = App::with_defaults();
        run(&mut app, "add Paint mspaint.exe");
        let paint = app.selected_app().unwrap();
        assert_eq!(app.selected_category().unwrap().name, "Jeux");
        assert_eq!(paint.watch_exe.as_deref(), Some("mspaint.exe"));
    }

    #[test]
    fn move_follows_the_app() {
        let mut app = App::with_defaults();
        run(&mut app, "mv bloc-notes dev");
        assert_eq!(
            app.message.as_ref().unwrap().0,
            "Déplacé : Bloc-notes → Dev"
        );
        assert_eq!(app.selected_category().unwrap().name, "Dev");
        assert_eq!(app.selected_app().unwrap().name, "Bloc-notes");
    }

    #[test]
    fn errors_are_reported_not_fatal() {
        let mut app = App::with_defaults();
        for line in ["launch Inexistant", "move Inexistant Dev", "fly", "add x"] {
            run(&mut app, line);
            assert_eq!(message_kind(&app), Some(MsgKind::Error), "{line}");
        }
    }

    #[test]
    fn help_shows_screen_or_usage() {
        let mut app = App::with_defaults();
        run(&mut app, "help add");
        assert_eq!(message_kind(&app), Some(MsgKind::Info));
        run(&mut app, "help");
        assert_eq!(app.screen, Screen::Help);
    }

    #[test]
    fn history_recalls_previous_command() {
        let mut app = App::with_defaults();
        run(&mut app, "help add");
        press(&mut app, KeyCode::Char(':'));
        press(&mut app, KeyCode::Up);
        assert_eq!(app.command_line.input.text(), "help add");
    }

    fn type_text(app: &mut App, text: &str) {
        text.chars().for_each(|c| press(app, KeyCode::Char(c)));
    }

    fn form(app: &App) -> &Form {
        match &app.mode {
            Mode::Popup(Popup::Form(form)) => form,
            other => panic!("expected a form, got {other:?}"),
        }
    }

    #[test]
    fn add_form_adds_app() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char('a')); // picker first, empty in tests
        type_text(&mut app, "Paint");
        press(&mut app, KeyCode::Enter); // no match: by hand, the search becomes the name
        assert_eq!(form(&app).fields[0].input.text(), "Paint");
        assert_eq!(form(&app).fields[2].input.text(), "Jeux"); // selected category
        assert_eq!(form(&app).focused, 1);

        type_text(&mut app, "mspaint.exe");
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Enter); // last field: submit

        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(message_kind(&app), Some(MsgKind::Success));
        let paint = app.selected_app().unwrap();
        assert_eq!(paint.name, "Paint");
        assert_eq!(paint.watch_exe.as_deref(), Some("mspaint.exe"));
    }

    #[test]
    fn form_keeps_errors_inside() {
        let mut app = App::with_defaults();
        run(&mut app, "add");
        press(&mut app, KeyCode::Tab); // skip the picker
        for _ in 0..4 {
            press(&mut app, KeyCode::Enter); // empty name: submit fails on the last field
        }
        assert_eq!(form(&app).error.as_deref(), Some("Nom : champ requis"));
        assert_eq!(form(&app).focused, 0);

        type_text(&mut app, "steam"); // already exists, ignoring case
        assert_eq!(form(&app).error, None); // typing clears the error
        press(&mut app, KeyCode::Tab);
        type_text(&mut app, "x.exe");
        press(&mut app, KeyCode::BackTab);
        press(&mut app, KeyCode::BackTab); // wraps to the last field
        press(&mut app, KeyCode::Enter);
        assert_eq!(form(&app).error.as_deref(), Some("« steam » existe déjà"));

        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.apps.len(), 5);
    }

    #[test]
    fn picker_fills_the_form_from_an_installed_app() {
        let mut app = App::with_defaults();
        let shortcut = |name: &str, target: &str, exe: Option<&str>| Shortcut {
            name: name.into(),
            target: target.into(),
            watch_exe: exe.map(Into::into),
        };
        app.shortcuts = vec![
            shortcut("Hades", r"C:\Start\Hades.lnk", Some("Hades.exe")),
            shortcut("Hollow Knight", "steam://rungameid/367520", None),
            shortcut("Steam", "STEAM://open/main", None), // already added: hidden
        ];
        press(&mut app, KeyCode::Char('a'));
        let Mode::Popup(Popup::Picker(picker)) = &app.mode else {
            panic!("{:?}", app.mode)
        };
        let names: Vec<_> = app
            .picker_matches(picker)
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, ["Hades", "Hollow Knight"]);

        type_text(&mut app, "hk");
        press(&mut app, KeyCode::Enter);
        let fields: Vec<_> = form(&app).fields.iter().map(|f| f.input.text()).collect();
        assert_eq!(
            fields,
            ["Hollow Knight", "steam://rungameid/367520", "Jeux", ""]
        );
        assert_eq!(form(&app).focused, 2); // only the category is left to check

        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Enter); // Hades, the first one
        assert_eq!(form(&app).fields[3].input.text(), "Hades.exe");
    }

    #[test]
    fn move_form_moves_selected_app() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char('m'));
        assert_eq!(form(&app).fields[0].input.text(), "Jeux");
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "Outils");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.selected_category().unwrap().name, "Outils");
        assert_eq!(app.selected_app().unwrap().name, "Steam");
    }

    #[test]
    fn delete_app_asks_first() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('d'));
        assert!(matches!(app.mode, Mode::Popup(Popup::Confirm { .. })));
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.apps.len(), 5);

        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.apps.len(), 4);
        assert!(app.find_app("Steam").is_none());
        assert_eq!(app.selected_app().map(|a| a.name.as_str()), None); // Jeux is empty now
    }

    #[test]
    fn rm_from_command_line_also_asks() {
        let mut app = App::with_defaults();
        run(&mut app, "rm bloc-notes");
        assert!(matches!(app.mode, Mode::Popup(Popup::Confirm { .. })));
        press(&mut app, KeyCode::Char('o'));
        assert!(app.find_app("Bloc-notes").is_none());
    }

    #[test]
    fn only_empty_categories_can_be_removed() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char('d')); // "Jeux" holds Steam
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(message_kind(&app), Some(MsgKind::Error));

        run(&mut app, "mv steam Dev");
        run(&mut app, "rmcat jeux");
        press(&mut app, KeyCode::Enter);
        assert_eq!(message_kind(&app), Some(MsgKind::Success));
        assert!(app.categories.iter().all(|c| c.name != "Jeux"));
    }

    fn steam_id(app: &App) -> i64 {
        app.find_app("Steam").unwrap().id
    }

    #[test]
    fn session_is_recorded_from_start_to_end() {
        let mut app = App::with_defaults();
        let steam = steam_id(&app);
        app.on_session_start(steam);
        app.on_session_start(steam); // duplicate start: ignored
        assert_eq!(app.active_sessions.len(), 1);
        assert_eq!(message_kind(&app), Some(MsgKind::Info));

        app.on_session_end(steam, 42 * 60);
        assert!(app.active_sessions.is_empty());
        // 42 XP for the minutes + 5 for a one-day streak (today).
        assert_eq!(
            app.message.as_ref().unwrap().0,
            "Session terminée : Steam (42 min, +47 XP)"
        );
        let entry = app.find_app("Steam").unwrap();
        assert_eq!(entry.total_secs, 42 * 60);
        assert_eq!(entry.total_xp, 47);
        assert_eq!(app.profile.total_xp, 47);
        // No level reached, but the first session ever unlocks a reward.
        assert_eq!(popup_title(&app), Some("Premiers pas".into()));
        assert!(entry.last_played.is_some());
    }

    #[test]
    fn short_session_is_not_recorded() {
        let mut app = App::with_defaults();
        let steam = steam_id(&app);
        app.on_session_start(steam);
        app.on_session_end(steam, 5);
        assert_eq!(message_kind(&app), Some(MsgKind::Info));
        assert_eq!(app.find_app("Steam").unwrap().total_secs, 0);
    }

    #[test]
    fn unknown_or_inactive_sessions_are_ignored() {
        let mut app = App::with_defaults();
        app.on_session_start(9_999);
        app.on_session_end(steam_id(&app), 600);
        assert!(app.active_sessions.is_empty());
        assert_eq!(app.message, None);
    }

    #[test]
    fn removing_an_app_mid_session_is_harmless() {
        let mut app = App::with_defaults();
        let steam = steam_id(&app);
        app.on_session_start(steam);
        run(&mut app, "rm steam");
        press(&mut app, KeyCode::Enter);
        app.on_session_end(steam, 600); // the tracker notices afterwards
        assert!(app.active_sessions.is_empty());
        assert!(app.find_app("Steam").is_none());
    }

    #[test]
    fn quitting_closes_running_sessions() {
        let mut app = App::with_defaults();
        app.on_session_start(steam_id(&app));
        app.end_all_sessions().unwrap();
        assert!(app.active_sessions.is_empty());
        assert!(app.db.close_orphan_sessions().unwrap().is_empty()); // nothing left open
    }

    #[test]
    fn orphan_sessions_earn_their_xp() {
        let mut app = App::with_defaults();
        let steam = steam_id(&app);
        let id = app.db.start_session(steam, unix_now() - 600).unwrap();
        app.db.checkpoint_session(id, 600).unwrap(); // then CmdBoard "crashed"

        app.close_orphan_sessions().unwrap();
        assert_eq!(app.find_app("Steam").unwrap().total_xp, 15); // 10 min + 5 streak
        assert_eq!(message_kind(&app), Some(MsgKind::Info));
        assert_eq!(popup_title(&app), Some("Premiers pas".into()));
    }

    #[test]
    fn long_session_levels_up_with_popup_and_animation() {
        let mut app = App::with_defaults();
        let steam = steam_id(&app);
        app.on_session_start(steam);
        app.on_session_end(steam, 3 * 3600); // 180 + 5 XP: Steam and profile reach level 2

        let Mode::Popup(Popup::LevelUp(level_up)) = &app.mode else {
            panic!("expected a level-up, got {:?}", app.mode);
        };
        assert_eq!(level_up.app, "Steam");
        assert_eq!(level_up.app_level, Some(2));
        assert_eq!(level_up.global_level, Some(2));
        assert_eq!(level_up.gained, 185);

        // The bars start from the old value and fill up over a few ticks.
        let entry = app.find_app("Steam").unwrap().clone();
        assert_eq!(app.shown_app_xp(&entry), (1, 0));
        for _ in 0..XP_ANIM_FRAMES {
            app.on_tick();
        }
        assert_eq!(app.shown_app_xp(&entry), (2, 85));
        assert_eq!(app.shown_profile_xp(), (2, 85));
        assert!(app.xp_anims.is_empty() && app.profile_anim.is_none());

        // Then the rewards: first session ever, and 3 h in a row on Steam (plus
        // "Noctambule" when the test runs at night).
        press(&mut app, KeyCode::Enter);
        let mut rewards = Vec::new();
        while let Mode::Popup(Popup::RewardUnlocked(reward)) = &app.mode {
            rewards.push((reward.name.clone(), reward.app.clone()));
            press(&mut app, KeyCode::Esc);
        }
        assert_eq!(rewards[0], ("Premiers pas".to_string(), None));
        assert!(rewards.contains(&("Marathon".to_string(), Some("Steam".to_string()))));
        assert_eq!(app.mode, Mode::Normal);
    }

    /// Name of the reward shown in the current popup, if any.
    fn popup_title(app: &App) -> Option<String> {
        match &app.mode {
            Mode::Popup(Popup::RewardUnlocked(reward)) => Some(reward.name.clone()),
            _ => None,
        }
    }

    /// Plays a whole session on `app_id` and closes every popup it opens.
    fn play(app: &mut App, app_id: i64, secs: u64) {
        app.on_session_start(app_id);
        app.on_session_end(app_id, secs);
        while matches!(app.mode, Mode::Popup(_)) {
            press(app, KeyCode::Esc);
        }
    }

    fn unlocked(app: &App, name: &str) -> Vec<Option<String>> {
        let reward = app.rewards.iter().find(|r| r.name == name).unwrap();
        reward.unlocks.iter().map(|u| u.app.clone()).collect()
    }

    #[test]
    fn rewards_unlock_once_globally_and_once_per_app() {
        let mut app = App::with_defaults();
        let steam = steam_id(&app);
        let notepad = app.find_app("Bloc-notes").unwrap().id;
        play(&mut app, steam, 3 * 3600);
        play(&mut app, steam, 3 * 3600);
        play(&mut app, notepad, 3 * 3600);

        assert_eq!(unlocked(&app, "Premiers pas"), [None]);
        assert_eq!(
            unlocked(&app, "Marathon"),
            [Some("Steam".to_string()), Some("Bloc-notes".to_string())]
        );
        assert!(unlocked(&app, "Centurion").is_empty());
        assert_eq!(app.find_app("Steam").unwrap().rewards, 1);
        assert!(matches!(
            &app.activity[0],
            Activity::Reward { name, .. } if name == "Marathon (Bloc-notes)"
        ));
    }

    #[test]
    fn broken_rule_is_reported_and_others_still_unlock() {
        let mut app = App::with_defaults();
        app.db
            .execute_for_tests("UPDATE rewards SET rule = 'hours >= 1' WHERE code = 'marathon'");
        let steam = steam_id(&app);
        app.on_session_start(steam);
        app.on_session_end(steam, 3 * 3600);
        let (text, kind) = app.message.clone().unwrap();
        assert_eq!(kind, MsgKind::Error);
        assert!(
            text.contains("règle « marathon » : variable inconnue : hours"),
            "{text}"
        );
        play(&mut app, steam, 0); // closes the popups
        assert_eq!(unlocked(&app, "Premiers pas"), [None]);
    }

    #[test]
    fn rewards_screen_has_its_own_selection() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char('3'));
        assert_eq!(app.reward_state.selected(), Some(0));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.reward_state.selected(), Some(1));
        press(&mut app, KeyCode::Char('k'));
        press(&mut app, KeyCode::Char('k')); // wraps
        assert_eq!(app.reward_state.selected(), Some(app.rewards.len() - 1));
        assert_eq!(app.selected_app().unwrap().name, "Steam"); // dashboard untouched
    }

    #[test]
    fn search_selects_an_app_from_any_category() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char('/'));
        assert_eq!(app.mode, Mode::Search);
        assert_eq!(app.visible_apps().len(), app.apps.len()); // empty query: everything
        type_text(&mut app, "wterm");
        assert_eq!(app.selected_app().unwrap().name, "Windows Terminal");
        press(&mut app, KeyCode::Enter);

        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.selected_category().unwrap().name, "Dev");
        assert_eq!(app.selected_app().unwrap().name, "Windows Terminal");
        assert_eq!(app.focus, Focus::Apps);
    }

    #[test]
    fn cancelled_search_restores_selection() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "bloc");
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.focus, Focus::Categories);
        assert_eq!(app.selected_app().unwrap().name, "Steam");

        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "zzz");
        assert!(app.selected_app().is_none());
        press(&mut app, KeyCode::Enter); // no match: like Esc
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.selected_app().unwrap().name, "Steam");
    }

    fn with_aliases(toml: &str) -> App {
        let mut app = App::with_defaults();
        app.aliases = Aliases::parse(toml).unwrap().0;
        app
    }

    #[test]
    fn alias_runs_its_commands_in_order() {
        let mut app = with_aliases("[alias]\nboost = \"sort xp; stats $1\"");
        run(&mut app, "boost bloc-notes");
        assert_eq!(app.sort, AppSort::Xp);
        assert_eq!(app.screen, Screen::Stats);
        assert_eq!(app.stats_app, Some(app.find_app("Bloc-notes").unwrap().id));

        run(&mut app, "boost");
        assert_eq!(
            app.message.as_ref().unwrap().0,
            "alias : argument $1 manquant"
        );
        run(&mut app, "help boost");
        assert_eq!(
            app.message.as_ref().unwrap().0,
            "alias boost : sort xp; stats $1"
        );
    }

    #[test]
    fn group_command_saves_a_launch_alias() {
        let dir = std::env::temp_dir().join(format!("cmdboard-app-group-{}", std::process::id()));
        let mut app = App::with_defaults();
        app.init_theme(&dir, None);
        run(&mut app, "group Outils bloc-notes, calculatrice");
        assert_eq!(message_kind(&app), Some(MsgKind::Success));
        assert_eq!(
            app.aliases.get("outils"),
            Some("launch Bloc-notes; launch Calculatrice")
        );
        assert_eq!(Aliases::load(&dir.join("commands.toml")).0, app.aliases);

        run(&mut app, "group outils nope");
        assert_eq!(message_kind(&app), Some(MsgKind::Error));
        assert_eq!(
            app.aliases.get("outils"),
            Some("launch Bloc-notes; launch Calculatrice")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn alias_stops_at_first_error_or_confirmation() {
        let mut app =
            with_aliases("[alias]\nbad = \"stats nope; sort xp\"\nclean = \"rm steam; sort xp\"");
        run(&mut app, "bad");
        assert_eq!(message_kind(&app), Some(MsgKind::Error));
        assert!(
            app.message
                .as_ref()
                .unwrap()
                .0
                .starts_with("stats nope : app inconnue")
        );
        assert_eq!(app.sort, AppSort::Name);

        run(&mut app, "clean");
        assert!(matches!(app.mode, Mode::Popup(Popup::Confirm { .. })));
        assert_eq!(app.sort, AppSort::Name);
    }

    #[test]
    fn tab_completes_in_the_command_line() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char(':'));
        type_text(&mut app, "lau");
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.command_line.input.text(), "launch ");
        type_text(&mut app, "calc");
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.command_line.input.text(), "launch Calculatrice");
    }

    #[test]
    fn stats_command_filters_and_resets() {
        let mut app = App::with_defaults();
        let steam = steam_id(&app);
        play(&mut app, steam, 600);
        let notepad = app.find_app("Bloc-notes").unwrap().id;
        play(&mut app, notepad, 1200);

        run(&mut app, "stats steam");
        assert_eq!(app.screen, Screen::Stats);
        assert_eq!(app.stats.session_count, 1);
        assert_eq!(app.stats_state.selected(), Some(0));

        run(&mut app, "stats");
        assert_eq!(app.stats_app, None);
        assert_eq!(app.stats.session_count, 2);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.stats_state.selected(), Some(1));

        run(&mut app, "stats inconnue");
        assert_eq!(message_kind(&app), Some(MsgKind::Error));
    }

    #[test]
    fn theme_command_switches_and_keeps_current_on_error() {
        let mut app = App::with_defaults();
        run(&mut app, "theme");
        let (text, _) = app.message.clone().unwrap();
        assert!(
            text.starts_with("Thèmes : catppuccin-frappe, catppuccin-latte"),
            "{text}"
        );
        assert!(text.ends_with("(actuel : terminal)"));

        run(&mut app, "theme Catppuccin-Latte");
        assert_eq!(app.message.as_ref().unwrap().0, "Thème : Catppuccin Latte");
        assert_eq!(app.theme_name, "catppuccin-latte");

        run(&mut app, "theme nope");
        assert_eq!(message_kind(&app), Some(MsgKind::Error));
        assert_eq!(app.theme.name, "Catppuccin Latte");
    }

    /// Stays in French: the language is global and tests run in parallel.
    #[test]
    fn lang_command_lists_and_rejects_unknown() {
        let mut app = App::with_defaults();
        run(&mut app, "lang");
        assert_eq!(
            app.message.as_ref().unwrap().0,
            "Langues : en, fr, pt (actuelle : fr)"
        );
        run(&mut app, "lang FR");
        assert_eq!(app.message.as_ref().unwrap().0, "Langue : Français");
        run(&mut app, "lang xx");
        assert_eq!(
            app.message.as_ref().unwrap().0,
            "langue inconnue : xx (:lang pour la liste)"
        );
    }

    #[test]
    fn configured_theme_is_loaded_and_saved() {
        let dir = std::env::temp_dir().join(format!("cmdboard-app-theme-{}", std::process::id()));
        let mut app = App::with_defaults();
        assert_eq!(app.init_theme(&dir, Some("catppuccin-frappe".into())), None);
        assert_eq!(app.theme.name, "Catppuccin Frappé");

        run(&mut app, "theme catppuccin-macchiato");
        let (config, _) = config::Config::load(&dir.join("config.toml"));
        assert_eq!(config.theme.as_deref(), Some("catppuccin-macchiato"));

        let mut app = App::with_defaults();
        let warning = app.init_theme(&dir, Some("gone".into()));
        assert!(warning.unwrap().contains("thème inconnu"));
        assert_eq!(app.theme_name, theme::default_name()); // fell back
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn level_up_waits_while_typing() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char(':'));
        let id = steam_id(&app);
        app.db.set_app_xp(id, 100).unwrap();
        app.reload().unwrap();
        app.on_xp_changed(id, 0, 0, 100);
        assert_eq!(app.mode, Mode::Command); // not interrupted
        press(&mut app, KeyCode::Esc);
        app.on_tick();
        assert!(matches!(app.mode, Mode::Popup(Popup::LevelUp(_))));
    }

    #[test]
    fn tracker_receives_watch_list_on_reload() {
        let mut app = App::with_defaults();
        let (tx, rx) = std::sync::mpsc::channel();
        app.attach_tracker(tx);
        let list = rx.try_recv().unwrap();
        assert_eq!(list.len(), 4); // Explorateur has no watch_exe
        assert!(list.contains(&Watched {
            app_id: steam_id(&app),
            exe: "steam.exe".into()
        }));

        run(&mut app, "add Paint mspaint.exe");
        let list = rx.try_iter().last().unwrap();
        assert!(list.iter().any(|w| w.exe == "mspaint.exe"));
    }

    #[test]
    fn update_outcomes_reach_status_bar_and_message() {
        use update::{Action, Outcome};
        let mut app = App::with_defaults();
        run(&mut app, "update"); // no event channel in tests
        assert_eq!(message_kind(&app), Some(MsgKind::Error));

        app.message = None;
        app.on_update_finished(Action::Check, Err("offline".into()));
        assert_eq!(app.message, None); // the passive check stays silent
        let available = Outcome::Available {
            version: "0.2.0".into(),
            managed: true,
        };
        app.on_update_finished(Action::Check, Ok(available.clone()));
        assert_eq!(
            (app.update_available.as_deref(), &app.message),
            (Some("0.2.0"), &None)
        );

        app.on_update_finished(Action::Install, Ok(available));
        assert!(
            app.message
                .as_ref()
                .unwrap()
                .0
                .contains("winget upgrade CmdBoard")
        );
        app.on_update_finished(
            Action::Install,
            Ok(Outcome::Installed {
                version: "0.2.0".into(),
            }),
        );
        assert_eq!(message_kind(&app), Some(MsgKind::Success));
        assert_eq!(app.update_available, None);
    }

    #[test]
    fn optimize_screen_keys() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char('5'));
        assert_eq!(app.screen, Screen::Optimize);
        assert!(app.system.is_some() && app.gaming.len() == Gaming::ALL.len());
        press(&mut app, KeyCode::Char('n'));
        assert!(app.bench_heavy);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.bench_state.selected(), Some(1));
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('k'));
        assert!(app.gaming_focus);
        assert_eq!(app.gaming_state.selected(), Some(Gaming::ALL.len() - 1));
        assert_eq!(app.bench_state.selected(), Some(1));
        // No event channel in tests: the benchmark thread does not start.
        app.execute(Command::Bench(Bench::CpuSingle));
        assert_eq!(app.bench_running, None);
        app.bench_running = Some(Bench::Disk);
        app.execute(Command::Bench(Bench::CpuSingle));
        assert_eq!(message_kind(&app), Some(MsgKind::Error));
    }

    #[test]
    fn q_quits() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_c_quits_from_any_mode() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char(':'));
        app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    #[test]
    fn screens_and_sorts_have_distinct_labels() {
        use std::collections::HashSet;
        let screens = [
            Screen::Dashboard,
            Screen::Stats,
            Screen::Rewards,
            Screen::Storage,
            Screen::Optimize,
            Screen::Help,
        ];
        let titles: HashSet<String> = screens.into_iter().map(Screen::title).collect();
        assert_eq!(titles.len(), screens.len());
        let labels: HashSet<String> = AppSort::ALL.into_iter().map(AppSort::label).collect();
        assert_eq!(labels.len(), AppSort::ALL.len());
    }

    #[test]
    fn sort_by_time_from_config_and_listed() {
        let mut app = App::with_defaults();
        let notepad = app.find_app("Bloc-notes").unwrap().id;
        play(&mut app, notepad, 600);
        assert_eq!(app.init_sort(None), None);
        assert_eq!(app.init_sort(Some("TIME")), None);
        assert_eq!(app.sort, AppSort::Time);
        assert_eq!(app.apps[0].name, "Bloc-notes");

        run(&mut app, "sort");
        let (text, kind) = app.message.clone().unwrap();
        assert_eq!(kind, MsgKind::Info);
        assert!(text.contains("name, xp, recent, time"), "{text}");
    }

    #[test]
    fn sort_is_saved_to_config() {
        let dir = std::env::temp_dir().join(format!("cmdboard-app-sort-{}", std::process::id()));
        let mut app = App::with_defaults();
        app.init_theme(&dir, None);
        run(&mut app, "sort recent");
        let (config, _) = config::Config::load(&dir.join("config.toml"));
        assert_eq!(config.sort.as_deref(), Some("recent"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn events_are_routed_to_their_handler() {
        use crate::launcher::folders::Entry;
        let mut app = App::with_defaults();
        let steam = steam_id(&app);
        app.handle(AppEvent::Key(KeyEvent::new(
            KeyCode::Char('2'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.screen, Screen::Stats);
        app.handle(AppEvent::Tick);
        assert_eq!(app.frame_count, 1);

        app.handle(AppEvent::SessionStarted { app_id: steam });
        assert_eq!(app.active_sessions.len(), 1);
        app.handle(AppEvent::SessionEnded {
            app_id: steam,
            secs: 5,
        });
        assert!(app.active_sessions.is_empty());

        app.handle(AppEvent::UpdateFinished {
            action: update::Action::Check,
            result: Ok(update::Outcome::Available {
                version: "9.9.9".into(),
                managed: false,
            }),
        });
        assert_eq!(app.update_available.as_deref(), Some("9.9.9"));

        let disk = Disk {
            letter: 'C',
            total: 100,
            free: 40,
        };
        app.handle(AppEvent::StorageScanned {
            disks: vec![disk],
            programs: Vec::new(),
        });
        assert_eq!(app.disks.len(), 1);

        app.handle(AppEvent::BenchFinished {
            bench: Bench::Memory,
            heavy: true,
            result: Err("x".into()),
        });
        assert_eq!(app.bench_results[&Bench::Memory], (true, Err("x".into())));

        // The drives list, then events for it.
        app.toggle_folders();
        let drive = PathBuf::from(r"C:\");
        assert_eq!(app.visible_entries()[0].size, Some(60));
        app.handle(AppEvent::FolderListed {
            dir: drive.clone(),
            entries: Vec::<Entry>::new(),
        }); // not the shown folder: ignored
        app.handle(AppEvent::FolderProgress {
            path: drive.clone(),
            percent: 40,
        });
        assert_eq!(app.folders.as_ref().unwrap().progress[&drive], 40);
        app.handle(AppEvent::FolderSized {
            path: drive.clone(),
            size: 70,
        });
        assert_eq!(app.visible_entries()[0].size, Some(70));
        app.handle(AppEvent::Trashed {
            path: drive,
            result: Err("refusé".into()),
        });
        assert_eq!(
            app.message.as_ref().unwrap(),
            &("refusé".into(), MsgKind::Error)
        );

        let mut picker = Picker::new("Jeux");
        picker.selected = 3;
        app.mode = Mode::Popup(Popup::Picker(picker));
        app.scan_running = true;
        app.handle(AppEvent::ShortcutsScanned(vec![Shortcut {
            name: "Hades".into(),
            target: "hades.exe".into(),
            watch_exe: None,
        }]));
        assert!(!app.scan_running);
        assert_eq!(app.shortcuts.len(), 1);
        assert!(matches!(&app.mode, Mode::Popup(Popup::Picker(p)) if p.selected == 0));
    }

    #[test]
    fn install_outcomes_show_a_message() {
        use update::{Action, Outcome};
        let mut app = App::with_defaults();
        app.update_available = Some("9.9.9".into());
        app.on_update_finished(Action::Install, Ok(Outcome::UpToDate));
        assert_eq!(app.update_available, None);
        assert_eq!(message_kind(&app), Some(MsgKind::Info));
        app.on_update_finished(Action::Install, Err("offline".into()));
        assert_eq!(message_kind(&app), Some(MsgKind::Error));

        app.update_running = true;
        run(&mut app, "update");
        assert_eq!(message_kind(&app), Some(MsgKind::Error));
        assert!(app.update_running);
    }

    #[test]
    fn running_sessions_checkpoint_every_minute() {
        let mut app = App::with_defaults();
        let steam = steam_id(&app);
        app.on_session_start(steam);
        let long_ago = Instant::now()
            .checked_sub(Duration::from_secs(120))
            .unwrap();
        let session = app.active_sessions.get_mut(&steam).unwrap();
        session.started = long_ago;
        session.last_checkpoint = long_ago;
        app.on_tick();
        assert!(app.active_sessions[&steam].last_checkpoint > long_ago);

        // A crash now: the checkpointed time is recovered at the next start.
        app.active_sessions.clear();
        app.close_orphan_sessions().unwrap();
        assert_eq!(app.find_app("Steam").unwrap().total_secs, 120);
    }

    #[test]
    fn orphan_recovery_reports_nothing_or_broken_rules() {
        let mut app = App::with_defaults();
        app.close_orphan_sessions().unwrap();
        assert_eq!(app.message, None); // nothing was left open

        app.db
            .execute_for_tests("UPDATE rewards SET rule = 'hours >= 1' WHERE code = 'marathon'");
        let id = app
            .db
            .start_session(steam_id(&app), unix_now() - 600)
            .unwrap();
        app.db.checkpoint_session(id, 600).unwrap();
        app.close_orphan_sessions().unwrap();
        let (text, kind) = app.message.clone().unwrap();
        assert_eq!(kind, MsgKind::Error);
        assert!(text.contains("marathon"), "{text}");
    }

    #[test]
    fn xp_display_without_animation_and_unknown_app() {
        let mut app = App::with_defaults();
        assert_eq!(app.shown_profile_xp(), (app.profile.level, app.profile.xp));
        app.on_xp_changed(9_999, 0, 0, 500);
        assert!(app.xp_anims.is_empty() && app.profile_anim.is_none());
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn search_keys_move_and_backspace_leaves() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.app_state.selected(), Some(1));
        press(&mut app, KeyCode::BackTab);
        press(&mut app, KeyCode::Up); // wraps
        assert_eq!(app.app_state.selected(), Some(app.apps.len() - 1));
        press(&mut app, KeyCode::Backspace); // empty query: leaves the search
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.selected_app().unwrap().name, "Steam");
    }

    #[test]
    fn command_line_keys() {
        let mut app = App::with_defaults();
        run(&mut app, "sort");
        press(&mut app, KeyCode::Char(':'));
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Down); // past the newest: empty again
        assert!(app.command_line.input.is_empty());
        press(&mut app, KeyCode::Backspace); // empty line: leaves
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn keys_map_to_commands_per_screen() {
        let mut app = App::with_defaults();
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        let cases = [
            (KeyCode::Char('1'), Some(Command::Show(Screen::Dashboard))),
            (KeyCode::Char('2'), Some(Command::Show(Screen::Stats))),
            (KeyCode::Char('z'), None),
        ];
        for (code, expected) in cases {
            assert_eq!(app.key_to_command(key(code)), expected, "{code:?}");
        }
        app.focus = Focus::Apps;
        assert_eq!(
            app.key_to_command(key(KeyCode::Enter)),
            Some(Command::Launch {
                app: "Steam".into()
            })
        );

        app.screen = Screen::Stats;
        assert_eq!(
            app.key_to_command(key(KeyCode::Char('s'))),
            Some(Command::ToggleStatsPie)
        );
        assert_eq!(app.key_to_command(key(KeyCode::Char('a'))), None);
        app.screen = Screen::Storage;
        assert_eq!(app.key_to_command(key(KeyCode::Char('x'))), None);

        app.screen = Screen::Optimize;
        app.gaming = vec![(Gaming::GameMode, true)];
        assert_eq!(
            app.key_to_command(key(KeyCode::Enter)),
            Some(Command::Bench(Bench::ALL[0]))
        );
        assert_eq!(app.key_to_command(key(KeyCode::Char('x'))), None);
        app.gaming_focus = true;
        assert_eq!(
            app.key_to_command(key(KeyCode::Enter)),
            Some(Command::ToggleGaming(Gaming::GameMode))
        );
        assert_eq!(
            app.key_to_command(key(KeyCode::Char('o'))),
            Some(Command::OpenGamingPage(Gaming::GameMode))
        );
    }

    #[test]
    fn folder_browser_without_threads() {
        use crate::launcher::folders::Entry;
        let mut app = App::with_defaults();
        app.disks = vec![Disk {
            letter: 'C',
            total: 100,
            free: 40,
        }];
        app.programs = vec![Program {
            name: "Hades".into(),
            publisher: None,
            size: None,
            drive: Some('C'),
            location: Some(r"C:\Games\Hades".into()),
            uninstall: "x.exe".into(),
        }];
        let dir = PathBuf::from(r"C:\Games");
        let entry = |name: &str, is_dir: bool, size: Option<u64>| Entry {
            name: name.into(),
            path: dir.join(name),
            is_dir,
            size,
        };
        app.on_folder_listed(&dir, Vec::new()); // no browser open: ignored
        app.screen = Screen::Storage;
        app.open_folder(Some(dir.clone()), None);
        app.on_folder_listed(
            &dir,
            vec![
                entry("Hades", true, None),
                entry("notes.txt", false, Some(10)),
            ],
        );
        let names = |app: &App| -> Vec<String> {
            app.visible_entries()
                .iter()
                .map(|e| e.name.clone())
                .collect()
        };
        assert_eq!(names(&app), ["notes.txt", "Hades"]); // unmeasured last

        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(
            app.key_to_command(key(KeyCode::Char('d'))),
            Some(Command::Uninstall {
                program: "Hades".into(),
                confirmed: false
            })
        );
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(
            app.key_to_command(key(KeyCode::Delete)),
            Some(Command::Trash {
                path: dir.join("notes.txt"),
                confirmed: false
            })
        );
        assert_eq!(
            app.key_to_command(key(KeyCode::Char('s'))),
            Some(Command::ToggleStorageOrder)
        );
        assert_eq!(app.key_to_command(key(KeyCode::Char('x'))), None);

        app.on_folder_sized(dir.join("Hades"), 100);
        assert_eq!(names(&app), ["Hades", "notes.txt"]);
        assert_eq!(app.selected_entry().unwrap().name, "notes.txt"); // followed it

        app.execute(Command::Trash {
            path: dir.join("notes.txt"),
            confirmed: true,
        }); // no event channel in tests
        assert_eq!(message_kind(&app), Some(MsgKind::Error));
        app.on_trashed(&dir.join("notes.txt"), Ok(()));
        assert_eq!(message_kind(&app), Some(MsgKind::Success));
        assert_eq!(names(&app), ["Hades"]);
        assert!(app.folder_sizes.contains_key(&dir.join("Hades")));

        press(&mut app, KeyCode::Left); // C:\
        press(&mut app, KeyCode::Left); // the drives
        assert_eq!(app.folders.as_ref().unwrap().dir, None);
        assert_eq!(app.key_to_command(key(KeyCode::Char('d'))), None);
        press(&mut app, KeyCode::Char('f'));
        assert!(app.folders.is_none());
    }

    #[test]
    fn storage_selection_and_unplugged_drive() {
        let mut app = App::with_defaults();
        let program = |name: &str| Program {
            name: name.into(),
            publisher: None,
            size: Some(1),
            drive: Some('C'),
            location: None,
            uninstall: "x.exe".into(),
        };
        app.storage_disk = Some('D');
        app.on_storage_scanned(
            vec![Disk {
                letter: 'C',
                total: 100,
                free: 40,
            }],
            vec![program("Alpha"), program("Beta")],
        );
        assert_eq!(app.storage_disk, None); // D: is gone
        press(&mut app, KeyCode::Char('4'));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.storage_state.selected(), Some(1));
    }

    #[test]
    fn threads_report_through_the_event_channel() {
        let dir = std::env::temp_dir().join(format!("cmdboard-app-threads-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("a.bin"), [0u8; 300]).unwrap();

        let mut app = App::with_defaults();
        let (tx, rx) = std::sync::mpsc::channel();
        let config = config::Config {
            update_check: false,
            ..Default::default()
        };
        app.attach_events(tx, &config);
        assert!(app.storage_scanning && !app.update_running);
        app.show(Screen::Storage); // already scanning: no second thread

        app.open_folder(Some(dir.clone()), None);
        app.execute(Command::OpenForm(FormKind::AddApp)); // the picker scans the installed apps
        assert!(app.scan_running);
        app.execute(Command::OpenForm(FormKind::AddApp)); // already scanning: no second thread

        let wait = Duration::from_secs(60);
        while app.storage_scanning
            || app.scan_running
            || app.folder_sizes.get(&dir.join("sub")) != Some(&300)
        {
            app.handle(rx.recv_timeout(wait).unwrap());
        }
        assert_eq!(app.visible_entries()[0].name, "sub");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn popup_keys_cancel_or_wait() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char('a'));
        press(&mut app, KeyCode::Esc); // closes the picker
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(message_kind(&app), Some(MsgKind::Info));

        run(&mut app, "rm steam");
        press(&mut app, KeyCode::Char('x')); // neither yes nor no
        assert!(matches!(app.mode, Mode::Popup(Popup::Confirm { .. })));
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Categories);
    }

    #[test]
    fn clear_commands_ask_first() {
        let mut app = App::with_defaults();
        let steam = steam_id(&app);
        play(&mut app, steam, 600);
        run(&mut app, "clear sessions");
        press(&mut app, KeyCode::Enter);
        assert_eq!(message_kind(&app), Some(MsgKind::Success));
        assert!(
            app.activity
                .iter()
                .all(|a| !matches!(a, Activity::Session { .. }))
        );

        run(&mut app, "clear stats");
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(message_kind(&app), Some(MsgKind::Success));
        assert_eq!(app.stats.session_count, 0);
    }

    #[test]
    fn export_then_import() {
        let path =
            std::env::temp_dir().join(format!("cmdboard-app-export-{}.json", std::process::id()));
        let mut app = App::with_defaults();
        app.execute(Command::Export {
            path: Some(path.display().to_string()),
        });
        assert_eq!(
            message_kind(&app),
            Some(MsgKind::Success),
            "{:?}",
            app.message
        );
        app.execute(Command::Import {
            path: path.display().to_string(),
        });
        assert_eq!(
            message_kind(&app),
            Some(MsgKind::Success),
            "{:?}",
            app.message
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn stats_of_a_removed_app_fall_back_to_all() {
        let mut app = App::with_defaults();
        run(&mut app, "stats steam");
        assert!(app.stats_app.is_some());
        app.db.delete_app(steam_id(&app)).unwrap();
        app.reload().unwrap();
        assert_eq!(app.stats_app, None);
    }

    #[test]
    fn no_category_selected() {
        let mut app = App::with_defaults();
        app.cat_state.select(None);
        assert!(app.visible_apps().is_empty());
        run(&mut app, "add Paint mspaint.exe");
        assert_eq!(message_kind(&app), Some(MsgKind::Error));
        app.select_app(9_999); // unknown: nothing changes
        assert_eq!(app.cat_state.selected(), None);
    }
}
