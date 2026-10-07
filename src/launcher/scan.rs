//! Installed apps, from the Steam and Epic libraries and the Start Menu and Desktop
//! shortcuts (`.lnk`, and `.url` for games). Feeds the picker of the "add app" form.

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Deserialize;

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
/// Library games come first so they win over a `.url` of the same name (no exe).
pub fn scan() -> Vec<Shortcut> {
    let (mut found, (epic, shortcuts)) =
        rayon::join(steam_games, || rayon::join(epic_games, shortcuts));
    found.extend(epic);
    found.extend(shortcuts);
    // Stable sort: dedup keeps the library game.
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

/// Start Menu and Desktop shortcuts. Listing files is cheap; parsing them runs in parallel.
fn shortcuts() -> Vec<Shortcut> {
    let mut files = Vec::new();
    for dir in shortcut_dirs() {
        walk(&dir, &mut files);
    }
    files.par_iter().filter_map(|p| read_shortcut(p)).collect()
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for path in entries.flatten().map(|e| e.path()) {
        if path.is_dir() {
            walk(&path, files);
        } else {
            files.push(path);
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

/// Installed Steam games: every library of `libraryfolders.vdf`, every `appmanifest_*.acf`.
fn steam_games() -> Vec<Shortcut> {
    let Some(steam) = steam_dir() else {
        return Vec::new();
    };
    let libraries: Vec<PathBuf> =
        std::fs::read_to_string(steam.join(r"steamapps\libraryfolders.vdf"))
            .map(|vdf| vdf_values(&vdf, "path").map(PathBuf::from).collect())
            .unwrap_or_else(|_| vec![steam]);
    let mut manifests = Vec::new();
    for steamapps in libraries.iter().map(|lib| lib.join("steamapps")) {
        let Ok(entries) = std::fs::read_dir(&steamapps) else {
            continue;
        };
        for path in entries.flatten().map(|e| e.path()) {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with("appmanifest_") && name.ends_with(".acf") {
                manifests.push((steamapps.clone(), path));
            }
        }
    }
    // Parallel: each game walks its install folder to guess the exe.
    manifests
        .par_iter()
        .filter_map(|(steamapps, path)| {
            steam_game(
                steamapps,
                &std::fs::read_to_string(path).unwrap_or_default(),
            )
        })
        .collect()
}

fn steam_dir() -> Option<PathBuf> {
    let out = std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let path = text.lines().find_map(|l| l.split_once("REG_SZ"))?.1.trim();
    Some(PathBuf::from(path))
}

fn steam_game(steamapps: &Path, acf: &str) -> Option<Shortcut> {
    let get = |key| vdf_values(acf, key).next();
    let id = get("appid")?;
    let flags: u32 = get("StateFlags").and_then(|f| f.parse().ok()).unwrap_or(0);
    // 4: fully installed. 228980: Steamworks Common Redistributables.
    if flags & 4 == 0 || id == "228980" {
        return None;
    }
    Some(Shortcut {
        name: get("name")?,
        target: format!("steam://rungameid/{id}"),
        watch_exe: main_exe(&steamapps.join("common").join(get("installdir")?)),
    })
}

/// Values of `"key" "value"` lines in a Valve KeyValues text (`.vdf`, `.acf`), in order.
fn vdf_values<'a>(text: &'a str, key: &'a str) -> impl Iterator<Item = String> + 'a {
    text.lines()
        .filter_map(move |line| match quoted(line).as_slice() {
            [k, v] if k.eq_ignore_ascii_case(key) => Some(v.clone()),
            _ => None,
        })
}

/// Quoted strings of a line, with backslash escapes resolved.
fn quoted(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.chars();
    while chars.by_ref().any(|c| c == '"') {
        let mut s = String::new();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => s.extend(chars.next()),
                c => s.push(c),
            }
        }
        out.push(s);
    }
    out
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct EpicManifest {
    display_name: String,
    install_location: String,
    #[serde(default)]
    launch_executable: String,
    app_name: String,
    catalog_namespace: String,
    catalog_item_id: String,
    #[serde(default)]
    main_game_app_name: String,
    #[serde(rename = "bIsIncompleteInstall", default)]
    incomplete: bool,
}

/// Installed Epic games, from the launcher's `.item` manifests.
fn epic_games() -> Vec<Shortcut> {
    let Some(data) = std::env::var_os("ProgramData") else {
        return Vec::new();
    };
    let dir = PathBuf::from(data).join(r"Epic\EpicGamesLauncher\Data\Manifests");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "item"))
        .filter_map(|e| epic_game(&std::fs::read_to_string(e.path()).ok()?))
        .collect()
}

fn epic_game(json: &str) -> Option<Shortcut> {
    let m: EpicManifest = serde_json::from_str(json).ok()?;
    // DLCs have their own manifest, pointing to the main game.
    let dlc = !m.main_game_app_name.is_empty() && m.main_game_app_name != m.app_name;
    if m.incomplete || dlc {
        return None;
    }
    let exe = Path::new(&m.install_location).join(&m.launch_executable);
    Some(Shortcut {
        name: m.display_name,
        target: format!(
            "com.epicgames.launcher://apps/{}%3A{}%3A{}?action=launch&silent=true",
            m.catalog_namespace, m.catalog_item_id, m.app_name
        ),
        watch_exe: watch_exe_for(&exe.to_string_lossy()),
    })
}

/// File name of a Steam game's exe, which the manifest does not record.
/// ponytail: guesses the largest exe up to three folders deep (Unreal games keep the real
/// one in `Binaries\Win64`); a wrong guess is fixed in the form.
fn main_exe(dir: &Path) -> Option<String> {
    fn visit(dir: &Path, depth: u8, best: &mut Option<(u64, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let lower = name.to_lowercase();
            let skipped = ["redist", "crash", "setup", "support", "prereq"]
                .iter()
                .any(|w| lower.contains(w));
            if skipped || is_noise(&name) || lower.starts_with("unins") {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                if depth > 0 {
                    visit(&path, depth - 1, best);
                }
            } else if is_exe(&name) {
                let size = entry.metadata().map_or(0, |m| m.len());
                if best.as_ref().is_none_or(|(s, _)| size > *s) {
                    *best = Some((size, name));
                }
            }
        }
    }
    let mut best = None;
    visit(dir, 3, &mut best);
    best.map(|(_, name)| name)
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
    fn missing_folder_has_no_shortcut() {
        let mut files = Vec::new();
        walk(Path::new(r"C:\cmdboard-does-not-exist"), &mut files);
        assert!(files.is_empty());
    }

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
    fn steam_manifests_give_rungameid_targets() {
        let acf = "\"AppState\"\n{\n\t\"appid\"\t\t\"1145360\"\n\t\"name\"\t\t\"Hades\"\n\t\"StateFlags\"\t\t\"4\"\n\t\"installdir\"\t\t\"Hades\"\n}\n";
        let game = steam_game(Path::new(r"C:\nope\steamapps"), acf).unwrap();
        assert_eq!(game.name, "Hades");
        assert_eq!(game.target, "steam://rungameid/1145360");
        assert_eq!(game.watch_exe, None); // install dir missing
        let updating = acf.replace("\"4\"", "\"6\"");
        assert!(steam_game(Path::new("x"), &updating).is_some());
        let downloading = acf.replace("\"4\"", "\"1026\"");
        assert_eq!(steam_game(Path::new("x"), &downloading), None);
        let redist = acf.replace("1145360", "228980");
        assert_eq!(steam_game(Path::new("x"), &redist), None);
    }

    #[test]
    fn vdf_paths_are_unescaped() {
        let vdf = r#""libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
	}
}"#;
        let paths: Vec<String> = vdf_values(vdf, "path").collect();
        assert_eq!(paths, [r"C:\Program Files (x86)\Steam", r"D:\SteamLibrary"]);
    }

    #[test]
    fn epic_manifests_give_launcher_uris_and_skip_dlcs() {
        let json = r#"{"DisplayName":"Hades","InstallLocation":"C:\\Games\\Hades","LaunchExecutable":"x64\\Hades.exe","AppName":"Min","CatalogNamespace":"ns","CatalogItemId":"item","MainGameAppName":"Min","bIsIncompleteInstall":false}"#;
        let game = epic_game(json).unwrap();
        assert_eq!(game.name, "Hades");
        assert_eq!(
            game.target,
            "com.epicgames.launcher://apps/ns%3Aitem%3AMin?action=launch&silent=true"
        );
        assert_eq!(game.watch_exe.as_deref(), Some("Hades.exe"));
        let dlc = json.replace(r#""MainGameAppName":"Min""#, r#""MainGameAppName":"Other""#);
        assert_eq!(epic_game(&dlc), None);
        assert_eq!(epic_game(&json.replace("false", "true")), None);
        assert_eq!(epic_game("not json"), None);
    }

    #[test]
    fn main_exe_is_the_largest_real_one() {
        let dir = std::env::temp_dir().join(format!("cmdboard-exe-{}", std::process::id()));
        let bin = dir.join(r"Game\Binaries\Win64");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(dir.join("Game.exe"), [0; 10]).unwrap();
        std::fs::write(bin.join("Game-Win64-Shipping.exe"), [0; 100]).unwrap();
        std::fs::write(dir.join("unins000.exe"), [0; 1000]).unwrap();
        std::fs::write(dir.join("UnityCrashHandler64.exe"), [0; 1000]).unwrap();
        std::fs::write(dir.join("data.pak"), [0; 1000]).unwrap();
        let found = main_exe(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(found.as_deref(), Some("Game-Win64-Shipping.exe"));
        assert_eq!(main_exe(Path::new(r"C:\nope")), None);
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
