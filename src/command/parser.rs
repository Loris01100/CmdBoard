//! Text typed after `:` -> `Command`.

use super::{Command, find_help};

pub fn parse(input: &str) -> Result<Command, String> {
    let args = split_args(input)?;
    let Some((name, rest)) = args.split_first() else {
        return Err("Commande vide".into());
    };
    let help = find_help(name)
        .ok_or_else(|| format!("Commande inconnue : {name} (:help pour la liste)"))?;

    match (help.name, rest) {
        // `launch` takes the rest of the line, so names with spaces need no quotes.
        ("launch", [_, ..]) => Ok(Command::Launch { app: rest.join(" ") }),
        ("add", [name, target]) => Ok(Command::Add {
            name: name.clone(),
            target: target.clone(),
            category: None,
        }),
        ("add", [name, target, category]) => Ok(Command::Add {
            name: name.clone(),
            target: target.clone(),
            category: Some(category.clone()),
        }),
        ("move", [app, category]) => Ok(Command::Move {
            app: app.clone(),
            category: category.clone(),
        }),
        ("help", []) => Ok(Command::Help { command: None }),
        ("help", [command]) => Ok(Command::Help { command: Some(command.clone()) }),
        ("quit", []) => Ok(Command::Quit),
        _ => Err(format!("Usage : {}", help.usage)),
    }
}

/// Splits on whitespace; double quotes group words. Backslashes are kept as-is,
/// unlike a POSIX shell, so Windows paths need no escaping.
fn split_args(input: &str) -> Result<Vec<String>, String> {
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
        return Err("Guillemet non fermé".into());
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
    fn parses_valid_commands() {
        let cases = [
            ("launch Hades", Command::Launch { app: s("Hades") }),
            ("l windows terminal", Command::Launch { app: s("windows terminal") }),
            ("LAUNCH  Hades ", Command::Launch { app: s("Hades") }),
            (
                r#"add Code "C:\Program Files\VS Code\Code.exe""#,
                Command::Add {
                    name: s("Code"),
                    target: s(r"C:\Program Files\VS Code\Code.exe"),
                    category: None,
                },
            ),
            (
                r#"add "Hollow Knight" steam://rungameid/367520 Jeux"#,
                Command::Add {
                    name: s("Hollow Knight"),
                    target: s("steam://rungameid/367520"),
                    category: Some(s("Jeux")),
                },
            ),
            (
                r#"mv "Bloc-notes" Dev"#,
                Command::Move { app: s("Bloc-notes"), category: s("Dev") },
            ),
            ("help", Command::Help { command: None }),
            ("? add", Command::Help { command: Some(s("add")) }),
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
            ("add Code", "Usage : add <nom> <cible> [catégorie]"),
            ("move Hades", "Usage : move <app> <catégorie>"),
            ("quit now", "Usage : quit"),
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
}
