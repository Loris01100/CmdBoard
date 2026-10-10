//! The "add app" picker (installed apps, scanned in a short-lived thread) and the
//! add/edit/move forms. A form builds a `Command`; its errors stay inside the form.

use crossterm::event::{KeyCode, KeyEvent};

use super::{App, Mode};
use crate::event::AppEvent;
use crate::fuzzy;
use crate::launcher::scan::{self, Shortcut};
use crate::popup::{Form, FormKind, Picker, Popup};
use crate::storage::models::AppEntry;

impl App {
    pub(super) fn open_form(&mut self, kind: FormKind) -> anyhow::Result<()> {
        let form = match kind {
            FormKind::Add => {
                let category = self.selected_category().map_or("", |c| c.name.as_str());
                let picker = Picker::new(category);
                self.mode = Mode::Popup(Popup::Picker(picker));
                self.start_shortcut_scan();
                return Ok(());
            }
            FormKind::Edit { app } => {
                let entry = self.app_named(&app)?;
                Form::edit_app(
                    &entry.name,
                    &entry.launch_target,
                    self.category_of(entry),
                    entry.watch_exe.as_deref(),
                    entry.launch_args.as_deref(),
                )
            }
            FormKind::Move { app } => {
                let entry = self.app_named(&app)?;
                Form::move_app(&entry.name, self.category_of(entry))
            }
        };
        self.mode = Mode::Popup(Popup::Form(form));
        Ok(())
    }

    fn category_of(&self, entry: &AppEntry) -> &str {
        self.categories
            .iter()
            .find(|c| c.id == entry.category_id)
            .map_or("", |c| c.name.as_str())
    }

    /// Scans the installed apps in a short-lived thread (about 0.1 s of disk reads).
    fn start_shortcut_scan(&mut self) {
        if !self.scan_running {
            self.scan_running = self.spawn(|| AppEvent::ShortcutsScanned(scan::scan()));
        }
    }

    pub(super) fn on_shortcuts_scanned(&mut self, found: Vec<Shortcut>) {
        self.scan_running = false;
        self.shortcuts = found;
        // The list changed: back to the best match.
        if let Mode::Popup(Popup::Picker(picker)) = &mut self.mode {
            picker.selected = 0;
        }
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

    pub(super) fn on_picker_key(&mut self, key: KeyEvent) {
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
                picker.selected = (picker.selected + count - 1) % count;
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

    pub(super) fn on_form_key(&mut self, key: KeyEvent) {
        let Mode::Popup(Popup::Form(form)) = &mut self.mode else {
            return;
        };
        match key.code {
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
}
