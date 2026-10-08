//! Preferences saved to `config.toml` (theme, sort, language), aliases saved to
//! `commands.toml`, export/import, updates, and clearing the history.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use super::{App, AppSort, MsgKind, Outcome};
use crate::command::{Command, alias::Aliases};
use crate::config;
use crate::i18n;
use crate::ui::theme;
use crate::update::{self, Action};

impl App {
    /// Saves one `config.toml` value; nothing is saved in tests.
    fn save_setting(&self, key: &str, value: &str, error: &str) -> anyhow::Result<()> {
        if let Some(path) = &self.config_path {
            config::save_value(path, key, value).with_context(|| error.to_string())?;
        }
        Ok(())
    }

    /// `:theme`: lists the themes, or switches. A broken theme file is an error message;
    /// the current theme stays.
    pub(super) fn theme_command(&mut self, name: Option<&str>) -> Outcome {
        let Some(name) = name else {
            let names = theme::available(self.themes_dir.as_deref());
            let list = names.join(", ");
            let text = t!("theme.list", list, current = self.theme_name);
            return Ok(Some((text, MsgKind::Info)));
        };
        self.theme = theme::load(name, self.themes_dir.as_deref()).map_err(anyhow::Error::msg)?;
        self.theme_name = name.trim().to_lowercase();
        self.save_setting("theme", &self.theme_name, &t!("theme.not_saved"))?;
        Ok(Some((
            t!("theme.set", name = self.theme.name),
            MsgKind::Success,
        )))
    }

    /// `:sort`: lists the orders, or switches, keeping the selected app.
    pub(super) fn sort_command(&mut self, sort: Option<AppSort>) -> Outcome {
        let Some(sort) = sort else {
            let names: Vec<_> = AppSort::ALL.iter().map(|s| s.name()).collect();
            let list = names.join(", ");
            let text = t!("sort.list", list, current = self.sort.name());
            return Ok(Some((text, MsgKind::Info)));
        };
        let selected = self.selected_app().map(|a| a.id);
        self.sort = sort;
        self.sort.apply(&mut self.apps);
        if let Some(id) = selected {
            self.select_app(id);
        }
        self.save_setting("sort", sort.name(), &t!("sort.not_saved"))?;
        let text = t!("sort.set", label = sort.label());
        Ok(Some((text, MsgKind::Success)))
    }

    /// `:lang`: lists the languages, or switches.
    pub(super) fn lang_command(&mut self, code: Option<&str>) -> Outcome {
        let Some(code) = code else {
            let codes: Vec<_> = i18n::LANGS.iter().map(|(code, _)| *code).collect();
            let list = codes.join(", ");
            let text = t!("lang.list", list, current = i18n::current());
            return Ok(Some((text, MsgKind::Info)));
        };
        if !i18n::set(code) {
            bail!(t!("lang.unknown", code));
        }
        self.save_setting("lang", i18n::current(), &t!("lang.not_saved"))?;
        let text = t!("lang.set", name = t!("language"));
        Ok(Some((text, MsgKind::Success)))
    }

    /// `:group`: saves an alias launching these apps to `commands.toml`.
    pub(super) fn save_group(&mut self, name: &str, apps: &[String]) -> Outcome {
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
        self.aliases = Aliases::save(&path, name, &body.join("; "))?;
        let text = t!(
            "action.group_saved",
            name = name.to_lowercase(),
            list = names.join(", ")
        );
        Ok(Some((text, MsgKind::Success)))
    }

    /// Asks before replacing an existing file.
    pub(super) fn export(&mut self, path: Option<String>, confirmed: bool) -> Outcome {
        let path = match path {
            Some(path) => PathBuf::from(path),
            None => self.db.default_export_path()?,
        };
        if path.exists() && !confirmed {
            let message = t!("action.confirm_overwrite", path = path.display());
            // The resolved path, so the dated default cannot change meanwhile.
            let path = Some(path.display().to_string());
            return self.confirm(
                message,
                Command::Export {
                    path,
                    confirmed: true,
                },
            );
        }
        let (apps, sessions) = self.db.export_to(&path)?;
        let text = t!("action.exported", apps, sessions, path = path.display());
        Ok(Some((text, MsgKind::Success)))
    }

    pub(super) fn import(&mut self, path: &str) -> Outcome {
        let imported = self.db.import_from(Path::new(path))?;
        self.reload()?;
        let text = t!(
            "action.imported",
            apps = imported.apps,
            sessions = imported.sessions
        );
        Ok(Some((text, MsgKind::Success)))
    }

    /// `:clear sessions`: hides the history; stats still count it.
    pub(super) fn clear_sessions(&mut self, confirmed: bool) -> Outcome {
        if !confirmed {
            let message = t!("action.confirm_clear_sessions");
            return self.confirm(message, Command::ClearSessions { confirmed: true });
        }
        let count = self.db.hide_sessions()?;
        self.reload()?;
        Ok(Some((
            t!("action.sessions_cleared", count),
            MsgKind::Success,
        )))
    }

    /// `:clear stats`: deletes the finished sessions; XP and rewards stay.
    pub(super) fn clear_stats(&mut self, confirmed: bool) -> Outcome {
        if !confirmed {
            let message = t!("action.confirm_clear_stats");
            return self.confirm(message, Command::ClearStats { confirmed: true });
        }
        let count = self.db.clear_sessions()?;
        self.reload()?;
        Ok(Some((t!("action.stats_cleared", count), MsgKind::Success)))
    }

    /// `:update`, in a short-lived thread.
    pub(super) fn start_update(&mut self) -> Outcome {
        if self.update_running {
            bail!(t!("update.running"));
        }
        let events = self
            .events
            .clone()
            .with_context(|| t!("update.unavailable"))?;
        self.update_running = true;
        update::spawn(events, Action::Install);
        Ok(Some((t!("update.searching"), MsgKind::Info)))
    }

    pub fn on_update_finished(&mut self, action: Action, result: Result<update::Outcome, String>) {
        use update::Outcome::{Available, Installed, UpToDate};
        self.update_running = false;
        let message = match (action, result) {
            // The passive check stays silent, apart from the status bar.
            (Action::Check, Ok(Available { version, .. })) => {
                self.update_available = Some(version);
                return;
            }
            (Action::Check, _) => return,
            (Action::Install, Err(error)) => (t!("update.failed", error), MsgKind::Error),
            (Action::Install, Ok(UpToDate)) => {
                self.update_available = None;
                let text = t!("update.up_to_date", version = update::CURRENT);
                (text, MsgKind::Info)
            }
            (Action::Install, Ok(Available { version, .. })) => {
                let text = t!("update.use_winget", version);
                self.update_available = Some(version);
                (text, MsgKind::Info)
            }
            (Action::Install, Ok(Installed { version })) => {
                self.update_available = None;
                (t!("update.installed", version), MsgKind::Success)
            }
        };
        self.message = Some(message);
    }
}
