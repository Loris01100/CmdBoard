//! Text typed after `:` -> `Command`.

use super::{Command, find_help};
use crate::app::AppSort;
use crate::popup::FormKind;

pub fn parse(input: &str) -> Result<Command, String> {
    let args = split_args(input)?;
    let Some((name, rest)) = args.split_first() else {
        return Err(t!("parse.empty"));
    };
    let help = find_help(name).ok_or_else(|| t!("parse.unknown", name))?;

    match (help.name, rest) {
        // Single-argument commands take the rest of the line, so names need no quotes.
        ("launch", [_, ..]) => Ok(Command::Launch {
            app: rest.join(" "),
        }),
        ("rm", [_, ..]) => Ok(Command::RemoveApp {
            app: rest.join(" "),
            confirmed: false,
        }),
        ("rmcat", [_, ..]) => Ok(Command::RemoveCategory {
            category: rest.join(" "),
            confirmed: false,
        }),
        ("add", []) => Ok(Command::OpenForm(FormKind::Add)),
        ("add", [name, target]) => Ok(Command::Add {
            name: name.clone(),
            target: target.clone(),
            category: None,
            watch_exe: None,
        }),
        ("add", [name, target, category]) => Ok(Command::Add {
            name: name.clone(),
            target: target.clone(),
            category: Some(category.clone()),
            watch_exe: None,
        }),
        ("move", [app]) => Ok(Command::OpenForm(FormKind::Move { app: app.clone() })),
        ("move", [app, category]) => Ok(Command::Move {
            app: app.clone(),
            category: category.clone(),
        }),
        ("edit", [_, ..]) => Ok(Command::OpenForm(FormKind::Edit {
            app: rest.join(" "),
        })),
        ("stats", []) => Ok(Command::Stats { app: None }),
        ("stats", [_, ..]) => Ok(Command::Stats {
            app: Some(rest.join(" ")),
        }),
        ("clear", [what]) if what.eq_ignore_ascii_case("sessions") => {
            Ok(Command::ClearSessions { confirmed: false })
        }
        ("clear", [what]) if what.eq_ignore_ascii_case("stats") => {
            Ok(Command::ClearStats { confirmed: false })
        }
        ("theme", []) => Ok(Command::Theme { name: None }),
        ("theme", [name]) => Ok(Command::Theme {
            name: Some(name.clone()),
        }),
        ("sort", []) => Ok(Command::Sort { by: None }),
        ("sort", [name]) => match AppSort::parse(name) {
            Some(sort) => Ok(Command::Sort { by: Some(sort) }),
            None => Err(t!("parse.bad_sort", name, usage = help.usage())),
        },
        ("lang", []) => Ok(Command::Lang { code: None }),
        ("lang", [code]) => Ok(Command::Lang {
            code: Some(code.clone()),
        }),
        ("group", [name, apps @ ..]) => {
            group(name, apps).ok_or_else(|| t!("parse.usage", usage = help.usage()))
        }
        ("export", []) => Ok(Command::Export {
            path: None,
            confirmed: false,
        }),
        ("export", [_, ..]) => Ok(Command::Export {
            path: Some(rest.join(" ")),
            confirmed: false,
        }),
        ("import", [_, ..]) => Ok(Command::Import {
            path: rest.join(" "),
            confirmed: false,
        }),
        ("uninstall", [_, ..]) => Ok(Command::Uninstall {
            program: rest.join(" "),
            confirmed: false,
        }),
        ("update", []) => Ok(Command::Update),
        ("help", []) => Ok(Command::Help { command: None }),
        ("help", [command]) => Ok(Command::Help {
            command: Some(command.clone()),
        }),
        ("quit", []) => Ok(Command::Quit),
        _ => Err(t!("parse.usage", usage = help.usage())),
    }
}

/// `:group name app, app…`: apps are comma-separated, so their names need no quotes.
/// `None` without any app.
fn group(name: &str, apps: &[String]) -> Option<Command> {
    let apps: Vec<String> = apps
        .join(" ")
        .split(',')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(String::from)
        .collect();
    (!apps.is_empty()).then(|| Command::Group {
        name: name.into(),
        apps,
    })
}

/// Splits on whitespace; double quotes group words. Backslashes are kept as-is,
/// unlike a POSIX shell, so Windows paths need no escaping.
pub(super) fn split_args(input: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_arg = false;
    let mut quoted = false;
    for c in input.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                in_arg = true;
            }
            c if c.is_whitespace() && !quoted => {
                if in_arg {
                    args.push(std::mem::take(&mut current));
                    in_arg = false;
                }
            }
            c => {
                current.push(c);
                in_arg = true;
            }
        }
    }
    if quoted {
        return Err(t!("parse.unclosed_quote"));
    }
    if in_arg {
        args.push(current);
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> String {
        v.to_string()
    }

    #[test]
    #[expect(clippy::too_many_lines, reason = "table-driven: one case per command")]
    fn parses_valid_commands() {
        let cases = [
            ("launch Hades", Command::Launch { app: s("Hades") }),
            (
                "l windows terminal",
                Command::Launch {
                    app: s("windows terminal"),
                },
            ),
            ("LAUNCH  Hades ", Command::Launch { app: s("Hades") }),
            (
                r#"add Code "C:\Program Files\VS Code\Code.exe""#,
                Command::Add {
                    name: s("Code"),
                    target: s(r"C:\Program Files\VS Code\Code.exe"),
                    category: None,
                    watch_exe: None,
                },
            ),
            (
                r#"add "Hollow Knight" steam://rungameid/367520 Jeux"#,
                Command::Add {
                    name: s("Hollow Knight"),
                    target: s("steam://rungameid/367520"),
                    category: Some(s("Jeux")),
                    watch_exe: None,
                },
            ),
            (
                r#"mv "Bloc-notes" Dev"#,
                Command::Move {
                    app: s("Bloc-notes"),
                    category: s("Dev"),
                },
            ),
            ("add", Command::OpenForm(FormKind::Add)),
            (
                "edit Windows Terminal",
                Command::OpenForm(FormKind::Edit {
                    app: s("Windows Terminal"),
                }),
            ),
            (
                "move Hades",
                Command::OpenForm(FormKind::Move { app: s("Hades") }),
            ),
            (
                "rm Windows Terminal",
                Command::RemoveApp {
                    app: s("Windows Terminal"),
                    confirmed: false,
                },
            ),
            (
                "rmcat Jeux",
                Command::RemoveCategory {
                    category: s("Jeux"),
                    confirmed: false,
                },
            ),
            ("stats", Command::Stats { app: None }),
            (
                "stats Windows Terminal",
                Command::Stats {
                    app: Some(s("Windows Terminal")),
                },
            ),
            (
                "clear sessions",
                Command::ClearSessions { confirmed: false },
            ),
            ("CLEAR Stats", Command::ClearStats { confirmed: false }),
            ("theme", Command::Theme { name: None }),
            (
                "theme catppuccin-latte",
                Command::Theme {
                    name: Some(s("catppuccin-latte")),
                },
            ),
            ("sort", Command::Sort { by: None }),
            (
                "sort XP",
                Command::Sort {
                    by: Some(AppSort::Xp),
                },
            ),
            ("lang", Command::Lang { code: None }),
            (
                "lang EN",
                Command::Lang {
                    code: Some(s("EN")),
                },
            ),
            (
                "group streaming OBS, Windows Terminal ,Spotify,",
                Command::Group {
                    name: s("streaming"),
                    apps: vec![s("OBS"), s("Windows Terminal"), s("Spotify")],
                },
            ),
            (
                "export",
                Command::Export {
                    path: None,
                    confirmed: false,
                },
            ),
            (
                r"export D:\Mes sauvegardes\cmdboard.json",
                Command::Export {
                    path: Some(s(r"D:\Mes sauvegardes\cmdboard.json")),
                    confirmed: false,
                },
            ),
            (
                r#"import "C:\Users\Me\cmdboard.json""#,
                Command::Import {
                    path: s(r"C:\Users\Me\cmdboard.json"),
                    confirmed: false,
                },
            ),
            ("update", Command::Update),
            ("help", Command::Help { command: None }),
            (
                "uninstall Visual Studio Code",
                Command::Uninstall {
                    program: s("Visual Studio Code"),
                    confirmed: false,
                },
            ),
            (
                "? add",
                Command::Help {
                    command: Some(s("add")),
                },
            ),
            ("q", Command::Quit),
        ];
        for (input, expected) in cases {
            assert_eq!(parse(input), Ok(expected), "input: {input}");
        }
    }

    #[test]
    fn rejects_invalid_commands() {
        let cases = [
            ("", "Commande vide"),
            ("   ", "Commande vide"),
            ("fly away", "Commande inconnue : fly (:help pour la liste)"),
            ("launch", "Usage : launch <app>"),
            ("add Code", "Usage : add [<nom> <cible> [catégorie]]"),
            ("move", "Usage : move <app> [catégorie]"),
            ("rm", "Usage : rm <app>"),
            ("edit", "Usage : edit <app>"),
            ("quit now", "Usage : quit"),
            ("clear", "Usage : clear sessions|stats"),
            ("clear apps", "Usage : clear sessions|stats"),
            ("update now", "Usage : update"),
            ("import", "Usage : import <fichier>"),
            ("group streaming", "Usage : group <nom> <app>, <app>…"),
            ("group streaming ,", "Usage : group <nom> <app>, <app>…"),
            (
                "sort size",
                "Tri inconnu : size (usage : sort [name|xp|recent|time])",
            ),
            (r#"launch "Hades"#, "Guillemet non fermé"),
        ];
        for (input, expected) in cases {
            assert_eq!(parse(input), Err(s(expected)), "input: {input}");
        }
    }

    #[test]
    fn split_keeps_backslashes_and_empty_quotes() {
        assert_eq!(
            split_args(r#"a C:\x\y.exe "" "b c""#).unwrap(),
            [s("a"), s(r"C:\x\y.exe"), s(""), s("b c")]
        );
    }

    proptest::proptest! {
        /// Any argument without a double quote survives being quoted: spaces,
        /// backslashes, empty text, accents.
        #[test]
        fn quoted_args_round_trip(args in proptest::collection::vec(r#"[^"]*"#, 0..6)) {
            let line: Vec<String> = args.iter().map(|a| format!("\"{a}\"")).collect();
            proptest::prop_assert_eq!(split_args(&line.join(" ")).unwrap(), args);
        }

        /// A Windows path without spaces needs no quotes and keeps every backslash.
        #[test]
        fn launch_keeps_windows_paths(path in r"[A-Z]:(\\[\w.\-]{1,12}){0,5}") {
            let command = parse(&format!("launch {path}")).unwrap();
            proptest::prop_assert_eq!(command, Command::Launch { app: path });
        }

        /// Whatever is typed, the parser answers with a command or an error.
        #[test]
        fn never_panics(input in r#"[a-z :,"\\ ]{0,40}|\PC{0,40}"#) {
            let _ = parse(&input);
        }
    }
}
