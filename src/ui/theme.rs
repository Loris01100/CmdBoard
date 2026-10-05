//! Themes: TOML files with a `[palette]` of raw colors and `[slots]` giving each UI role a
//! palette name, a `#rrggbb` color or an ANSI color name. Built-in themes are embedded;
//! user themes in `%APPDATA%\CmdBoard\themes\*.toml` override them by name. The palette
//! is resolved at load time and not kept.

use std::path::Path;
use std::str::FromStr;

use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, BorderType},
};

/// Every color used by the UI. Widgets must never hardcode colors.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    /// Display name, from the file's `name`.
    pub name: String,
    pub border: Style,
    pub border_focused: Style,
    pub title: Style,
    pub selected: Style,
    pub selected_unfocused: Style,
    pub xp_fill: Color,
    pub info: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub muted: Color,
}

/// Built-in themes, by name (the file stem).
const BUILTIN: &[(&str, &str)] = &[
    (
        "catppuccin-latte",
        include_str!("../../themes/catppuccin-latte.toml"),
    ),
    (
        "catppuccin-frappe",
        include_str!("../../themes/catppuccin-frappe.toml"),
    ),
    (
        "catppuccin-macchiato",
        include_str!("../../themes/catppuccin-macchiato.toml"),
    ),
    (
        "catppuccin-mocha",
        include_str!("../../themes/catppuccin-mocha.toml"),
    ),
    ("terminal", include_str!("../../themes/terminal.toml")),
];

/// Used without truecolor: 16 ANSI colors that follow the terminal's scheme.
pub const FALLBACK: &str = "terminal";

const STYLE_SLOTS: &[&str] = &[
    "border",
    "border_focused",
    "title",
    "selected",
    "selected_unfocused",
];
const COLOR_SLOTS: &[&str] = &["xp_fill", "info", "success", "warning", "error", "muted"];

impl Default for Theme {
    fn default() -> Self {
        // Built-in themes are covered by tests, so this cannot fail at runtime.
        builtin(FALLBACK).expect("built-in theme")
    }
}

impl Theme {
    /// Parses a theme file. Every slot is required, and must resolve.
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: toml::Table = toml::from_str(text).map_err(|e| e.message().trim().to_string())?;
        let name = match file.get("name") {
            Some(toml::Value::String(name)) => name.clone(),
            Some(_) => return Err("name : texte attendu".into()),
            None => return Err("name manquant".into()),
        };
        let empty = toml::Table::new();
        let palette = table(&file, "palette")?.unwrap_or(&empty);
        let slots = table(&file, "slots")?.ok_or("section [slots] manquante")?;
        if let Some(unknown) = slots
            .keys()
            .find(|k| !STYLE_SLOTS.contains(&k.as_str()) && !COLOR_SLOTS.contains(&k.as_str()))
        {
            return Err(format!("slot inconnu : {unknown}"));
        }

        let slot = |key: &str| {
            slots
                .get(key)
                .ok_or_else(|| format!("slot manquant : {key}"))
        };
        let style =
            |key: &str| resolve_style(slot(key)?, palette).map_err(|e| format!("slot {key} : {e}"));
        let color = |key: &str| match slot(key)? {
            toml::Value::String(value) => {
                resolve_color(value, palette).map_err(|e| format!("slot {key} : {e}"))
            }
            _ => Err(format!("slot {key} : couleur attendue")),
        };
        Ok(Self {
            name,
            border: style("border")?,
            border_focused: style("border_focused")?,
            title: style("title")?,
            selected: style("selected")?,
            selected_unfocused: style("selected_unfocused")?,
            xp_fill: color("xp_fill")?,
            info: color("info")?,
            success: color("success")?,
            warning: color("warning")?,
            error: color("error")?,
            muted: color("muted")?,
        })
    }

    /// Bordered panel with its title embedded in the frame.
    pub fn panel(&self, title: &str, focused: bool) -> Block<'static> {
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(if focused {
                self.border_focused
            } else {
                self.border
            })
            .title(Span::styled(format!(" {title} "), self.title))
    }

    pub fn highlight(&self, focused: bool) -> Style {
        if focused {
            self.selected
        } else {
            self.selected_unfocused
        }
    }

    pub fn muted(&self) -> Style {
        Style::new().fg(self.muted)
    }
}

/// Every theme name: built-in ones and the user's, sorted, without duplicates.
pub fn available(user_dir: Option<&Path>) -> Vec<String> {
    let mut names: Vec<String> = BUILTIN.iter().map(|(name, _)| name.to_string()).collect();
    if let Some(entries) = user_dir.and_then(|dir| std::fs::read_dir(dir).ok()) {
        for path in entries.flatten().map(|e| e.path()) {
            if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"))
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                names.push(stem.to_lowercase());
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

/// Loads a theme by name (ignoring case): the user's file if there is one, else the
/// built-in theme. Errors name the file and the problem; the caller keeps its theme.
pub fn load(name: &str, user_dir: Option<&Path>) -> Result<Theme, String> {
    let name = name.trim().to_lowercase();
    if let Some(dir) = user_dir {
        let path = dir.join(format!("{name}.toml"));
        match std::fs::read_to_string(&path) {
            Ok(text) => return Theme::parse(&text).map_err(|e| format!("thème {name}.toml : {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("thème {name}.toml : {e}")),
        }
    }
    builtin(&name).map_err(|e| format!("thème {name} : {e}"))
}

fn builtin(name: &str) -> Result<Theme, String> {
    let (_, text) = BUILTIN
        .iter()
        .find(|(builtin, _)| *builtin == name)
        .ok_or("thème inconnu (:theme pour la liste)")?;
    Theme::parse(text)
}

/// Theme to use when none is configured: Catppuccin Mocha on a truecolor terminal
/// (Windows Terminal sets `WT_SESSION`), else the 16-color one.
pub fn default_name() -> &'static str {
    default_name_for(
        std::env::var_os("WT_SESSION").is_some(),
        std::env::var("COLORTERM").ok().as_deref(),
    )
}

fn default_name_for(wt_session: bool, colorterm: Option<&str>) -> &'static str {
    let truecolor = wt_session || matches!(colorterm, Some("truecolor" | "24bit"));
    if truecolor {
        "catppuccin-mocha"
    } else {
        FALLBACK
    }
}

fn table<'a>(file: &'a toml::Table, key: &str) -> Result<Option<&'a toml::Table>, String> {
    match file.get(key) {
        Some(toml::Value::Table(table)) => Ok(Some(table)),
        Some(_) => Err(format!("[{key}] : section attendue")),
        None => Ok(None),
    }
}

/// A palette name, else anything `Color` parses: `#rrggbb`, ANSI names, indexes.
fn resolve_color(value: &str, palette: &toml::Table) -> Result<Color, String> {
    if let Some(entry) = palette.get(value) {
        let toml::Value::String(raw) = entry else {
            return Err(format!("palette {value} : couleur attendue"));
        };
        return Color::from_str(raw)
            .map_err(|_| format!("palette {value} : couleur invalide « {raw} »"));
    }
    Color::from_str(value).map_err(|_| format!("couleur inconnue « {value} »"))
}

/// A color (foreground only), or `{ fg, bg, bold, italic, underlined, dim }`.
fn resolve_style(value: &toml::Value, palette: &toml::Table) -> Result<Style, String> {
    let table = match value {
        toml::Value::String(color) => return Ok(Style::new().fg(resolve_color(color, palette)?)),
        toml::Value::Table(table) => table,
        _ => return Err("couleur ou { fg, bg, bold… } attendu".into()),
    };
    let mut style = Style::new();
    for (key, value) in table {
        style = match (key.as_str(), value) {
            ("fg", toml::Value::String(c)) => style.fg(resolve_color(c, palette)?),
            ("bg", toml::Value::String(c)) => style.bg(resolve_color(c, palette)?),
            ("bold", toml::Value::Boolean(on)) => modifier(style, Modifier::BOLD, *on),
            ("italic", toml::Value::Boolean(on)) => modifier(style, Modifier::ITALIC, *on),
            ("underlined", toml::Value::Boolean(on)) => modifier(style, Modifier::UNDERLINED, *on),
            ("dim", toml::Value::Boolean(on)) => modifier(style, Modifier::DIM, *on),
            _ => return Err(format!("clé invalide : {key}")),
        };
    }
    Ok(style)
}

fn modifier(style: Style, modifier: Modifier, on: bool) -> Style {
    if on {
        style.add_modifier(modifier)
    } else {
        style
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_theme_resolves() {
        for (name, _) in BUILTIN {
            let theme = load(name, None).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!theme.name.is_empty());
        }
        let mocha = load("Catppuccin-Mocha", None).unwrap();
        assert_eq!(mocha.name, "Catppuccin Mocha");
        assert_eq!(
            mocha.border_focused,
            Style::new().fg(Color::Rgb(0x74, 0xc7, 0xec))
        );
        assert_eq!(
            mocha.selected,
            Style::new()
                .fg(Color::Rgb(0x1e, 0x1e, 0x2e))
                .bg(Color::Rgb(0xcb, 0xa6, 0xf7))
                .add_modifier(Modifier::BOLD)
        );
        assert_eq!(
            Theme::default().border_focused,
            Style::new().fg(Color::Cyan)
        );
    }

    #[test]
    fn terminal_theme_uses_ansi_colors_only() {
        let theme = load("terminal", None).unwrap();
        let colors = [
            theme.xp_fill,
            theme.info,
            theme.success,
            theme.warning,
            theme.error,
            theme.muted,
        ];
        assert!(colors.iter().all(|c| !matches!(c, Color::Rgb(..))));
    }

    fn with_slots(slots: &str) -> String {
        let mut text = String::from("name = \"T\"\n[palette]\nleaf = \"#00ff00\"\n[slots]\n");
        for slot in STYLE_SLOTS.iter().chain(COLOR_SLOTS) {
            if !slots.contains(&format!("{slot} ")) {
                text.push_str(&format!("{slot} = \"leaf\"\n"));
            }
        }
        text + slots
    }

    #[test]
    fn slots_accept_palette_hex_ansi_and_tables() {
        let theme = Theme::parse(&with_slots(
            "title = { fg = \"#112233\", bg = \"leaf\", italic = true }\nerror = \"lightred\"\n",
        ))
        .unwrap();
        assert_eq!(theme.success, Color::Rgb(0, 255, 0));
        assert_eq!(theme.error, Color::LightRed);
        assert_eq!(
            theme.title,
            Style::new()
                .fg(Color::Rgb(0x11, 0x22, 0x33))
                .bg(Color::Rgb(0, 255, 0))
                .add_modifier(Modifier::ITALIC)
        );
    }

    #[test]
    fn broken_themes_are_reported() {
        let cases = [
            (
                with_slots("error = \"nope\"\n"),
                "slot error : couleur inconnue « nope »",
            ),
            (
                with_slots("muted = { fg = \"leaf\" }\n"),
                "slot muted : couleur attendue",
            ),
            (
                with_slots("title = { size = 3 }\n"),
                "slot title : clé invalide : size",
            ),
            (with_slots("shadow = \"leaf\"\n"), "slot inconnu : shadow"),
            (
                "name = \"T\"\n[slots]\nborder = \"red\"\n".into(),
                "slot manquant : border_focused",
            ),
            ("[slots]\n".into(), "name manquant"),
            ("name = \"T\"\n".into(), "section [slots] manquante"),
        ];
        for (text, expected) in cases {
            assert_eq!(Theme::parse(&text), Err(expected.to_string()), "{text}");
        }
        assert!(Theme::parse("name = ").is_err());
        assert_eq!(
            load("nope", None),
            Err("thème nope : thème inconnu (:theme pour la liste)".into())
        );
    }

    #[test]
    fn user_themes_override_and_extend() {
        let dir = std::env::temp_dir().join(format!("cmdboard-themes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Terminal.toml"),
            with_slots("").replace("\"T\"", "\"Mine\""),
        )
        .unwrap();
        std::fs::write(dir.join("broken.toml"), "name = \"B\"").unwrap();
        std::fs::write(dir.join("notes.txt"), "").unwrap();

        let names = available(Some(&dir));
        assert!(names.contains(&"broken".to_string()));
        assert_eq!(names.iter().filter(|n| *n == "terminal").count(), 1);
        assert!(!names.contains(&"notes".to_string()));
        // Windows file names ignore case: "terminal" finds "Terminal.toml".
        assert_eq!(load("terminal", Some(&dir)).unwrap().name, "Mine");
        assert_eq!(
            load("broken", Some(&dir)),
            Err("thème broken.toml : section [slots] manquante".into())
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn truecolor_picks_catppuccin() {
        assert_eq!(default_name_for(true, None), "catppuccin-mocha");
        assert_eq!(
            default_name_for(false, Some("truecolor")),
            "catppuccin-mocha"
        );
        assert_eq!(default_name_for(false, Some("24bit")), "catppuccin-mocha");
        assert_eq!(default_name_for(false, None), "terminal");
        assert_eq!(default_name_for(false, Some("256")), "terminal");
    }
}
