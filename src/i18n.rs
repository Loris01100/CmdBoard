//! UI text in several languages. Each language is a TOML file in `locales/`, embedded at
//! build time; nested tables give dotted keys (`[form] name = …` is `form.name`). A key
//! missing from the current language falls back to English, then to the key itself.
//!
//! The current language is global rather than in `App`: parser, storage and launcher
//! errors are translated too, and the update thread formats its own.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// `(code, file)`. Adding a language: a file in `locales/` and a line here.
pub const LANGS: &[(&str, &str)] = &[
    ("en", include_str!("../locales/en.toml")),
    ("fr", include_str!("../locales/fr.toml")),
    ("pt", include_str!("../locales/pt.toml")),
];

/// Index of English in `LANGS`, the fallback for missing keys.
const FALLBACK: usize = 0;

/// French until `init` picks the language: the tests read French.
static CURRENT: AtomicUsize = AtomicUsize::new(1);

static TABLES: LazyLock<Vec<HashMap<String, String>>> = LazyLock::new(|| {
    LANGS
        .iter()
        .map(|(_, text)| {
            let mut keys = HashMap::new();
            flatten("", text.parse().unwrap_or_default(), &mut keys);
            keys
        })
        .collect()
});

fn flatten(prefix: &str, table: toml::Table, out: &mut HashMap<String, String>) {
    for (key, value) in table {
        let key = if prefix.is_empty() {
            key
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            toml::Value::Table(table) => flatten(&key, table, out),
            toml::Value::String(text) => {
                out.insert(key, text);
            }
            _ => {}
        }
    }
}

/// `t!("key")`, or `t!("key", name = value, …)` to fill `{name}`; `t!("key", name)`
/// is short for `name = name`.
macro_rules! t {
    (@value $name:ident) => {
        $name
    };
    (@value $name:ident $value:expr) => {
        $value
    };
    ($key:literal) => {
        $crate::i18n::tr($key, &[])
    };
    ($key:literal, $($name:ident $(= $value:expr)?),+ $(,)?) => {
        $crate::i18n::tr(
            $key,
            &[$((stringify!($name), &t!(@value $name $($value)?) as &dyn std::fmt::Display)),+],
        )
    };
}

/// The text for `key` in the current language, with `{name}` placeholders filled.
pub fn tr(key: &str, args: &[(&str, &dyn Display)]) -> String {
    fill(lookup(key).unwrap_or(key), args)
}

fn lookup(key: &str) -> Option<&'static str> {
    let tables = &*TABLES;
    tables[CURRENT.load(Ordering::Relaxed)]
        .get(key)
        .or_else(|| tables[FALLBACK].get(key))
        .map(String::as_str)
}

/// One pass, so a value holding `{x}` is never filled again.
fn fill(template: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let value = after
            .find('}')
            .and_then(|close| Some((close, args.iter().find(|(n, _)| *n == &after[..close])?)));
        match value {
            Some((close, (_, value))) => {
                out.push_str(&value.to_string());
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Name and description of a starter reward in the current language; rewards the
/// locale files don't know (added by hand) keep the text stored in the database.
pub fn reward_text(code: &str, field: &str, stored: String) -> String {
    lookup(&format!("starter_rewards.{code}.{field}")).map_or(stored, String::from)
}

/// Switches language. `false` if `code` is not one of `LANGS`.
pub fn set(code: &str) -> bool {
    let code = code.trim().to_lowercase();
    match LANGS.iter().position(|(c, _)| *c == code) {
        Some(i) => {
            CURRENT.store(i, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

pub fn current() -> &'static str {
    LANGS[CURRENT.load(Ordering::Relaxed)].0
}

/// At startup: the language from `config.toml`, else Windows' display language if we
/// have it, else English. Returns an error to show if the configured one is unknown.
pub fn init(configured: Option<&str>) -> Option<String> {
    if let Some(code) = configured
        && set(code)
    {
        return None;
    }
    if !system().is_some_and(|code| set(&code)) {
        set("en");
    }
    configured.map(|code| t!("lang.unknown_config", code))
}

/// Windows' display language, e.g. "fr" for fr-FR.
fn system() -> Option<String> {
    use windows_sys::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName};
    let mut name = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH
    // SAFETY: the buffer is as long as we say.
    let len = unsafe {
        LCIDToLocaleName(
            GetUserDefaultUILanguage() as u32,
            name.as_mut_ptr(),
            name.len() as i32,
            0,
        )
    };
    let name = String::from_utf16_lossy(name.get(..(len as usize).checked_sub(1)?)?);
    Some(name.split('-').next()?.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only French is set: the language is global and tests run in parallel.
    #[test]
    fn init_keeps_a_known_language_and_reads_windows() {
        assert_eq!(init(Some("fr")), None);
        assert_eq!(current(), "fr");
        let code = system().unwrap();
        assert!(
            code.len() >= 2 && code.chars().all(|c| c.is_ascii_lowercase()),
            "{code}"
        );
    }

    #[test]
    fn flatten_skips_non_text_values() {
        let mut out = HashMap::new();
        flatten("", "a = 1\n[b]\nc = \"x\"".parse().unwrap(), &mut out);
        assert_eq!(out, HashMap::from([("b.c".to_string(), "x".to_string())]));
    }

    /// `{placeholder}` names in a text, sorted.
    fn placeholders(text: &str) -> Vec<&str> {
        let mut names: Vec<&str> = text
            .split('{')
            .skip(1)
            .filter_map(|s| s.split_once('}').map(|(name, _)| name))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn every_language_has_every_key_and_placeholder() {
        let english = &TABLES[FALLBACK];
        assert!(english.len() > 100, "en.toml did not parse");
        for (i, (code, _)) in LANGS.iter().enumerate() {
            let table = &TABLES[i];
            for (key, text) in english {
                let translated = table
                    .get(key)
                    .unwrap_or_else(|| panic!("{code}: missing {key}"));
                assert_eq!(
                    placeholders(translated),
                    placeholders(text),
                    "{code}: placeholders of {key}"
                );
            }
            let extra: Vec<_> = table.keys().filter(|k| !english.contains_key(*k)).collect();
            assert!(
                extra.is_empty(),
                "{code}: keys missing from en.toml: {extra:?}"
            );
        }
    }

    /// Every `t!("key"` literal in the sources is in en.toml.
    #[test]
    fn every_key_used_exists() {
        fn walk(dir: &std::path::Path, missing: &mut Vec<String>) {
            for path in std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()) {
                if path.is_dir() {
                    walk(&path, missing);
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                for (at, _) in text.match_indices("t!(\"") {
                    // Not the end of `format!("` and the like.
                    if text[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    let key = text[at + 4..].split('"').next().unwrap();
                    if !TABLES[FALLBACK].contains_key(key) && key != "key" {
                        missing.push(format!("{}: {key}", path.display()));
                    }
                }
            }
        }
        let mut missing = Vec::new();
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut missing,
        );
        assert!(missing.is_empty(), "unknown keys: {missing:#?}");
    }

    #[test]
    fn fills_placeholders_once() {
        let name = "{count}";
        assert_eq!(
            fill("{name} has {count} {x}", &[("name", &name), ("count", &3)]),
            "{count} has 3 {x}"
        );
        assert_eq!(fill("{ unclosed", &[]), "{ unclosed");
    }

    #[test]
    fn falls_back_to_the_key() {
        assert_eq!(tr("no.such.key", &[]), "no.such.key");
        assert_eq!(reward_text("custom", "name", "Mine".into()), "Mine");
    }
}
