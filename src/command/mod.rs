pub mod alias;
pub mod complete;
pub mod line;
pub mod parser;

use crate::app::{AppSort, Focus, Screen};
use crate::core::goals::{GoalKind, Period};
use crate::popup::FormKind;

/// Every user action. Keys, the `:` command line and aliases are translated
/// into a `Command`, then run by `App::execute`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    // navigation, bound to keys
    Show(Screen),
    SelectNext,
    SelectPrev,
    FocusPanel(Focus),
    ToggleFocus,
    /// Stats screen: pie of the time per category, or per app.
    ToggleStatsPie,
    /// Storage screen: programs of the next (or previous) disk, then of every disk.
    CycleDisk {
        forward: bool,
    },
    /// Storage screen: biggest programs first, or smallest first.
    ToggleStorageOrder,
    /// Storage screen: folder browser instead of the programs, or back.
    ToggleFolders,
    /// Folder browser: opens the selected folder and measures what it holds.
    OpenFolder,
    /// Folder browser: back to the parent folder, then to the drives.
    ParentFolder,
    /// Storage screen: reads the drives, the programs and the shown folder again.
    RefreshStorage,
    /// Destructive: sends a file or folder to the Recycle Bin, after confirmation.
    Trash {
        path: std::path::PathBuf,
        confirmed: bool,
    },

    /// Optimization screen: runs a benchmark at the current level, in a thread.
    Bench(crate::optimize::Bench),
    /// Optimization screen: light or heavy benchmarks.
    ToggleBenchLevel,
    /// Optimization screen: switches a gaming setting, or opens its Windows page.
    ToggleGaming(crate::optimize::Gaming),
    /// Optimization screen: opens the Windows page of a gaming setting.
    OpenGamingPage(crate::optimize::Gaming),

    // actions, also available from the command line
    Launch {
        app: String,
    },
    /// `watch_exe: None` derives the process from the target when it is an exe.
    Add {
        name: String,
        target: String,
        category: Option<String>,
        watch_exe: Option<String>,
        /// Arguments passed to the target, `None` for none.
        args: Option<String>,
    },
    Move {
        app: String,
        category: String,
    },
    /// Replaces an app's details, keeping its history. `watch_exe: None` derives the
    /// process from the target when it is an exe, like `Add`.
    Edit {
        app: String,
        name: String,
        target: String,
        category: String,
        watch_exe: Option<String>,
        args: Option<String>,
    },
    /// Destructive: without `confirmed`, opens a confirmation popup first.
    RemoveApp {
        app: String,
        confirmed: bool,
    },
    /// Only empty categories can be removed.
    RemoveCategory {
        category: String,
        confirmed: bool,
    },
    /// Destructive: starts the program's own uninstaller, after confirmation.
    Uninstall {
        program: String,
        confirmed: bool,
    },
    /// Destructive: hides finished sessions from the history; stats still count them.
    ClearSessions {
        confirmed: bool,
    },
    /// Destructive: deletes finished sessions, so stats start over. App XP and rewards stay.
    ClearStats {
        confirmed: bool,
    },
    OpenForm(FormKind),
    /// Selects an app in its category (Enter in the `/` search).
    Select {
        app: String,
    },
    /// Opens the Stats screen, for one app or (`None`) all of them.
    Stats {
        app: Option<String>,
    },
    /// Switches theme and remembers it; `None` lists the themes.
    Theme {
        name: Option<String>,
    },
    /// Orders the apps panel and remembers it; `None` lists the orders.
    Sort {
        by: Option<AppSort>,
    },
    /// Switches the UI language and remembers it; `None` lists the languages.
    Lang {
        code: Option<String>,
    },
    /// `:pin`: lists the favorites (`change: None`), or puts `app` in a slot or out of it.
    Pin {
        change: Option<PinChange>,
        app: Option<String>,
    },
    /// `Alt+1` to `Alt+9`: launches the app in that favorite slot.
    LaunchPin {
        slot: u8,
    },
    /// `:goal` or `:limit`: lists them (`change: None`), or sets or removes the one of
    /// `target`, an app or a category (`None`: every app together).
    Goal {
        kind: GoalKind,
        change: Option<GoalChange>,
        target: Option<String>,
    },
    /// Saves an alias launching these apps in order (`commands.toml`).
    Group {
        name: String,
        apps: Vec<String>,
    },
    /// Writes apps and sessions to a JSON file; `None` picks a dated file in Documents.
    /// Destructive when the file exists: without `confirmed`, asks before replacing it.
    Export {
        path: Option<String>,
        confirmed: bool,
    },
    /// Merges a file written by `Export`: adds what is missing, never deletes.
    /// Without `confirmed`, asks first when it adds apps, showing what they launch.
    Import {
        path: String,
        confirmed: bool,
    },
    /// Installs the latest GitHub release, or points to winget for MSI installs.
    Update,
    Help {
        command: Option<String>,
    },
    Quit,
}

/// What `:pin` does to an app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinChange {
    /// Slot 1 to 9, taken from the app that had it.
    Slot(u8),
    Off,
    /// `p` on the dashboard: out of its slot if it has one, else into the first free one.
    Toggle,
}

/// What `:goal` or `:limit` does to a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalChange {
    Set { minutes: u32, period: Period },
    Remove,
}

/// Command line reference, used by `:help` and the Help screen. Usage and summary are
/// `cmd.<name>.usage` and `cmd.<name>.summary` in the locale files.
pub struct CommandHelp {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
}

impl CommandHelp {
    pub fn usage(&self) -> String {
        crate::i18n::tr(&format!("cmd.{}.usage", self.name), &[])
    }

    pub fn summary(&self) -> String {
        crate::i18n::tr(&format!("cmd.{}.summary", self.name), &[])
    }
}

pub const COMMANDS: &[CommandHelp] = &[
    CommandHelp {
        name: "launch",
        aliases: &["l"],
    },
    CommandHelp {
        name: "add",
        aliases: &[],
    },
    CommandHelp {
        name: "move",
        aliases: &["mv"],
    },
    CommandHelp {
        name: "edit",
        aliases: &[],
    },
    CommandHelp {
        name: "rm",
        aliases: &["delete"],
    },
    CommandHelp {
        name: "rmcat",
        aliases: &[],
    },
    CommandHelp {
        name: "stats",
        aliases: &[],
    },
    CommandHelp {
        name: "clear",
        aliases: &[],
    },
    CommandHelp {
        name: "pin",
        aliases: &[],
    },
    CommandHelp {
        name: "goal",
        aliases: &[],
    },
    CommandHelp {
        name: "limit",
        aliases: &[],
    },
    CommandHelp {
        name: "theme",
        aliases: &[],
    },
    CommandHelp {
        name: "sort",
        aliases: &[],
    },
    CommandHelp {
        name: "lang",
        aliases: &[],
    },
    CommandHelp {
        name: "group",
        aliases: &[],
    },
    CommandHelp {
        name: "export",
        aliases: &[],
    },
    CommandHelp {
        name: "import",
        aliases: &[],
    },
    CommandHelp {
        name: "uninstall",
        aliases: &[],
    },
    CommandHelp {
        name: "update",
        aliases: &[],
    },
    CommandHelp {
        name: "help",
        aliases: &["h", "?"],
    },
    CommandHelp {
        name: "quit",
        aliases: &["q"],
    },
];

/// Looks a command up by name or alias, ignoring case.
pub fn find_help(name: &str) -> Option<&'static CommandHelp> {
    let name = name.to_lowercase();
    COMMANDS
        .iter()
        .find(|c| c.name == name || c.aliases.contains(&name.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_help_in_every_language() {
        for (code, _) in crate::i18n::LANGS {
            let text = crate::i18n::LANGS
                .iter()
                .find(|(c, _)| c == code)
                .unwrap()
                .1;
            let table: toml::Table = text.parse().unwrap();
            for command in COMMANDS {
                let entry = &table["cmd"][command.name];
                assert!(entry.get("usage").is_some(), "{code}: {}", command.name);
                assert!(entry.get("summary").is_some(), "{code}: {}", command.name);
            }
        }
    }
}
