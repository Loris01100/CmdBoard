use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal,
    widgets::{ListState, TableState},
};

use crate::command::Command;
use crate::launcher::launch;
use crate::storage::{
    Database,
    models::{AppEntry, Category, Profile},
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Categories,
    Apps,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgKind {
    Success,
    Error,
}

pub struct App {
    // navigation
    pub screen: Screen,
    pub focus: Focus,
    pub cat_state: ListState,
    pub app_state: TableState,

    // cached data, reloaded from the database after each write
    pub categories: Vec<Category>,
    pub apps: Vec<AppEntry>,
    pub profile: Profile,
    pub recent_rewards: Vec<String>,

    /// Feedback from the last command, shown on the command line row.
    pub message: Option<(String, MsgKind)>,

    pub theme: Theme,
    pub db: Database,
    pub should_quit: bool,
}

impl App {
    pub fn new(db: Database) -> anyhow::Result<Self> {
        let mut app = Self {
            screen: Screen::Dashboard,
            focus: Focus::Categories,
            cat_state: ListState::default(),
            app_state: TableState::default(),
            categories: Vec::new(),
            apps: Vec::new(),
            profile: Profile::default(),
            recent_rewards: Vec::new(),
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

    // Navigation keys still act directly; they move to `Command` with the parser in step 5.
    pub fn on_key(&mut self, key: KeyEvent) {
        self.message = None;
        match key.code {
            KeyCode::Char('q') => self.execute(Command::Quit),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.execute(Command::Quit)
            }
            KeyCode::Char('1') => self.screen = Screen::Dashboard,
            KeyCode::Char('2') => self.screen = Screen::Stats,
            KeyCode::Char('3') => self.screen = Screen::Rewards,
            KeyCode::Char('4') => self.screen = Screen::Help,
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(true),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(false),
            KeyCode::Tab | KeyCode::BackTab => self.toggle_focus(),
            KeyCode::Left | KeyCode::Char('h') => self.focus = Focus::Categories,
            KeyCode::Right | KeyCode::Char('l') => self.focus = Focus::Apps,
            KeyCode::Enter if self.screen == Screen::Dashboard => match self.focus {
                Focus::Categories => self.focus = Focus::Apps,
                Focus::Apps => {
                    if let Some(app) = self.selected_app() {
                        let app = app.name.clone();
                        self.execute(Command::Launch { app });
                    }
                }
            },
            _ => {}
        }
    }

    /// Single execution path for every `Command`.
    pub fn execute(&mut self, command: Command) {
        match command {
            Command::Quit => self.should_quit = true,
            Command::Launch { app } => {
                let Some(entry) = self.find_app(&app) else {
                    self.message = Some((format!("App inconnue : {app}"), MsgKind::Error));
                    return;
                };
                self.message = Some(match launch::launch(&entry.launch_target) {
                    Ok(()) => (format!("Lancé : {}", entry.name), MsgKind::Success),
                    Err(e) => (format!("{e:#}"), MsgKind::Error),
                });
            }
        }
    }

    /// Case-insensitive lookup by name.
    pub fn find_app(&self, name: &str) -> Option<&AppEntry> {
        self.apps.iter().find(|a| a.name.eq_ignore_ascii_case(name))
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

    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Categories => Focus::Apps,
            Focus::Apps => Focus::Categories,
        };
    }

    fn reset_app_selection(&mut self) {
        let selected = (!self.visible_apps().is_empty()).then_some(0);
        self.app_state = TableState::default().with_selected(selected);
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
    fn launching_unknown_app_reports_error() {
        let mut app = App::with_defaults();
        app.execute(Command::Launch { app: "Inexistant".into() });
        assert!(matches!(app.message, Some((_, MsgKind::Error))));
        assert!(app.find_app("bloc-NOTES").is_some());
    }

    #[test]
    fn q_quits() {
        let mut app = App::with_defaults();
        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit);
    }
}
