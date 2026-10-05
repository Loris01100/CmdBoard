use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
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
use crate::launcher::launch;
use crate::popup::{Form, FormKind, LevelUp, Popup, RewardUnlocked};
use crate::storage::{
    Database,
    models::{AppEntry, Category, NewApp, Profile, RewardView, Stats},
    unix_now,
};
use crate::text_input::TextInput;
use crate::tracker::Watched;
use crate::update;
use crate::ui::{
    self,
    theme::{self, Theme},
};

/// How often running sessions save their time played, in case of a crash.
const CHECKPOINT_EVERY: Duration = Duration::from_secs(60);

/// How many ticks an XP bar takes to fill up to its new value (2 s at 250 ms).
pub const XP_ANIM_FRAMES: u64 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Dashboard,
    Stats,
    Rewards,
    Help,
}

impl Screen {
    pub fn title(self) -> &'static str {
        match self {
            Screen::Dashboard => "Dashboard",
            Screen::Stats => "Stats",
            Screen::Rewards => "Récompenses",
            Screen::Help => "Aide",
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
        xp::animate(self.from, self.to, frame.saturating_sub(self.start), XP_ANIM_FRAMES)
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

    // cached data, reloaded from the database after each write
    pub categories: Vec<Category>,
    pub apps: Vec<AppEntry>,
    pub profile: Profile,
    pub recent_rewards: Vec<String>,
    /// Every reward, for the Rewards screen.
    pub rewards: Vec<RewardView>,
    pub reward_state: TableState,
    /// Stats screen data, for `stats_app` or every app.
    pub stats: Stats,
    pub stats_app: Option<i64>,
    pub stats_state: TableState,

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
            categories: Vec::new(),
            apps: Vec::new(),
            profile: Profile::default(),
            recent_rewards: Vec::new(),
            rewards: Vec::new(),
            reward_state: TableState::default(),
            stats: Stats::default(),
            stats_app: None,
            stats_state: TableState::default(),
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
            should_quit: false,
        };
        app.reload()?;
        Ok(app)
    }

    /// Refreshes the cached data and keeps both selections in range.
    pub fn reload(&mut self) -> anyhow::Result<()> {
        self.categories = self.db.categories()?;
        self.apps = self.db.apps()?;
        self.profile = self.db.profile()?;
        self.recent_rewards = self.db.recent_rewards(3)?;
        if self.stats_app.is_some_and(|id| self.find_app_by_id(id).is_none()) {
            self.stats_app = None; // the app was removed
        }
        self.stats = self.db.stats(self.stats_app)?;
        self.stats_state
            .select(clamp(self.stats_state.selected(), self.stats.sessions.len()));
        self.rewards = self.db.reward_views()?;
        self.reward_state
            .select(clamp(self.reward_state.selected(), self.rewards.len()));
        self.cat_state
            .select(clamp(self.cat_state.selected(), self.categories.len()));
        let visible = self.visible_apps().len();
        self.app_state.select(clamp(self.app_state.selected(), visible));
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
    }

    fn send_watch_list(&self) {
        let Some(tracker) = &self.tracker else { return };
        let list = self
            .apps
            .iter()
            .filter_map(|a| {
                let exe = a.watch_exe.as_deref()?.trim();
                (!exe.is_empty()).then(|| Watched { app_id: a.id, exe: exe.into() })
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
            match events.recv()? {
                AppEvent::Key(key) => self.on_key(key),
                AppEvent::Tick => self.on_tick(),
                AppEvent::SessionStarted { app_id } => self.on_session_start(app_id),
                AppEvent::SessionEnded { app_id, secs } => self.on_session_end(app_id, secs),
                AppEvent::UpdateFinished { action, result } => self.on_update_finished(action, result),
            }
        }
        self.end_all_sessions()
    }

    pub fn on_update_finished(&mut self, action: update::Action, result: Result<update::Outcome, String>) {
        use update::{Action, Outcome};
        self.update_running = false;
        let message = match (action, result) {
            // The passive check stays silent, apart from the status bar.
            (Action::Check, Ok(Outcome::Available { version, .. })) => {
                self.update_available = Some(version);
                return;
            }
            (Action::Check, _) => return,
            (Action::Install, Err(e)) => (format!("Mise à jour : {e}"), MsgKind::Error),
            (Action::Install, Ok(Outcome::UpToDate)) => {
                self.update_available = None;
                (format!("CmdBoard est à jour (v{})", update::CURRENT), MsgKind::Info)
            }
            (Action::Install, Ok(Outcome::Available { version, .. })) => {
                let text = format!("v{version} disponible. Installé via MSI/winget : lancez « winget upgrade CmdBoard »");
                self.update_available = Some(version);
                (text, MsgKind::Info)
            }
            (Action::Install, Ok(Outcome::Installed { version })) => {
                self.update_available = None;
                (format!("Mis à jour en v{version} : relancez CmdBoard"), MsgKind::Success)
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
        let Some(name) = self.app_name(app_id) else { return };
        match self.db.start_session(app_id, unix_now()) {
            Ok(session_id) => {
                let now = Instant::now();
                self.active_sessions.insert(
                    app_id,
                    ActiveSession { session_id, started: now, last_checkpoint: now },
                );
                self.message = Some((format!("Session démarrée : {name}"), MsgKind::Info));
            }
            Err(e) => self.message = Some((format!("{e:#}"), MsgKind::Error)),
        }
    }

    /// `secs` is measured by the tracker, from detection to disappearance.
    pub fn on_session_end(&mut self, app_id: i64, secs: u64) {
        let Some(session) = self.active_sessions.remove(&app_id) else { return };
        let Some(before) = self.find_app_by_id(app_id).map(|a| a.total_xp) else {
            return; // removed meanwhile
        };
        let profile_before = self.profile.total_xp;
        let result = self
            .finish_session(session.session_id, secs)
            .and_then(|outcome| self.reload().map(|()| outcome));
        let Some(name) = self.app_name(app_id) else { return };
        let message = match result {
            Ok(Some(outcome)) => {
                let text = format!("Session terminée : {name} ({} min, +{} XP)", secs / 60, outcome.xp);
                let message = match outcome.rule_errors.first() {
                    Some(error) => (format!("{text} · {error}"), MsgKind::Error),
                    None => (text, MsgKind::Success),
                };
                self.on_xp_changed(app_id, before, profile_before, outcome.xp);
                self.queue_rewards(outcome.rewards);
                message
            }
            Ok(None) => (
                format!("Session trop courte, non enregistrée : {name}"),
                MsgKind::Info,
            ),
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
        Ok(SessionOutcome { xp, rewards, rule_errors })
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
    fn unlock_rewards(&self, session_id: i64) -> anyhow::Result<(Vec<RewardUnlocked>, Vec<String>)> {
        let (app_id, facts) = self.db.session_facts(session_id)?;
        let app_name = self.app_name(app_id);
        let (mut unlocked, mut errors) = (Vec::new(), Vec::new());
        for reward in self.db.pending_rewards(app_id)? {
            match rewards::evaluate(&reward.rule, &facts) {
                Ok(true) => {
                    let for_app = reward.per_app.then_some(app_id);
                    self.db.unlock_reward(reward.id, for_app, session_id, unix_now())?;
                    unlocked.push(RewardUnlocked {
                        name: reward.name,
                        description: reward.description,
                        app: if reward.per_app { app_name.clone() } else { None },
                    });
                }
                Ok(false) => {}
                Err(e) => errors.push(format!("règle « {} » : {e}", reward.code)),
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
        let text = format!("{} session(s) interrompue(s) récupérée(s)", closed.len());
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
        let Some(entry) = self.find_app_by_id(app_id) else { return };
        let (name, app_after, app_level) = (entry.name.clone(), entry.total_xp, entry.level);
        let frame = self.frame_count;

        // Start from what is on screen, in case a previous animation is still running.
        let from = self.xp_anims.get(&app_id).map_or(app_before, |a| a.value(frame));
        self.xp_anims.insert(app_id, XpAnim { from, to: app_after, start: frame });
        let from = self.profile_anim.map_or(profile_before, |a| a.value(frame));
        self.profile_anim = Some(XpAnim { from, to: self.profile.total_xp, start: frame });

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
        self.app_state.select(clamp(Some(0), self.visible_apps().len()));
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
                    self.app_state.select(clamp(Some(0), self.visible_apps().len()));
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
        self.app_state.select(clamp(self.app_state.selected(), visible));
    }

    /// Normal-mode key bindings (plan section 7).
    fn key_to_command(&self, key: KeyEvent) -> Option<Command> {
        Some(match key.code {
            KeyCode::Char('q') => Command::Quit,
            KeyCode::Char('1') => Command::Show(Screen::Dashboard),
            KeyCode::Char('2') => Command::Show(Screen::Stats),
            KeyCode::Char('3') => Command::Show(Screen::Rewards),
            KeyCode::Char('4') | KeyCode::Char('?') => Command::Show(Screen::Help),
            KeyCode::Down | KeyCode::Char('j') => Command::SelectNext,
            KeyCode::Up | KeyCode::Char('k') => Command::SelectPrev,
            KeyCode::Tab | KeyCode::BackTab => Command::ToggleFocus,
            KeyCode::Left | KeyCode::Char('h') => Command::FocusPanel(Focus::Categories),
            KeyCode::Right | KeyCode::Char('l') => Command::FocusPanel(Focus::Apps),
            _ if self.screen != Screen::Dashboard => return None,
            KeyCode::Enter => match self.focus {
                Focus::Categories => Command::FocusPanel(Focus::Apps),
                Focus::Apps => Command::Launch {
                    app: self.selected_app()?.name.clone(),
                },
            },
            KeyCode::Char('a') => Command::OpenForm(FormKind::AddApp),
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
        let Mode::Popup(popup) = &mut self.mode else { return };
        match popup {
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

    /// Runs the form's command. On error the form stays open and shows it.
    fn submit_form(&mut self) {
        let Mode::Popup(Popup::Form(form)) = &mut self.mode else { return };
        let command = match form.to_command() {
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
        self.message = Some(("Annulé".into(), MsgKind::Info));
    }

    /// Tab in the command line: completes commands, apps and categories.
    fn complete_command(&mut self, forward: bool) {
        let themes = theme::available(self.themes_dir.as_deref());
        let sources = Sources {
            themes: themes.iter().map(String::as_str).collect(),
            aliases: self.aliases.names().collect(),
            apps: self.apps.iter().map(|a| a.name.as_str()).collect(),
            categories: self.categories.iter().map(|c| c.name.as_str()).collect(),
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
                self.message = Some((format!("alias : {e}"), MsgKind::Error));
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
                    self.message = Some((format!("{sub} : {e:#}"), MsgKind::Error));
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
            Command::Show(screen) => self.screen = screen,
            Command::SelectNext => self.move_selection(true),
            Command::SelectPrev => self.move_selection(false),
            Command::FocusPanel(focus) => self.focus = focus,
            Command::ToggleFocus => {
                self.focus = match self.focus {
                    Focus::Categories => Focus::Apps,
                    Focus::Apps => Focus::Categories,
                }
            }
            Command::Quit => self.should_quit = true,

            Command::Launch { app } => {
                let entry = self.app_named(&app)?;
                launch::launch(&entry.launch_target)?;
                return success(format!("Lancé : {}", entry.name));
            }
            Command::Add { name, target, category, watch_exe } => {
                if self.find_app(&name).is_some() {
                    bail!("« {name} » existe déjà");
                }
                launch::check_target(&target)?;
                let (category_id, created) = match category {
                    Some(category) => self.category_or_create(&category)?,
                    None => {
                        let selected = self
                            .selected_category()
                            .context("aucune catégorie sélectionnée : précisez-en une")?;
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
                return success(format!("Ajouté : {name}{}", created_note(created)));
            }
            Command::Move { app, category } => {
                let (id, name) = {
                    let entry = self.app_named(&app)?;
                    (entry.id, entry.name.clone())
                };
                let (category_id, created) = self.category_or_create(&category)?;
                self.db.move_app(id, category_id)?;
                self.reload()?;
                self.select_app(id);
                let category = &self.selected_category().map_or(category, |c| c.name.clone());
                return success(format!("{name} → {category}{}", created_note(created)));
            }
            Command::RemoveApp { app, confirmed } => {
                let (id, name) = {
                    let entry = self.app_named(&app)?;
                    (entry.id, entry.name.clone())
                };
                if !confirmed {
                    self.mode = Mode::Popup(Popup::Confirm {
                        message: format!(
                            "Supprimer « {name} » ? Son temps de jeu, son XP et ses \
                             récompenses seront perdus."
                        ),
                        command: Command::RemoveApp { app: name, confirmed: true },
                    });
                    return Ok(None);
                }
                self.db.delete_app(id)?;
                self.reload()?;
                return success(format!("Supprimé : {name}"));
            }
            Command::RemoveCategory { category, confirmed } => {
                let (id, name) = {
                    let found = self.category_named(&category)?;
                    (found.id, found.name.clone())
                };
                let count = self.app_count(id);
                if count > 0 {
                    bail!("« {name} » contient encore {count} app(s) : déplacez-les ou supprimez-les d'abord");
                }
                if !confirmed {
                    self.mode = Mode::Popup(Popup::Confirm {
                        message: format!("Supprimer la catégorie « {name} » ?"),
                        command: Command::RemoveCategory { category: name, confirmed: true },
                    });
                    return Ok(None);
                }
                self.db.delete_category(id)?;
                self.reload()?;
                return success(format!("Catégorie supprimée : {name}"));
            }
            Command::OpenForm(kind) => {
                let form = match kind {
                    FormKind::AddApp => {
                        let category = self.selected_category().map_or("", |c| c.name.as_str());
                        Form::add_app(category)
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
            }
            Command::Xp { app, amount } => {
                let (id, name, before) = {
                    let entry = self.app_named(&app)?;
                    (entry.id, entry.name.clone(), entry.total_xp)
                };
                let profile_before = self.profile.total_xp;
                let after = xp::apply_delta(before, amount);
                self.db.set_app_xp(id, after)?;
                self.reload()?;
                self.on_xp_changed(id, before, profile_before, after.saturating_sub(before));
                let change = after as i64 - before as i64;
                return success(format!("{name} : {change:+} XP (total {after})"));
            }
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
                let text = format!("Thèmes : {} (actuel : {})", names.join(", "), self.theme_name);
                return Ok(Some((text, MsgKind::Info)));
            }
            Command::Theme { name: Some(name) } => {
                // A broken theme file is an error message; the current theme stays.
                self.theme = theme::load(&name, self.themes_dir.as_deref()).map_err(anyhow::Error::msg)?;
                self.theme_name = name.trim().to_lowercase();
                if let Some(path) = &self.config_path {
                    config::save_value(path, "theme", self.theme_name.as_str())
                        .context("thème appliqué mais non mémorisé")?;
                }
                return success(format!("Thème : {}", self.theme.name));
            }
            Command::Update => {
                if self.update_running {
                    bail!("vérification de mise à jour déjà en cours");
                }
                let events = self.events.clone().context("mise à jour indisponible")?;
                self.update_running = true;
                update::spawn(events, update::Action::Install);
                return Ok(Some(("Recherche d'une mise à jour…".into(), MsgKind::Info)));
            }
            Command::Help { command: None } => self.screen = Screen::Help,
            Command::Help { command: Some(name) } => {
                if let Some(body) = self.aliases.get(&name) {
                    return Ok(Some((format!("alias {name} : {body}"), MsgKind::Info)));
                }
                let help = find_help(&name).with_context(|| format!("commande inconnue : {name}"))?;
                return Ok(Some((format!("{} : {}", help.usage, help.summary), MsgKind::Info)));
            }
        }
        Ok(None)
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
            .with_context(|| format!("app inconnue : {name}"))
    }

    fn category_named(&self, name: &str) -> anyhow::Result<&Category> {
        let lower = name.to_lowercase();
        self.categories
            .iter()
            .find(|c| c.name.to_lowercase() == lower)
            .with_context(|| format!("catégorie inconnue : {name}"))
    }

    /// Id of the category with this name (ignoring case), created if missing.
    fn category_or_create(&mut self, name: &str) -> anyhow::Result<(i64, bool)> {
        match self.category_named(name) {
            Ok(category) => Ok((category.id, false)),
            Err(_) => Ok((self.db.add_category(name)?, true)),
        }
    }

    pub fn selected_category(&self) -> Option<&Category> {
        self.cat_state.selected().and_then(|i| self.categories.get(i))
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
            Some(cat) => self.apps.iter().filter(|a| a.category_id == cat.id).collect(),
            None => Vec::new(),
        }
    }

    pub fn selected_app(&self) -> Option<&AppEntry> {
        let i = self.app_state.selected()?;
        self.visible_apps().get(i).copied()
    }

    pub fn app_count(&self, category_id: i64) -> usize {
        self.apps.iter().filter(|a| a.category_id == category_id).count()
    }

    /// Selects an app and its category, and focuses the apps panel.
    fn select_app(&mut self, id: i64) {
        let Some(category_id) = self.apps.iter().find(|a| a.id == id).map(|a| a.category_id)
        else {
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
                let next = step(self.stats_state.selected(), self.stats.sessions.len(), forward);
                self.stats_state.select(next);
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
                let next = step(self.app_state.selected(), self.visible_apps().len(), forward);
                self.app_state.select(next);
            }
        }
    }

    fn reset_app_selection(&mut self) {
        let selected = (!self.visible_apps().is_empty()).then_some(0);
        self.app_state = TableState::default().with_selected(selected);
    }
}

fn created_note(created: bool) -> &'static str {
    if created { " (nouvelle catégorie)" } else { "" }
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
        db.seed_defaults().unwrap();
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
        run(&mut app, r#"add "Hollow Knight" steam://rungameid/367520 Metroidvania"#);
        assert_eq!(message_kind(&app), Some(MsgKind::Success), "{:?}", app.message);
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
        assert_eq!(app.message.as_ref().unwrap().0, "Bloc-notes → Dev");
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
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(form(&app).fields[2].input.text(), "Jeux"); // selected category

        type_text(&mut app, "Paint");
        press(&mut app, KeyCode::Enter);
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
        assert_eq!(app.message.as_ref().unwrap().0, "Session terminée : Steam (42 min, +47 XP)");
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
        assert_eq!(app.recent_rewards[0], "Marathon (Bloc-notes)");
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
        assert!(text.contains("règle « marathon » : variable inconnue : hours"), "{text}");
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
        let mut app = with_aliases("[alias]\nboost = \"xp $1 50; stats $1\"");
        run(&mut app, "boost bloc-notes");
        assert_eq!(app.find_app("Bloc-notes").unwrap().total_xp, 50);
        assert_eq!(app.screen, Screen::Stats);
        assert_eq!(app.stats_app, Some(app.find_app("Bloc-notes").unwrap().id));

        run(&mut app, "boost");
        assert_eq!(app.message.as_ref().unwrap().0, "alias : argument $1 manquant");
        run(&mut app, "help boost");
        assert_eq!(app.message.as_ref().unwrap().0, "alias boost : xp $1 50; stats $1");
    }

    #[test]
    fn alias_stops_at_first_error_or_confirmation() {
        let mut app = with_aliases("[alias]\nbad = \"xp nope 5; xp steam 5\"\nclean = \"rm steam; xp steam 5\"");
        run(&mut app, "bad");
        assert_eq!(message_kind(&app), Some(MsgKind::Error));
        assert!(app.message.as_ref().unwrap().0.starts_with("xp nope 5 : app inconnue"));
        assert_eq!(app.find_app("Steam").unwrap().total_xp, 0);

        run(&mut app, "clean");
        assert!(matches!(app.mode, Mode::Popup(Popup::Confirm { .. })));
        assert_eq!(app.find_app("Steam").unwrap().total_xp, 0);
    }

    #[test]
    fn tab_completes_in_the_command_line() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char(':'));
        type_text(&mut app, "la");
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
        assert!(text.starts_with("Thèmes : catppuccin-frappe, catppuccin-latte"), "{text}");
        assert!(text.ends_with("(actuel : terminal)"));

        run(&mut app, "theme Catppuccin-Latte");
        assert_eq!(app.message.as_ref().unwrap().0, "Thème : Catppuccin Latte");
        assert_eq!(app.theme_name, "catppuccin-latte");

        run(&mut app, "theme nope");
        assert_eq!(message_kind(&app), Some(MsgKind::Error));
        assert_eq!(app.theme.name, "Catppuccin Latte");
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
        app.execute(Command::Xp { app: "Steam".into(), amount: 100 });
        assert_eq!(app.mode, Mode::Command); // not interrupted
        press(&mut app, KeyCode::Esc);
        app.on_tick();
        assert!(matches!(app.mode, Mode::Popup(Popup::LevelUp(_))));
    }

    #[test]
    fn xp_command_adds_and_removes() {
        let mut app = App::with_defaults();
        run(&mut app, "xp windows terminal 250");
        assert_eq!(app.find_app("Windows Terminal").unwrap().total_xp, 250);
        assert!(matches!(
            &app.mode,
            Mode::Popup(Popup::LevelUp(LevelUp { app_level: Some(2), .. }))
        ));
        press(&mut app, KeyCode::Esc);

        run(&mut app, "xp windows terminal -1000");
        assert_eq!(app.message.as_ref().unwrap().0, "Windows Terminal : -250 XP (total 0)");
        assert_eq!(app.find_app("Windows Terminal").unwrap().total_xp, 0);
        assert_eq!(app.mode, Mode::Normal); // going down is no level-up
    }

    #[test]
    fn tracker_receives_watch_list_on_reload() {
        let mut app = App::with_defaults();
        let (tx, rx) = std::sync::mpsc::channel();
        app.attach_tracker(tx);
        let list = rx.try_recv().unwrap();
        assert_eq!(list.len(), 4); // Explorateur has no watch_exe
        assert!(list.contains(&Watched { app_id: steam_id(&app), exe: "steam.exe".into() }));

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
        let available = Outcome::Available { version: "0.2.0".into(), managed: true };
        app.on_update_finished(Action::Check, Ok(available.clone()));
        assert_eq!((app.update_available.as_deref(), &app.message), (Some("0.2.0"), &None));

        app.on_update_finished(Action::Install, Ok(available));
        assert!(app.message.as_ref().unwrap().0.contains("winget upgrade CmdBoard"));
        app.on_update_finished(Action::Install, Ok(Outcome::Installed { version: "0.2.0".into() }));
        assert_eq!(message_kind(&app), Some(MsgKind::Success));
        assert_eq!(app.update_available, None);
    }

    #[test]
    fn q_quits() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit);
    }
}
