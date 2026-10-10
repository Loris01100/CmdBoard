//! Games and apps of the launchers that keep their library in the registry (GOG Galaxy,
//! Ubisoft Connect), and Microsoft Store / Xbox apps, which have no `.lnk` to scan.
//! Feeds the picker of the "add app" form, with the Steam and Epic libraries (`scan`).

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;
use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

use super::launch::watch_exe_for;
use super::programs::wide;
use super::registry::Key;
use super::scan::{Shortcut, is_noise, main_exe};

/// Both launchers are 32-bit and write under `WOW6432Node`.
const GOG_GAMES: &str = r"SOFTWARE\GOG.com\Games";
const UBISOFT_INSTALLS: &str = r"SOFTWARE\Ubisoft\Launcher\Installs";
const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

/// GOG games, launched through their own exe: GOG games run without the launcher.
pub fn gog_games() -> Vec<Shortcut> {
    let Some(key) = Key::open(HKEY_LOCAL_MACHINE, GOG_GAMES, KEY_WOW64_32KEY) else {
        return Vec::new();
    };
    key.subkeys()
        .iter()
        .filter_map(|id| {
            let id = wide(id);
            let text = |name: &str| key.string(&id, name);
            gog_game(text("gameName"), text("exe"), text("dependsOn").as_deref())
        })
        .filter(|game| Path::new(&game.target).exists())
        .collect()
}

/// `depends_on` marks a DLC or a mod of another game.
fn gog_game(
    name: Option<String>,
    exe: Option<String>,
    depends_on: Option<&str>,
) -> Option<Shortcut> {
    if depends_on.is_some() {
        return None;
    }
    let exe = exe?;
    Some(Shortcut {
        name: name?,
        watch_exe: watch_exe_for(&exe),
        target: exe,
    })
}

/// Ubisoft Connect games, launched with `uplay://launch/<id>/0`. Their name is in the
/// uninstall entry Ubisoft Connect writes; the process is guessed like Steam's.
pub fn ubisoft_games() -> Vec<Shortcut> {
    let Some(installs) = Key::open(HKEY_LOCAL_MACHINE, UBISOFT_INSTALLS, KEY_WOW64_32KEY) else {
        return Vec::new();
    };
    let uninstall = Key::open(HKEY_LOCAL_MACHINE, UNINSTALL_KEY, KEY_WOW64_32KEY);
    installs
        .subkeys()
        .iter()
        .filter_map(|id| {
            let dir = installs.string(&wide(id), "InstallDir")?;
            let name = uninstall
                .as_ref()
                .and_then(|u| u.string(&wide(&format!("Uplay Install {id}")), "DisplayName"));
            let mut game = ubisoft_game(id, &dir, name)?;
            game.watch_exe = main_exe(Path::new(&dir));
            Some(game)
        })
        .collect()
}

/// Without its uninstall entry, a game is named after its folder.
fn ubisoft_game(id: &str, install_dir: &str, name: Option<String>) -> Option<Shortcut> {
    let dir = Path::new(install_dir.trim_end_matches(['\\', '/']));
    let name = name.or_else(|| Some(dir.file_name()?.to_string_lossy().into_owned()))?;
    Some(Shortcut {
        name,
        target: format!("uplay://launch/{id}/0"),
        watch_exe: None,
    })
}

/// Lists the Start menu's packaged apps (`Get-StartApps`, which resolves their display
/// names) with their package folder (`Get-AppxPackage`), as JSON.
const STORE_SCRIPT: &str = "$ErrorActionPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$locations = @{}
Get-AppxPackage | ForEach-Object { $locations[$_.PackageFamilyName] = $_.InstallLocation }
$apps = Get-StartApps | Where-Object { $_.AppID -like '*!*' } | ForEach-Object {
    [pscustomobject]@{ Name = $_.Name; AppId = $_.AppID; Location = $locations[$_.AppID.Split('!')[0]] }
}
ConvertTo-Json -InputObject @($apps) -Compress";

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StoreApp {
    name: String,
    /// `<package family>!<app id>`.
    app_id: String,
    location: Option<String>,
}

/// Microsoft Store and Xbox (Game Pass) apps, launched with `shell:AppsFolder\<AppID>`.
/// Read through a hidden Windows PowerShell, about a second, in the scan thread.
pub fn store_apps() -> Vec<Shortcut> {
    let Some(root) = std::env::var_os("SystemRoot") else {
        return Vec::new();
    };
    let powershell = PathBuf::from(root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    // Its own hidden console: it must not touch the one CmdBoard draws in.
    let output = Command::new(powershell)
        .args(["-NoProfile", "-NonInteractive", "-Command", STORE_SCRIPT])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match output {
        Ok(out) => store_shortcuts(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => Vec::new(),
    }
}

fn store_shortcuts(json: &str) -> Vec<Shortcut> {
    let apps: Vec<StoreApp> = serde_json::from_str(json.trim()).unwrap_or_default();
    apps.into_iter()
        .filter(|app| !is_noise(&app.name))
        .map(|app| {
            let entry = app.app_id.split_once('!').map_or("", |(_, entry)| entry);
            let watch_exe = app
                .location
                .as_deref()
                .and_then(|dir| store_exe(Path::new(dir), entry));
            Shortcut {
                target: format!(r"shell:AppsFolder\{}", app.app_id),
                name: app.name,
                watch_exe,
            }
        })
        .collect()
}

/// Process of a packaged app: the game's exe for Xbox games (`MicrosoftGame.config`, as
/// their manifest only names `GameLaunchHelper.exe`), else the manifest's `Executable`.
fn store_exe(dir: &Path, entry: &str) -> Option<String> {
    let read = |file: &str| std::fs::read_to_string(dir.join(file)).ok();
    let exe = read("MicrosoftGame.config")
        .and_then(|config| {
            let config = strip_comments(&config);
            elements(&config, "Executable")
                .into_iter()
                .find_map(|tag| attribute(tag, "Name"))
                .map(String::from)
        })
        .or_else(|| {
            let manifest = strip_comments(&read("AppxManifest.xml")?);
            elements(&manifest, "Application")
                .into_iter()
                .find(|tag| attribute(tag, "Id") == Some(entry))
                .and_then(|tag| attribute(tag, "Executable"))
                .map(String::from)
        })?;
    watch_exe_for(&exe)
}

fn strip_comments(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        rest = rest[start..]
            .find("-->")
            .map_or("", |end| &rest[start + end + 3..]);
    }
    out.push_str(rest);
    out
}

/// Opening tags `<name …>` of an XML text, attributes included.
fn elements<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let open = format!("<{name}");
    xml.match_indices(open.as_str())
        .filter_map(|(start, _)| {
            let tag = &xml[start..];
            let after = tag[open.len()..].chars().next()?;
            let end = tag.find('>')?;
            (after.is_whitespace() || after == '>' || after == '/').then(|| &tag[..end])
        })
        .collect()
}

/// Value of `name="…"` in an opening tag.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag;
    loop {
        let at = rest.find(name)?;
        let before = rest[..at].chars().next_back();
        let after = rest[at + name.len()..].trim_start();
        rest = &rest[at + name.len()..];
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let Some(value) = after.strip_prefix('=').map(str::trim_start) else {
            continue;
        };
        let value = value.strip_prefix('"')?;
        return value.find('"').map(|end| &value[..end]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gog_games_launch_their_exe_and_skip_dlcs() {
        let exe = || Some(r"C:\GOG Games\Witcher 3\bin\x64\witcher3.exe".to_string());
        let game = gog_game(Some("The Witcher 3".into()), exe(), None).unwrap();
        assert_eq!(game.target, r"C:\GOG Games\Witcher 3\bin\x64\witcher3.exe");
        assert_eq!(game.watch_exe.as_deref(), Some("witcher3.exe"));
        assert_eq!(
            gog_game(Some("Blood and Wine".into()), exe(), Some("1207664643")),
            None
        );
        assert_eq!(gog_game(None, exe(), None), None);
        assert_eq!(gog_game(Some("Broken".into()), None, None), None);
    }

    #[test]
    fn ubisoft_games_use_uplay_uris_and_fall_back_to_the_folder_name() {
        let named = ubisoft_game("635", r"C:\Games\Rayman", Some("Rayman Legends".into())).unwrap();
        assert_eq!(
            (named.name.as_str(), named.target.as_str()),
            ("Rayman Legends", "uplay://launch/635/0")
        );
        let unnamed = ubisoft_game("5595", r"D:\Ubisoft\Anno 1800\", None).unwrap();
        assert_eq!(unnamed.name, "Anno 1800");
        assert_eq!(ubisoft_game("1", r"C:\", None), None);
    }

    #[test]
    fn store_apps_launch_from_apps_folder_with_their_real_exe() {
        let dir = std::env::temp_dir().join(format!("cmdboard-store-{}", std::process::id()));
        let notepad = dir.join("Notepad");
        let game = dir.join("Game");
        std::fs::create_dir_all(&notepad).unwrap();
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(
            notepad.join("AppxManifest.xml"),
            r#"<Package><Applications>
                <!-- <Application Id="App" Executable="Old.exe"> -->
                <Application Id="Other" Executable="Other.exe"/>
                <Application Id="App" Executable="Notepad\Notepad.exe" EntryPoint="Windows.FullTrustApplication">
            </Applications></Package>"#,
        )
        .unwrap();
        std::fs::write(
            game.join("AppxManifest.xml"),
            r#"<Application Id="Game" Executable="GameLaunchHelper.exe">"#,
        )
        .unwrap();
        std::fs::write(
            game.join("MicrosoftGame.config"),
            r#"<Game><ExecutableList>
                <Executable Name="Minecraft.exe" TargetDeviceFamily="PC" />
                <!-- Name="YourProjectExeName" -->
            </ExecutableList></Game>"#,
        )
        .unwrap();
        let json = serde_json::json!([
            {"Name": "Bloc-notes", "AppId": "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App", "Location": notepad},
            {"Name": "Minecraft Launcher", "AppId": "Microsoft.4297127D64EC6_8wekyb3d8bbwe!Game", "Location": game},
            {"Name": "No folder", "AppId": "Some.App_123!App", "Location": null},
            {"Name": "Uninstall helper", "AppId": "Some.Tool_123!App", "Location": null},
        ]);
        let found = store_shortcuts(&json.to_string());
        std::fs::remove_dir_all(&dir).unwrap();

        let summary: Vec<_> = found
            .iter()
            .map(|s| (s.name.as_str(), s.target.as_str(), s.watch_exe.as_deref()))
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "Bloc-notes",
                    r"shell:AppsFolder\Microsoft.WindowsNotepad_8wekyb3d8bbwe!App",
                    Some("Notepad.exe")
                ),
                (
                    "Minecraft Launcher",
                    r"shell:AppsFolder\Microsoft.4297127D64EC6_8wekyb3d8bbwe!Game",
                    Some("Minecraft.exe")
                ),
                ("No folder", r"shell:AppsFolder\Some.App_123!App", None),
            ]
        );
        assert!(store_shortcuts("").is_empty());
        assert!(store_shortcuts("not json").is_empty());
    }

    #[test]
    fn attributes_need_a_whole_name() {
        let tag = r#"<Application uap:Id="x" AppId="y" Id = "App" Executable="a.exe""#;
        assert_eq!(attribute(tag, "Id"), Some("App"));
        assert_eq!(attribute(tag, "Executable"), Some("a.exe"));
        assert_eq!(attribute(tag, "Missing"), None);
        assert_eq!(
            elements("<Applications><Application Id=\"A\">", "Application").len(),
            1
        );
    }

    #[test]
    fn this_pc_store_apps_are_read_without_panicking() {
        // Reads the real Start menu; CI runners may have few packaged apps.
        for app in store_apps() {
            assert!(app.target.starts_with(r"shell:AppsFolder\") && !app.name.is_empty());
        }
        gog_games();
        ubisoft_games();
    }
}
