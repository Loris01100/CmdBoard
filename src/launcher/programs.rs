//! Disks and installed programs, for the Storage screen. Programs come from the registry
//! keys Windows' "Apps & features" reads (`...\CurrentVersion\Uninstall`), which also hold
//! their uninstall command.

use std::path::Path;
use std::ptr::{null, null_mut};

use anyhow::bail;
use windows_sys::Win32::System::Registry::{
    HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
};
use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};

use super::registry::Key;

const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disk {
    /// Drive letter, uppercase.
    pub letter: char,
    pub total: u64,
    pub free: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub name: String,
    pub publisher: Option<String>,
    /// Bytes, as estimated by the installer (`EstimatedSize`). Often missing.
    pub size: Option<u64>,
    /// Drive it is installed on, from its install folder or icon.
    pub drive: Option<char>,
    /// Install folder, when the installer recorded it (`InstallLocation`).
    pub location: Option<String>,
    /// Command line that starts the program's own uninstaller.
    pub uninstall: String,
}

/// Fixed and removable drives with a letter, by letter.
pub fn disks() -> Vec<Disk> {
    let mut disks: Vec<Disk> = sysinfo::Disks::new_with_refreshed_list()
        .iter()
        .filter(|d| d.total_space() > 0)
        .filter_map(|d| {
            Some(Disk {
                letter: drive_of(&d.mount_point().to_string_lossy())?,
                total: d.total_space(),
                free: d.available_space(),
            })
        })
        .collect();
    disks.sort_by_key(|d| d.letter);
    disks.dedup_by_key(|d| d.letter);
    disks
}

/// Programs listed in "Apps & features" (64-bit, 32-bit and per-user), by name, without
/// system components and updates. Store (MSIX) apps are not there.
pub fn installed() -> Vec<Program> {
    let roots = [
        (HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY),
        (HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY),
        (HKEY_CURRENT_USER, 0),
    ];
    let mut programs: Vec<Program> = roots
        .into_iter()
        .filter_map(|(root, view)| Key::open(root, UNINSTALL_KEY, view))
        .flat_map(|key| {
            key.subkeys()
                .iter()
                .filter_map(|sub| read_program(&key, sub))
                .collect::<Vec<_>>()
        })
        .collect();
    // Stable: on a duplicate (both registry views), the first one read stays.
    programs.sort_by_key(|p| p.name.to_lowercase());
    programs.dedup_by_key(|p| p.name.to_lowercase());
    programs
}

fn read_program(key: &Key, sub: &str) -> Option<Program> {
    let sub = wide(sub);
    let text = |name: &str| key.string(&sub, name);
    let hidden = key.dword(&sub, "SystemComponent") == Some(1)
        || text("ParentKeyName").is_some()
        || text("ReleaseType").is_some_and(|t| t.contains("Update") || t.contains("Hotfix"));
    if hidden {
        return None;
    }
    let location =
        text("InstallLocation").map(|l| l.trim_matches('"').trim_end_matches('\\').to_string());
    let drive = location
        .as_deref()
        .or(text("DisplayIcon").as_deref())
        .and_then(drive_of);
    Some(Program {
        name: text("DisplayName")?,
        publisher: text("Publisher"),
        size: key
            .dword(&sub, "EstimatedSize")
            .map(|kb| u64::from(kb) * 1024),
        drive,
        location,
        uninstall: text("UninstallString")?,
    })
}

/// `C` for `C:\...` or `"c:\..."`.
pub fn drive_of(path: &str) -> Option<char> {
    let mut chars = path.trim_start_matches('"').chars();
    match (chars.next(), chars.next()) {
        (Some(letter), Some(':')) if letter.is_ascii_alphabetic() => {
            Some(letter.to_ascii_uppercase())
        }
        _ => None,
    }
}

/// The program installed in `dir`, if any.
pub fn installed_in<'a>(programs: &'a [Program], dir: &Path) -> Option<&'a Program> {
    let dir = dir.to_string_lossy();
    let dir = dir.trim_end_matches('\\');
    programs.iter().find(|p| {
        p.location
            .as_deref()
            .is_some_and(|l| l.eq_ignore_ascii_case(dir))
    })
}

/// Starts the program's uninstaller through the shell, so Windows asks for admin rights
/// (UAC) when it needs them. Returns once it is started, not when it is done.
pub fn uninstall(program: &Program) -> anyhow::Result<()> {
    let (file, params) = split_command(&program.uninstall);
    if !shell_execute(&file, &params) {
        bail!(t!("storage.uninstall_failed", name = program.name));
    }
    Ok(())
}

/// Opens `file` (exe, shortcut, URI) with `params`, like the Run dialog: Windows asks for
/// elevation (UAC) when the program needs it. Returns whether it started.
pub(super) fn shell_execute(file: &str, params: &str) -> bool {
    let (file, params) = (wide(file), wide(params));
    // SAFETY: every pointer is a nul-terminated UTF-16 string alive for the call.
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            null(),
            file.as_ptr(),
            params.as_ptr(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    // Above 32: started. Otherwise an error code, including a UAC prompt refused.
    result as isize > 32
}

/// Exe and arguments of an uninstall command. The exe may be quoted, or an unquoted
/// path with spaces ending in `.exe` (some installers write it that way).
fn split_command(command: &str) -> (String, String) {
    let command = command.trim();
    if let Some(rest) = command.strip_prefix('"') {
        let (file, params) = rest.split_once('"').unwrap_or((rest, ""));
        return (file.into(), params.trim().into());
    }
    let end = match command.to_ascii_lowercase().find(".exe") {
        Some(i) => i + 4,
        None => command.find(' ').unwrap_or(command.len()),
    };
    (command[..end].into(), command[end..].trim().into())
}

pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// Whether `HKEY_CLASSES_ROOT\{name}` exists, such as a registered URI scheme.
pub fn class_exists(name: &str) -> bool {
    Key::open(HKEY_CLASSES_ROOT, name, 0).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_uninstall_commands() {
        let s = |file: &str, params: &str| (file.to_string(), params.to_string());
        let cases = [
            (
                r#""C:\Program Files\App\unins000.exe" /SILENT"#,
                s(r"C:\Program Files\App\unins000.exe", "/SILENT"),
            ),
            (
                r#""C:\Program Files\App\uninstall.exe""#,
                s(r"C:\Program Files\App\uninstall.exe", ""),
            ),
            (
                r"MsiExec.exe /X{1234-ABCD}",
                s("MsiExec.exe", "/X{1234-ABCD}"),
            ),
            (
                r"C:\Program Files\Bad Vendor\Uninstall.EXE --remove",
                s(r"C:\Program Files\Bad Vendor\Uninstall.EXE", "--remove"),
            ),
            ("rundll32 x.dll,Run", s("rundll32", "x.dll,Run")),
        ];
        for (command, expected) in cases {
            assert_eq!(split_command(command), expected, "{command}");
        }
    }

    #[test]
    fn finds_the_program_of_a_folder() {
        let program = |name: &str, location: Option<&str>| Program {
            name: name.into(),
            publisher: None,
            size: None,
            drive: Some('C'),
            location: location.map(String::from),
            uninstall: "x.exe".into(),
        };
        let programs = [
            program("Nothing", None),
            program("Git", Some(r"C:\Program Files\Git")),
        ];
        let found = |dir: &str| installed_in(&programs, Path::new(dir)).map(|p| p.name.as_str());
        assert_eq!(found(r"c:\program files\git"), Some("Git"));
        assert_eq!(found("C:\\Program Files\\Git\\"), Some("Git"));
        assert_eq!(found(r"C:\Program Files\Git\bin"), None);
        assert_eq!(found(r"C:\Program Files"), None);
    }

    #[test]
    fn drive_letter_of_paths() {
        assert_eq!(drive_of(r"C:\Program Files\App"), Some('C'));
        assert_eq!(drive_of(r#""d:\Games\x.exe",0"#), Some('D'));
        assert_eq!(drive_of(r"\\server\share"), None);
        assert_eq!(drive_of("app.exe"), None);
        assert_eq!(drive_of(""), None);
    }

    #[test]
    fn reads_this_pc() {
        assert!(disks().iter().any(|d| d.total >= d.free && d.total > 0));
        for program in installed() {
            assert!(!program.name.is_empty() && !program.uninstall.is_empty());
        }
    }
}
