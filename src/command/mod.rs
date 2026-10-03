pub mod line;
pub mod parser;

use crate::app::{Focus, Screen};

/// Every user action. Keys and the `:` command line (and later aliases) are translated
/// into a `Command`, then run by `App::execute`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    // navigation, bound to keys
    Show(Screen),
    SelectNext,
    SelectPrev,
    FocusPanel(Focus),
    ToggleFocus,

    // actions, also available from the command line
    Launch { app: String },
    Add { name: String, target: String, category: Option<String> },
    Move { app: String, category: String },
    Help { command: Option<String> },
    Quit,
}

/// Command line reference, used by `:help` and the Help screen.
pub struct CommandHelp {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub usage: &'static str,
    pub summary: &'static str,
}

pub const COMMANDS: &[CommandHelp] = &[
    CommandHelp {
        name: "launch",
        aliases: &["l"],
        usage: "launch <app>",
        summary: "Lance une app",
    },
    CommandHelp {
        name: "add",
        aliases: &[],
        usage: "add <nom> <cible> [catégorie]",
        summary: "Ajoute une app (exe ou URI) dans la catégorie donnée, sinon la catégorie sélectionnée",
    },
    CommandHelp {
        name: "move",
        aliases: &["mv"],
        usage: "move <app> <catégorie>",
        summary: "Déplace une app dans une autre catégorie",
    },
    CommandHelp {
        name: "help",
        aliases: &["h", "?"],
        usage: "help [commande]",
        summary: "Affiche l'aide, ou l'usage d'une commande",
    },
    CommandHelp {
        name: "quit",
        aliases: &["q"],
        usage: "quit",
        summary: "Quitte CmdBoard",
    },
];

/// Looks a command up by name or alias, ignoring case.
pub fn find_help(name: &str) -> Option<&'static CommandHelp> {
    let name = name.to_lowercase();
    COMMANDS
        .iter()
        .find(|c| c.name == name || c.aliases.contains(&name.as_str()))
}
