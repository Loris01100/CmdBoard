pub mod alias;
pub mod complete;
pub mod line;
pub mod parser;

use crate::app::{Focus, Screen};
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
    /// Adds (or removes, if negative) XP to an app by hand, outside of any session.
    Xp {
        app: String,
        amount: i64,
    },
    /// Installs the latest GitHub release, or points to winget for MSI installs.
    Update,
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
        name: "stats",
        aliases: &[],
        usage: "stats [app]",
        summary: "Statistiques de toutes les apps, ou d'une seule",
    },
    CommandHelp {
        name: "theme",
        aliases: &[],
        usage: "theme [nom]",
        summary: "Change de thème (mémorisé). Sans nom : liste les thèmes",
    },
    CommandHelp {
        name: "xp",
        aliases: &[],
        usage: "xp <app> <montant>",
        summary: "Ajoute (ou retire, si négatif) de l'XP à une app",
    },
    CommandHelp {
        name: "update",
        aliases: &[],
        usage: "update",
        summary: "Installe la dernière version (MSI/winget : winget upgrade CmdBoard)",
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
