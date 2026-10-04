//! `%APPDATA%\CmdBoard\config.toml`: user preferences (for now, the theme).

use std::path::Path;

use anyhow::Context;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// Theme name chosen with `:theme`; `None` picks one from the terminal's colors.
    pub theme: Option<String>,
}

impl Config {
    /// A missing file is an empty config. A broken one too, plus a message to show.
    pub fn load(path: &Path) -> (Self, Option<String>) {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Self::default(), None),
            Err(e) => return (Self::default(), Some(format!("config.toml : {e}"))),
        };
        match text.parse::<toml::Table>() {
            Ok(table) => {
                let theme = table.get("theme").and_then(|v| v.as_str()).map(String::from);
                (Self { theme }, None)
            }
            Err(e) => (Self::default(), Some(format!("config.toml : {}", e.message().trim()))),
        }
    }
}

/// Sets one key in the file, keeping the others (including ones this version ignores).
pub fn save_value(path: &Path, key: &str, value: &str) -> anyhow::Result<()> {
    let mut table = match std::fs::read_to_string(path) {
        Ok(text) => text.parse::<toml::Table>().unwrap_or_default(),
        Err(_) => toml::Table::new(),
    };
    table.insert(key.into(), toml::Value::String(value.into()));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, toml::to_string(&table)?)
        .with_context(|| format!("impossible d'écrire {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads_keeping_other_keys() {
        let dir = std::env::temp_dir().join(format!("cmdboard-config-{}", std::process::id()));
        let path = dir.join("config.toml");
        assert_eq!(Config::load(&path), (Config::default(), None)); // missing

        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "other = 3\ntheme = \"terminal\"\n").unwrap();
        save_value(&path, "theme", "catppuccin-latte").unwrap();
        let (config, warning) = Config::load(&path);
        assert_eq!((config.theme.as_deref(), warning), (Some("catppuccin-latte"), None));
        assert!(std::fs::read_to_string(&path).unwrap().contains("other = 3"));

        std::fs::write(&path, "theme = ").unwrap();
        let (config, warning) = Config::load(&path);
        assert_eq!(config, Config::default());
        assert!(warning.unwrap().starts_with("config.toml : "));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
