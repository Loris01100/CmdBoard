//! Single execution path for every `Command`, whether it comes from a key, the command
//! line, an alias or a form. Each arm delegates to the module owning that feature.

use anyhow::Context;
use ratatui::widgets::TableState;

use super::{App, Focus, Mode, MsgKind, Outcome, Screen, optimize, step};
use crate::command::{
    Command,
    complete::{self, Sources},
    find_help, parser,
};
use crate::ui::theme;

impl App {
    /// Runs a command and shows its outcome as a message.
    pub fn execute(&mut self, command: Command) {
        match self.run_command(command) {
            Ok(Some(message)) => self.message = Some(message),
            Ok(None) => {}
            Err(e) => self.message = Some((format!("{e:#}"), MsgKind::Error)),
        }
    }

    pub(super) fn run_command(&mut self, command: Command) -> Outcome {
        match command {
            // navigation, bound to keys
            Command::Show(screen) => self.show(screen),
            Command::SelectNext => self.move_selection(true),
            Command::SelectPrev => self.move_selection(false),
            Command::FocusPanel(focus) => self.focus = focus,
            Command::ToggleFocus => self.toggle_focus(),
            Command::ToggleStatsPie => self.stats_by_app = !self.stats_by_app,
            Command::Select { app } => self.select_by_name(&app)?,
            Command::Stats { app } => self.show_stats(app.as_deref())?,
            Command::Help { command: None } => self.screen = Screen::Help,
            Command::Help {
                command: Some(name),
            } => return self.help(&name),
            Command::Quit => self.should_quit = true,

            // apps and categories
            Command::OpenForm(kind) => self.open_form(kind)?,
            Command::Launch { app } => return self.launch(&app),
            Command::Add {
                name,
                target,
                category,
                watch_exe,
                args,
            } => return self.add_app(&name, target, category, watch_exe, args),
            Command::Edit {
                app,
                name,
                target,
                category,
                watch_exe,
                args,
            } => return self.edit_app(&app, &name, target, &category, watch_exe, args),
            Command::Move { app, category } => return self.move_app(&app, &category),
            Command::Pin { change, app } => return self.pin_command(change, app.as_deref()),
            Command::LaunchPin { slot } => return self.launch_pin(slot),
            Command::RemoveApp { app, confirmed } => return self.remove_app(&app, confirmed),
            Command::RemoveCategory {
                category,
                confirmed,
            } => return self.remove_category(&category, confirmed),

            // Storage and Optimization screens
            Command::CycleDisk { forward } => self.cycle_disk(forward),
            Command::ToggleStorageOrder => self.toggle_storage_order(),
            Command::ToggleFolders => self.toggle_folders(),
            Command::OpenFolder => self.open_selected_folder(),
            Command::ParentFolder => self.parent_folder(),
            Command::RefreshStorage => self.refresh_storage(),
            Command::Trash { path, confirmed } => return self.trash(path, confirmed),
            Command::Uninstall { program, confirmed } => {
                return self.uninstall(&program, confirmed);
            }
            Command::ToggleBenchLevel => self.optimize.heavy = !self.optimize.heavy,
            Command::Bench(bench) => self.bench(bench)?,
            Command::ToggleGaming(setting) => return self.toggle_gaming(setting),
            Command::OpenGamingPage(setting) => return optimize::open_gaming_page(setting),

            // settings and data
            Command::ClearSessions { confirmed } => return self.clear_sessions(confirmed),
            Command::ClearStats { confirmed } => return self.clear_stats(confirmed),
            Command::Theme { name } => return self.theme_command(name.as_deref()),
            Command::Sort { by } => return self.sort_command(by),
            Command::Lang { code } => return self.lang_command(code.as_deref()),
            Command::Goal {
                kind,
                change,
                target,
            } => return self.goal_command(kind, change, target.as_deref()),
            Command::Group { name, apps } => return self.save_group(&name, &apps),
            Command::Export { path, confirmed } => return self.export(path, confirmed),
            Command::Import { path, confirmed } => return self.import(path, confirmed),
            Command::Update => return self.start_update(),
        }
        Ok(None)
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

    /// Tab in the command line: completes commands, apps and categories.
    pub(super) fn complete_command(&mut self, forward: bool) {
        let themes = theme::available(self.themes_dir.as_deref());
        let sources = Sources {
            themes: themes.iter().map(String::as_str).collect(),
            aliases: self.aliases.names().collect(),
            apps: self.apps.iter().map(|a| a.name.as_str()).collect(),
            categories: self.categories.iter().map(|c| c.name.as_str()).collect(),
            programs: self
                .storage
                .programs
                .iter()
                .map(|p| p.name.as_str())
                .collect(),
        };
        self.command_line
            .complete(|text| complete::complete(text, &sources), forward);
    }

    fn help(&self, name: &str) -> Outcome {
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

    pub(super) fn show(&mut self, screen: Screen) {
        self.screen = screen;
        match screen {
            Screen::Storage => self.start_storage_scan(),
            Screen::Optimize => self.optimize.open(),
            _ => {}
        }
    }

    /// `:stats [app]`: the Stats screen, for one app or every app.
    fn show_stats(&mut self, app: Option<&str>) -> anyhow::Result<()> {
        self.stats_app = match app {
            Some(app) => Some(self.app_named(app)?.id),
            None => None,
        };
        self.stats = self.db.stats(self.stats_app)?;
        self.stats_state =
            TableState::default().with_selected((!self.stats.sessions.is_empty()).then_some(0));
        self.screen = Screen::Stats;
        Ok(())
    }

    fn toggle_focus(&mut self) {
        if self.screen == Screen::Optimize {
            self.optimize.gaming_focus = !self.optimize.gaming_focus;
            return;
        }
        self.focus = match self.focus {
            Focus::Categories => Focus::Apps,
            Focus::Apps => Focus::Categories,
        };
    }

    /// Down/Up: moves the selection of the current screen's list.
    pub(super) fn move_selection(&mut self, forward: bool) {
        match self.screen {
            Screen::Rewards => {
                let next = step(self.reward_state.selected(), self.rewards.len(), forward);
                self.reward_state.select(next);
            }
            Screen::Stats => {
                let len = self.stats.sessions.len();
                let next = step(self.stats_state.selected(), len, forward);
                self.stats_state.select(next);
            }
            Screen::Storage => self.storage.move_selection(forward),
            Screen::Optimize => self.optimize.move_selection(forward),
            Screen::Dashboard | Screen::Help => match self.focus {
                Focus::Categories => {
                    let next = step(self.cat_state.selected(), self.category_rows(), forward);
                    if next != self.cat_state.selected() {
                        self.cat_state.select(next);
                        self.reset_app_selection();
                    }
                }
                Focus::Apps => {
                    let len = self.visible_apps().len();
                    self.app_state
                        .select(step(self.app_state.selected(), len, forward));
                }
            },
        }
    }
}
