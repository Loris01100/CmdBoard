//! Installed apps, from the Start Menu and Desktop shortcuts (`.lnk`, and `.url` for
//! Steam/Epic games). Feeds the picker of the "add app" form.

use std::path::{Path, PathBuf};

use super::launch::watch_exe_for;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    /// File name without extension, as shown in the Start Menu.
    pub name: String,
    /// What to launch: the `.lnk` itself (keeps its arguments), or the `.url`'s URI.
    pub target: String,
    /// Exe the shortcut points to, when it can be read.
    pub watch_exe: Option<String>,
}

/// Every shortcut found, sorted by name, without duplicates or uninstallers.
pub fn scan() -> Vec<Shortcut> {
    let mut found = Vec::new();
    for dir in shortcut_dirs() {
        walk(&dir, &mut found);
    }
    found.sort_by_key(|s| s.name.to_lowercase());
    found.dedup_by_key(|s| s.name.to_lowercase());
    found
}

fn shortcut_dirs() -> Vec<PathBuf> {
    let env = |var: &str| std::env::var_os(var).map(PathBuf::from);
    let start_menu = r"Microsoft\Windows\Start Menu\Programs";
    let mut dirs: Vec<PathBuf> = [env("ProgramData"), env("APPDATA")]
        .into_iter()
        .flatten()
        .map(|base| base.join(start_menu))
        .collect();
    dirs.extend(directories::UserDirs::new().and_then(|u| u.desktop_dir().map(Path::to_path_buf)));
    dirs.extend(env("PUBLIC").map(|p| p.join("Desktop")));
    dirs
}

fn walk(dir: &Path, found: &mut Vec<Shortcut>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, found);
        } else if let Some(shortcut) = read_shortcut(&path) {
            found.push(shortcut);
        }
    }
}

fn read_shortcut(path: &Path) -> Option<Shortcut> {
    let name = path.file_stem()?.to_string_lossy().into_owned();
    if is_noise(&name) {
        return None;
    }
    let extension = path.extension()?.to_string_lossy().to_lowercase();
    let (target, watch_exe) = match extension.as_str() {
        "lnk" => {
            let points_to = lnk_target(path);
            // Shortcuts to documents or folders are not apps. Unreadable targets
            // (Store apps, MSI "advertised" shortcuts) are kept, without a process.
            if points_to.as_ref().is_some_and(|t| !is_exe(t)) {
                return None;
            }
            let watch = points_to.as_deref().and_then(watch_exe_for);
            (path.to_string_lossy().into_owned(), watch)
        }
        "url" => (url_of(&std::fs::read_to_string(path).ok()?)?, None),
        _ => return None,
    };
    Some(Shortcut {
        name,
        target,
        watch_exe,
    })
}

/// Path the `.lnk` points to. Reads the fields directly: `ShellLink::link_target`
/// panics on some malformed shortcuts.
fn lnk_target(path: &Path) -> Option<String> {
    let link = lnk::ShellLink::open(path, lnk::encoding::WINDOWS_1252).ok()?;
    let info = link.link_info().as_ref()?;
    let base = info
        .local_base_path_unicode()
        .as_deref()
        .or(info.local_base_path())?;
    let suffix = info
        .common_path_suffix_unicode()
        .as_deref()
        .unwrap_or(info.common_path_suffix());
    Some(if suffix.is_empty() {
        base.to_string()
    } else {
        format!("{}\\{suffix}", base.trim_end_matches('\\'))
    })
}

fn is_exe(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
}

/// The `URL=` line of an Internet shortcut, only for app URIs (not web pages).
fn url_of(text: &str) -> Option<String> {
    let url = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("URL="))?
        .trim();
    let web = url.starts_with("http://") || url.starts_with("https://");
    (!web && url.contains("://")).then(|| url.to_string())
}

fn is_noise(name: &str) -> bool {
    let name = name.to_lowercase();
    ["uninstall", "désinstall", "desinstall"]
        .iter()
        .any(|w| name.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_files_keep_app_uris_only() {
        let steam = "[{000214A0-0000-0000-C000-000000000046}]\r\n[InternetShortcut]\r\nIDList=\r\nURL=steam://rungameid/1145360\r\nIconIndex=0\r\n";
        assert_eq!(url_of(steam).as_deref(), Some("steam://rungameid/1145360"));
        assert_eq!(
            url_of("[InternetShortcut]\nURL=https://example.com\n"),
            None
        );
        assert_eq!(url_of("[InternetShortcut]\n"), None);
    }

    #[test]
    fn uninstallers_are_skipped() {
        assert!(is_noise("Uninstall Hades"));
        assert!(is_noise("Désinstaller Discord"));
        assert!(!is_noise("Hades"));
    }

    #[test]
    fn scan_finds_start_menu_apps_without_panicking() {
        // Every Windows install has a few Start Menu shortcuts.
        let found = scan();
        assert!(!found.is_empty());
        assert!(
            found
                .iter()
                .all(|s| !s.name.is_empty() && !s.target.is_empty())
        );
    }
}
