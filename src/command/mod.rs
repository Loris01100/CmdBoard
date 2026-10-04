pub mod line;
pub mod parser;

use crate::app::{Focus, Screen};
use crate::popup::FormKind;

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
    /// Adds (or removes, if negative) XP to an app by hand, outside of any session.
    Xp {
        app: String,
        amount: i64,
    },
    Help {
        command: Option<String>,
    },
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
        usage: "add [<nom> <cible> [catégorie]]",
        summary: "Ajoute une app (exe ou URI). Sans argument : ouvre le formulaire",
    },
    CommandHelp {
        name: "move",
        aliases: &["mv"],
        usage: "move <app> [catégorie]",
        summary: "Déplace une app. Sans catégorie : ouvre le formulaire",
    },
    CommandHelp {
        name: "rm",
        aliases: &["delete"],
        usage: "rm <app>",
        summary: "Supprime une app et son historique, après confirmation",
    },
    CommandHelp {
        name: "rmcat",
        aliases: &[],
        usage: "rmcat <catégorie>",
        summary: "Supprime une catégorie vide, après confirmation",
    },
    CommandHelp {
        name: "xp",
        aliases: &[],
        usage: "xp <app> <montant>",
        summary: "Ajoute (ou retire, si négatif) de l'XP à une app",
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
