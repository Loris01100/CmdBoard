pub mod alias;
pub mod complete;
pub mod line;
pub mod parser;

use crate::app::{AppSort, Focus, Screen};
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
    },
    Move {
        app: String,
        category: String,
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
    /// Adds (or removes, if negative) XP to an app by hand, outside of any session.
    Xp {
        app: String,
        amount: i64,
    },
    /// Writes apps and sessions to a JSON file; `None` picks a dated file in Documents.
    Export {
        path: Option<String>,
    },
    /// Merges a file written by `Export`: adds what is missing, never deletes.
    Import {
        path: String,
    },
    /// Installs the latest GitHub release, or points to winget for MSI installs.
    Update,
    Help {
        command: Option<String>,
    },
    Quit,
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
        name: "xp",
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
