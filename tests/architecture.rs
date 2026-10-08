//! Architecture rules, read from the source text so that CI fails when one is broken.
//! The layers and limits are described in AGENTS.md and in section 2 of the design plan.
//!
//! - Every file under `src/` belongs to a layer; a new module must be classified here.
//! - A layer only depends on the layers below it: domain ← infrastructure ← application
//!   ← presentation. The check looks for forbidden paths outside `#[cfg(test)] mod tests`.
//! - A file holds at most `MAX_FNS` functions and `MAX_LINES` lines, tests excluded.
//!   Function length is limited by clippy (`too_many_lines`, clippy.toml).

use std::fs;
use std::path::{Path, PathBuf};

const MAX_FNS: usize = 30;
const MAX_LINES: usize = 600;

struct Layer {
    name: &'static str,
    /// Files (`.rs`) or folders (ending in `/`), relative to `src/`.
    paths: &'static [&'static str],
    /// Text that must not appear in the layer's code.
    forbidden: &'static [&'static str],
}

/// Outward dependencies and I/O are forbidden, innermost layer first.
const LAYERS: &[Layer] = &[
    Layer {
        name: "domain",
        paths: &["core/"],
        forbidden: &[
            "crate::app",
            "crate::command",
            "crate::config",
            "crate::event",
            "crate::launcher",
            "crate::optimize",
            "crate::popup",
            "crate::storage",
            "crate::tracker",
            "crate::ui",
            "crate::update",
            "crossterm",
            "ratatui",
            "rusqlite",
            "sysinfo",
            "windows_sys",
            "std::fs",
            "std::net",
            "std::process",
            "std::thread",
        ],
    },
    Layer {
        name: "infrastructure",
        paths: &[
            "storage/",
            "launcher/",
            "config.rs",
            "event.rs",
            "instance.rs",
            "optimize.rs",
            "tracker.rs",
            "update.rs",
        ],
        forbidden: &[
            "crate::app",
            "crate::command",
            "crate::popup",
            "crate::ui",
            "ratatui",
        ],
    },
    Layer {
        name: "application",
        paths: &["app/", "command/", "popup.rs"],
        forbidden: &["rusqlite", "crate::ui::screens", "crate::ui::widgets"],
    },
    Layer {
        name: "presentation",
        paths: &["ui/"],
        forbidden: &[
            "rusqlite",
            "crate::config",
            "crate::launcher::launch",
            "crate::storage::Database",
            "crate::storage::db",
            "crate::tracker",
            "crate::update",
            "std::process",
            "std::thread",
        ],
    },
    // Used by every layer: translations, text editing, fuzzy matching, and the
    // composition root that wires the layers together.
    Layer {
        name: "shared",
        paths: &["i18n.rs", "fuzzy.rs", "text_input.rs", "main.rs"],
        forbidden: &[],
    },
];

/// Every `.rs` file under `dir`, as a path relative to `src/` with `/` separators.
fn sources(dir: &Path, root: &Path, out: &mut Vec<(String, PathBuf)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let relative = path.strip_prefix(root).unwrap().to_string_lossy();
            out.push((relative.replace('\\', "/"), path));
        }
    }
}

fn all_sources() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &root, &mut files);
    files
        .into_iter()
        .map(|(name, path)| (name, fs::read_to_string(path).unwrap()))
        .collect()
}

/// Whole test files (`app/tests/`) are left out of the rules.
fn is_test_file(name: &str) -> bool {
    name.contains("/tests/") || name.ends_with("/tests.rs")
}

/// The code before the file's test module, without comment lines.
fn code_lines(text: &str) -> Vec<&str> {
    let lines: Vec<&str> = text.lines().collect();
    let end = lines
        .windows(2)
        .position(|w| {
            // An inline `mod tests { … }`; `mod tests;` only declares a test file.
            let item = w[1].trim();
            w[0].trim() == "#[cfg(test)]" && item.starts_with("mod ") && item.ends_with('{')
        })
        .unwrap_or(lines.len());
    lines[..end]
        .iter()
        .copied()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect()
}

/// `fn` items, whatever their visibility and qualifiers.
fn is_fn(line: &str) -> bool {
    let mut rest = line.trim_start();
    if let Some(after) = rest.strip_prefix("pub") {
        rest = after.trim_start();
        if rest.starts_with('(') {
            rest = rest
                .split_once(')')
                .map_or("", |(_, after)| after)
                .trim_start();
        }
    }
    for qualifier in ["const ", "async ", "unsafe "] {
        rest = rest.strip_prefix(qualifier).unwrap_or(rest);
    }
    rest.starts_with("fn ")
}

fn layer_of(name: &str) -> Option<&'static Layer> {
    LAYERS.iter().find(|layer| {
        layer.paths.iter().any(|path| match path.strip_suffix('/') {
            Some(dir) => name.starts_with(&format!("{dir}/")),
            None => name == *path,
        })
    })
}

#[test]
fn every_module_belongs_to_a_layer() {
    let unknown: Vec<String> = all_sources()
        .into_iter()
        .map(|(name, _)| name)
        .filter(|name| !is_test_file(name) && layer_of(name).is_none())
        .collect();
    assert!(
        unknown.is_empty(),
        "classify these files in tests/architecture.rs: {unknown:?}"
    );
}

#[test]
fn layers_only_depend_inwards() {
    let mut broken = Vec::new();
    for (name, text) in all_sources() {
        let Some(layer) = layer_of(&name).filter(|_| !is_test_file(&name)) else {
            continue;
        };
        for line in code_lines(&text) {
            for forbidden in layer.forbidden {
                if line.contains(forbidden) {
                    broken.push(format!("{name} ({}): {}", layer.name, line.trim()));
                }
            }
        }
    }
    assert!(
        broken.is_empty(),
        "forbidden dependencies:\n{}",
        broken.join("\n")
    );
}

#[test]
fn files_stay_small() {
    let mut too_big = Vec::new();
    for (name, text) in all_sources() {
        if is_test_file(&name) {
            continue;
        }
        let lines = code_lines(&text);
        let fns = lines.iter().filter(|l| is_fn(l)).count();
        if fns > MAX_FNS || lines.len() > MAX_LINES {
            too_big.push(format!("{name}: {fns} fns, {} lines", lines.len()));
        }
    }
    assert!(
        too_big.is_empty(),
        "split these files (max {MAX_FNS} fns, {MAX_LINES} lines, tests excluded):\n{}",
        too_big.join("\n")
    );
}

#[test]
fn rule_helpers() {
    for (line, expected) in [
        ("    pub(super) fn step(", true),
        ("pub const fn a()", true),
        ("    fn b() {", true),
        ("    let fn_name = 1;", false),
        ("    // fn commented()", false),
    ] {
        assert_eq!(is_fn(line), expected, "{line}");
    }
    let text = "fn a() {}\n// fn b\n#[cfg(test)]\nmod tests {\n    fn c() {}\n}\n";
    assert_eq!(code_lines(text), ["fn a() {}"]);
    let declared = "#[cfg(test)]
mod tests;
fn a() {}
";
    assert_eq!(code_lines(declared).len(), 3); // the code after it still counts
    assert_eq!(layer_of("core/xp.rs").unwrap().name, "domain");
    assert_eq!(layer_of("ui/theme.rs").unwrap().name, "presentation");
    assert!(layer_of("new.rs").is_none());
}
