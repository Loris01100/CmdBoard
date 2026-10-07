//! Tab completion for the `:` command line: command names, then app or category names
//! depending on the command and the argument being typed. Candidates are fuzzy-ranked.

use super::{COMMANDS, find_help};
use crate::app::AppSort;
use crate::fuzzy;

/// Names that can be completed, besides the built-in commands.
#[derive(Debug, Default)]
pub struct Sources<'a> {
    pub aliases: Vec<&'a str>,
    pub apps: Vec<&'a str>,
    pub categories: Vec<&'a str>,
    /// Installed programs, for `:uninstall`.
    pub programs: Vec<&'a str>,
    pub themes: Vec<&'a str>,
}

/// The line splits into `base`, kept as typed, followed by one of the `candidates`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub base: String,
    pub candidates: Vec<String>,
}

impl Completion {
    pub fn line(&self, index: usize) -> String {
        format!("{}{}", self.base, self.candidates[index])
    }
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    Command,
    App,
    Category,
    Program,
    Theme,
    Sort,
    Lang,
}

/// Completes the last word of `line`. `None` when nothing applies or nothing matches.
pub fn complete(line: &str, sources: &Sources) -> Option<Completion> {
    let lead = line.len() - line.trim_start().len();
    let body = &line[lead..];
    let Some(space) = body.find(char::is_whitespace) else {
        // Still typing the command name: complete it and add a space for the arguments.
        return build(&line[..lead], body, Kind::Command, sources, " ");
    };
    let command = find_help(&body[..space])?.name;
    let after = &body[space..];
    let rest_start = lead + space + (after.len() - after.trim_start().len());

    match command {
        // These take the rest of the line as one name: no quotes needed.
        "launch" | "rm" | "stats" | "xp" => whole(line, rest_start, Kind::App, sources),
        "rmcat" => whole(line, rest_start, Kind::Category, sources),
        "uninstall" => whole(line, rest_start, Kind::Program, sources),
        "help" => whole(line, rest_start, Kind::Command, sources),
        "theme" => whole(line, rest_start, Kind::Theme, sources),
        "sort" => whole(line, rest_start, Kind::Sort, sources),
        "lang" => whole(line, rest_start, Kind::Lang, sources),
        "move" => argument(
            line,
            rest_start,
            &[Some(Kind::App), Some(Kind::Category)],
            sources,
        ),
        "add" => argument(
            line,
            rest_start,
            &[None, None, Some(Kind::Category)],
            sources,
        ),
        _ => None,
    }
}

fn whole(line: &str, start: usize, kind: Kind, sources: &Sources) -> Option<Completion> {
    build(&line[..start], &line[start..], kind, sources, "")
}

/// Completes the argument at the end of the line, if its position has a known kind.
/// Names holding spaces are quoted.
fn argument(
    line: &str,
    rest_start: usize,
    kinds: &[Option<Kind>],
    sources: &Sources,
) -> Option<Completion> {
    let rest = &line[rest_start..];
    let tokens = token_starts(rest);
    let (index, start) = if rest.is_empty() || rest.ends_with(char::is_whitespace) {
        (tokens.len(), line.len())
    } else {
        (tokens.len() - 1, rest_start + *tokens.last()?)
    };
    let kind = (*kinds.get(index)?)?;
    let partial = line[start..].trim_matches('"');
    let mut completion = build(&line[..start], partial, kind, sources, "")?;
    for candidate in &mut completion.candidates {
        if candidate.contains(char::is_whitespace) {
            *candidate = format!("\"{candidate}\"");
        }
    }
    Some(completion)
}

/// Byte offset where each argument starts; double quotes group words, like the parser.
fn token_starts(text: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let (mut in_token, mut quoted) = (false, false);
    for (i, c) in text.char_indices() {
        if c.is_whitespace() && !quoted {
            in_token = false;
            continue;
        }
        if !in_token {
            starts.push(i);
            in_token = true;
        }
        if c == '"' {
            quoted = !quoted;
        }
    }
    starts
}

fn build(
    base: &str,
    partial: &str,
    kind: Kind,
    sources: &Sources,
    suffix: &str,
) -> Option<Completion> {
    let mut items: Vec<&str> = match kind {
        Kind::Command => COMMANDS
            .iter()
            .map(|c| c.name)
            .chain(sources.aliases.iter().copied())
            .collect(),
        Kind::App => sources.apps.clone(),
        Kind::Category => sources.categories.clone(),
        Kind::Program => sources.programs.clone(),
        Kind::Theme => sources.themes.clone(),
        Kind::Sort => AppSort::ALL.iter().map(|s| s.name()).collect(),
        Kind::Lang => crate::i18n::LANGS.iter().map(|(code, _)| *code).collect(),
    };
    items.sort_by_key(|item| item.to_lowercase());
    let candidates: Vec<String> = fuzzy::rank(partial, items.iter().copied())
        .into_iter()
        .map(|i| format!("{}{suffix}", items[i]))
        .collect();
    (!candidates.is_empty()).then(|| Completion {
        base: base.to_string(),
        candidates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources() -> Sources<'static> {
        Sources {
            aliases: vec!["gaming"],
            apps: vec!["Steam", "Windows Terminal", "Bloc-notes"],
            categories: vec!["Jeux", "Dev", "Outils"],
            programs: vec!["Visual Studio Code", "7-Zip"],
            themes: vec!["catppuccin-latte", "catppuccin-mocha", "terminal"],
        }
    }

    /// First candidate's full line, or `None`.
    fn first(line: &str) -> Option<String> {
        complete(line, &sources()).map(|c| c.line(0))
    }

    #[test]
    fn completes_by_position() {
        let cases = [
            ("lau", Some("launch ")),
            ("lan", Some("lang ")),
            ("lang f", Some("lang fr")),
            ("gam", Some("gaming ")),
            ("launch ste", Some("launch Steam")),
            ("l wterm", Some("l Windows Terminal")), // alias of launch, fuzzy match
            ("rmcat ou", Some("rmcat Outils")),
            ("uninstall vsc", Some("uninstall Visual Studio Code")),
            ("help mo", Some("help move")),
            ("theme moc", Some("theme catppuccin-mocha")),
            ("sort rec", Some("sort recent")),
            ("move wind", Some(r#"move "Windows Terminal""#)),
            (
                r#"move "Windows Terminal" d"#,
                Some(r#"move "Windows Terminal" Dev"#),
            ),
            ("add Hades steam://x je", Some("add Hades steam://x Jeux")),
            ("add Had", None), // the name is free text
            ("quit x", None),
            ("fly x", None),
            ("launch zzz", None),
        ];
        for (line, expected) in cases {
            assert_eq!(first(line).as_deref(), expected, "line: {line}");
        }
    }

    #[test]
    fn empty_argument_lists_everything_sorted() {
        let completion = complete("move Steam ", &sources()).unwrap();
        assert_eq!(completion.base, "move Steam ");
        assert_eq!(completion.candidates, ["Dev", "Jeux", "Outils"]);
    }

    #[test]
    fn token_starts_respect_quotes() {
        assert_eq!(token_starts(r#"a "b c" d"#), [0, 2, 8]);
        assert_eq!(token_starts("  "), Vec::<usize>::new());
    }
}
