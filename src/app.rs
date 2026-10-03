use anyhow::{Context, bail};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal,
    widgets::{ListState, TableState},
};

use crate::command::{Command, find_help, line::CommandLine, parser};
use crate::launcher::launch;
use crate::storage::{
    Database,
    models::{AppEntry, Category, NewApp, Profile},
};
use crate::ui::{self, theme::Theme};

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

/// How keys are interpreted. Search and popups come in steps 6 and 10.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// Typing after `:`.
    Command,
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

    // command line
    pub command_line: CommandLine,
    /// Feedback from the last command, shown on the command line row.
    pub message: Option<Message>,

    pub theme: Theme,
    pub db: Database,
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
            command_line: CommandLine::default(),
            message: None,
            theme: Theme::default(),
            db,
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
        self.cat_state
            .select(clamp(self.cat_state.selected(), self.categories.len()));
        let visible = self.visible_apps().len();
        self.app_state.select(clamp(self.app_state.selected(), visible));
        Ok(())
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
        while !self.should_quit {
            terminal.draw(|f| ui::draw(f, self))?;
            // Windows sends both Press and Release: only handle Press.
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    self.on_key(key);
                }
            }
        }
        Ok(())
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.execute(Command::Quit);
            return;
        }
        match self.mode {
            Mode::Normal => self.on_normal_key(key),
            Mode::Command => self.on_command_key(key),
        }
    }

    fn on_normal_key(&mut self, key: KeyEvent) {
        self.message = None;
        if key.code == KeyCode::Char(':') {
            self.mode = Mode::Command;
            return;
        }
        if let Some(command) = self.key_to_command(key) {
            self.execute(command);
        }
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
            KeyCode::Enter if self.screen == Screen::Dashboard => match self.focus {
                Focus::Categories => Command::FocusPanel(Focus::Apps),
                Focus::Apps => Command::Launch {
                    app: self.selected_app()?.name.clone(),
                },
            },
            _ => return None,
        })
    }

    fn on_command_key(&mut self, key: KeyEvent) {
        let line = &mut self.command_line;
        match key.code {
            KeyCode::Esc => {
                line.clear();
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => {
                let text = line.submit();
                self.mode = Mode::Normal;
                match parser::parse(&text) {
                    Ok(command) => self.execute(command),
                    Err(e) => self.message = Some((e, MsgKind::Error)),
                }
            }
            // Backspace on an empty line leaves command mode, like Vim.
            KeyCode::Backspace if line.input.is_empty() => self.mode = Mode::Normal,
            KeyCode::Backspace => line.backspace(),
            KeyCode::Delete => line.delete(),
            KeyCode::Left => line.left(),
            KeyCode::Right => line.right(),
            KeyCode::Home => line.home(),
            KeyCode::End => line.end(),
            KeyCode::Up => line.history_prev(),
            KeyCode::Down => line.history_next(),
            KeyCode::Char(c) => line.insert(c),
            _ => {}
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
            Command::Add { name, target, category } => {
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
                    watch_exe: launch::watch_exe_for(&target),
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
            Command::Help { command: None } => self.screen = Screen::Help,
            Command::Help { command: Some(name) } => {
                let help = find_help(&name).with_context(|| format!("commande inconnue : {name}"))?;
                return Ok(Some((format!("{} : {}", help.usage, help.summary), MsgKind::Info)));
            }
        }
        Ok(None)
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

    /// Id of the category with this name (ignoring case), created if missing.
    fn category_or_create(&mut self, name: &str) -> anyhow::Result<(i64, bool)> {
        let lower = name.to_lowercase();
        match self.categories.iter().find(|c| c.name.to_lowercase() == lower) {
            Some(category) => Ok((category.id, false)),
            None => Ok((self.db.add_category(name)?, true)),
        }
    }

    pub fn selected_category(&self) -> Option<&Category> {
        self.cat_state.selected().and_then(|i| self.categories.get(i))
    }

    /// Apps of the selected category, in display order.
    pub fn visible_apps(&self) -> Vec<&AppEntry> {
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
        assert_eq!(app.command_line.input, "help add");
    }

    #[test]
    fn q_quits() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit);
    }
}
