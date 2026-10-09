//! Key bindings per mode (plan section 7). Normal-mode keys map to a `Command`; screen
//! specific bindings live with their screen (`storage`, `optimize`), popups in `forms`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{App, Focus, Mode, MsgKind, Screen, clamp};
use crate::command::Command;
use crate::popup::{FormKind, Popup};
use crate::text_input::TextInput;

impl App {
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

    /// Normal-mode key bindings: global ones first, then the screen's.
    pub(super) fn key_to_command(&self, key: KeyEvent) -> Option<Command> {
        Some(match key.code {
            KeyCode::Char('q') => Command::Quit,
            KeyCode::Char('1') => Command::Show(Screen::Dashboard),
            KeyCode::Char('2') => Command::Show(Screen::Stats),
            KeyCode::Char('3') => Command::Show(Screen::Rewards),
            KeyCode::Char('4') => Command::Show(Screen::Storage),
            KeyCode::Char('5') => Command::Show(Screen::Optimize),
            KeyCode::Char('0' | '?') => Command::Show(Screen::Help),
            KeyCode::Down | KeyCode::Char('j') => Command::SelectNext,
            KeyCode::Up | KeyCode::Char('k') => Command::SelectPrev,
            _ if self.screen == Screen::Storage => self.storage_key(key)?,
            _ if self.screen == Screen::Optimize => self.optimize_key(key)?,
            KeyCode::Tab | KeyCode::BackTab => Command::ToggleFocus,
            KeyCode::Left | KeyCode::Char('h') => Command::FocusPanel(Focus::Categories),
            KeyCode::Right | KeyCode::Char('l') => Command::FocusPanel(Focus::Apps),
            KeyCode::Char('s') if self.screen == Screen::Stats => Command::ToggleStatsPie,
            _ if self.screen == Screen::Dashboard => self.dashboard_key(key)?,
            _ => return None,
        })
    }

    fn dashboard_key(&self, key: KeyEvent) -> Option<Command> {
        let app = || Some(self.selected_app()?.name.clone());
        Some(match key.code {
            KeyCode::Enter => match self.focus {
                Focus::Categories => Command::FocusPanel(Focus::Apps),
                Focus::Apps => Command::Launch { app: app()? },
            },
            KeyCode::Char('a') => Command::OpenForm(FormKind::Add),
            KeyCode::Char('s') => Command::Sort {
                by: Some(self.sort.next()),
            },
            KeyCode::Char('e') => Command::OpenForm(FormKind::Edit { app: app()? }),
            KeyCode::Char('m') => Command::OpenForm(FormKind::Move { app: app()? }),
            KeyCode::Char('d') => match self.focus {
                Focus::Categories => Command::RemoveCategory {
                    category: self.selected_category()?.name.clone(),
                    confirmed: false,
                },
                Focus::Apps => Command::RemoveApp {
                    app: app()?,
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
        let Mode::Popup(popup) = &self.mode else {
            return;
        };
        match popup {
            Popup::Picker(_) => self.on_picker_key(key),
            Popup::Form(_) => self.on_form_key(key),
            Popup::Confirm { command, .. } => match key.code {
                KeyCode::Enter | KeyCode::Char('o' | 'O' | 'y' | 'Y') => {
                    let command = command.clone();
                    self.mode = Mode::Normal;
                    self.execute(command);
                }
                KeyCode::Esc | KeyCode::Char('n' | 'N') => self.cancel_popup(),
                _ => {}
            },
            Popup::LevelUp(_) | Popup::RewardUnlocked(_) => {
                if matches!(key.code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char(' ')) {
                    self.mode = Mode::Normal;
                    self.show_pending_popup();
                }
            }
        }
    }

    pub(super) fn cancel_popup(&mut self) {
        self.mode = Mode::Normal;
        self.message = Some((t!("cancelled"), MsgKind::Info));
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
}
