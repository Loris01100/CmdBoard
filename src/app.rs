use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    DefaultTerminal,
    widgets::{ListState, TableState},
};

use crate::storage::models::{AppEntry, Category, Profile};
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

pub struct App {
    // navigation
    pub screen: Screen,
    pub focus: Focus,
    pub cat_state: ListState,
    pub app_state: TableState,

    // cached data (fake until SQLite lands in step 4)
    pub categories: Vec<Category>,
    pub apps: Vec<AppEntry>,
    pub profile: Profile,
    pub recent_rewards: Vec<String>,

    pub theme: Theme,
    pub should_quit: bool,
}

impl App {
    pub fn new() -> Self {
        let (categories, apps, profile, recent_rewards) = fake_data();
        let cat_selected = (!categories.is_empty()).then_some(0);
        let mut app = Self {
            screen: Screen::Dashboard,
            focus: Focus::Categories,
            cat_state: ListState::default().with_selected(cat_selected),
            app_state: TableState::default(),
            categories,
            apps,
            profile,
            recent_rewards,
            theme: Theme::default(),
            should_quit: false,
        };
        app.reset_app_selection();
        app
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

    // Keys map directly to actions for now; they will go through `Command` in step 5.
    pub fn on_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true
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
            _ => {}
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

// Placeholder data until step 4 (SQLite).
fn fake_data() -> (Vec<Category>, Vec<AppEntry>, Profile, Vec<String>) {
    let categories = [(1, "Jeux"), (2, "Dev"), (3, "Créa")]
        .map(|(id, name)| Category { id, name: name.into() })
        .to_vec();

    let app = |name: &str, category_id, level, xp, hours: u64, last: Option<&str>, rewards| {
        AppEntry {
            name: name.into(),
            category_id,
            level,
            xp,
            total_secs: hours * 3600,
            last_played: last.map(Into::into),
            rewards,
        }
    };
    let apps = vec![
        app("Elden Ring", 1, 12, 3300, 96, Some("hier"), 3),
        app("Hades", 1, 7, 1100, 42, Some("il y a 3 jours"), 2),
        app("Celeste", 1, 4, 720, 18, Some("la semaine dernière"), 1),
        app("Hollow Knight", 1, 9, 400, 55, None, 0),
        app("VS Code", 2, 15, 2900, 210, Some("aujourd'hui"), 4),
        app("Windows Terminal", 2, 6, 650, 30, Some("aujourd'hui"), 1),
        app("Krita", 3, 3, 90, 8, Some("le mois dernier"), 0),
        app("Blender", 3, 5, 800, 25, Some("hier"), 1),
    ];

    let profile = Profile {
        level: 23,
        xp: 8800,
        streak_days: 5,
        xp_today: 120,
    };
    let recent_rewards = ["Marathon (Elden Ring)", "Lève-tôt", "Streak 5 jours"]
        .map(String::from)
        .to_vec();

    (categories, apps, profile, recent_rewards)
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
    fn changing_category_resets_app_selection() {
        let mut app = App::new();
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.app_state.selected(), Some(1));

        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected_category().unwrap().name, "Dev");
        assert_eq!(app.selected_app().unwrap().name, "VS Code");
    }

    #[test]
    fn q_quits() {
        let mut app = App::new();
        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit);
    }
}
