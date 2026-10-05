//! User aliases from `%APPDATA%\CmdBoard\commands.toml`:
//!
//! ```toml
//! [alias]
//! gaming = "launch steam; launch discord"
//! jouer  = "launch $1; stats $1"
//! ```
//!
//! An alias expands to one or more command lines (split on `;`), run one after the other.
//! `$1`…`$9` are replaced by its arguments, `$*` by all of them.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use super::{find_help, parser::split_args};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Aliases {
    /// Lowercase name -> body.
    map: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    alias: BTreeMap<String, String>,
}

impl Aliases {
    /// Reads `path`. A missing file means no aliases. Returns the aliases that could be
    /// loaded and, if something was wrong, a message to show.
    pub fn load(path: &Path) -> (Self, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match Self::parse(&text) {
                Ok(result) => result,
                Err(e) => (Self::default(), Some(format!("commands.toml : {e}"))),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(e) => (Self::default(), Some(format!("commands.toml : {e}"))),
        }
    }

    /// Parses the TOML. Aliases named like a built-in command, or not a single word,
    /// are skipped and reported in the returned message.
    pub fn parse(text: &str) -> Result<(Self, Option<String>), String> {
        let file: File = toml::from_str(text).map_err(|e| e.message().to_string())?;
        let mut map = BTreeMap::new();
        let mut skipped = Vec::new();
        for (name, body) in file.alias {
            let lower = name.to_lowercase();
            if find_help(&lower).is_some()
                || lower.is_empty()
                || lower.contains(char::is_whitespace)
            {
                skipped.push(name);
            } else {
                map.insert(lower, body);
            }
        }
        let warning = (!skipped.is_empty())
            .then(|| format!("commands.toml : alias ignoré(s) : {}", skipped.join(", ")));
        Ok((Self { map }, warning))
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.map.keys().map(String::as_str)
    }

    /// `(name, body)` pairs, sorted by name.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.map.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.map.get(&name.to_lowercase()).map(String::as_str)
    }

    /// If `line` starts with an alias, its expansion into command lines.
    /// `None` when the first word is not an alias.
    pub fn expand(&self, line: &str) -> Option<Result<Vec<String>, String>> {
        let args = match split_args(line) {
            Ok(args) => args,
            Err(e) => return Some(Err(e)),
        };
        let (name, rest) = args.split_first()?;
        let body = self.get(name)?;
        Some(substitute(body, rest).map(|body| {
            body.split(';')
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect()
        }))
    }
}

/// Replaces `$1`…`$9` and `$*`. Arguments holding spaces are quoted back.
fn substitute(body: &str, args: &[String]) -> Result<String, String> {
    let quote = |a: &String| {
        if a.contains(char::is_whitespace) {
            format!("\"{a}\"")
        } else {
            a.clone()
        }
    };
    let mut out = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('*') => {
                chars.next();
                out.push_str(&args.iter().map(quote).collect::<Vec<_>>().join(" "));
            }
            Some(d @ '1'..='9') => {
                chars.next();
                let i = d.to_digit(10).unwrap_or(1) as usize;
                let arg = args
                    .get(i - 1)
                    .ok_or_else(|| format!("argument ${i} manquant"))?;
                out.push_str(&quote(arg));
            }
            _ => out.push('$'),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aliases() -> Aliases {
        Aliases::parse(
            r#"
            [alias]
            gaming = "launch steam; launch discord"
            Jouer  = "launch $1; stats $1"
            tout   = "xp $*"
            "#,
        )
        .unwrap()
        .0
    }

    #[test]
    fn expands_sequences_and_arguments() {
        let a = aliases();
        assert_eq!(
            a.expand("gaming"),
            Some(Ok(vec!["launch steam".into(), "launch discord".into()]))
        );
        assert_eq!(
            a.expand(r#"JOUER "Hollow Knight""#),
            Some(Ok(vec![
                r#"launch "Hollow Knight""#.into(),
                r#"stats "Hollow Knight""#.into()
            ]))
        );
        assert_eq!(
            a.expand("tout Steam 50"),
            Some(Ok(vec!["xp Steam 50".into()]))
        );
        assert_eq!(a.expand("jouer"), Some(Err("argument $1 manquant".into())));
        assert_eq!(a.expand("launch steam"), None);
        assert_eq!(a.expand(""), None);
    }

    #[test]
    fn builtin_names_are_reserved() {
        let (a, warning) =
            Aliases::parse("[alias]\nhelp = \"quit\"\nq = \"quit\"\nok = \"help\"").unwrap();
        assert_eq!(a.names().collect::<Vec<_>>(), ["ok"]);
        assert_eq!(
            warning.as_deref(),
            Some("commands.toml : alias ignoré(s) : help, q")
        );
    }

    #[test]
    fn broken_or_missing_file() {
        assert!(Aliases::parse("[alias\n").is_err());
        assert_eq!(Aliases::parse("").unwrap().0, Aliases::default());
        let (a, warning) =
            Aliases::load(&std::env::temp_dir().join("cmdboard-no-such-dir/commands.toml"));
        assert_eq!((a, warning), (Aliases::default(), None));
    }
}
