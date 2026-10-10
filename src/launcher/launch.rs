use std::path::Path;

use anyhow::{Context, bail};

use super::programs;

/// Hands `target` (exe path, exe name on the PATH, or URI such as `steam://...`) to the
/// Windows shell, like double-clicking it. Returns once the process is started.
/// `args` (`-dx12`, a folder to open) go to the program as its command line.
pub fn launch(target: &str, args: Option<&str>) -> anyhow::Result<()> {
    check_target(target)?;
    let target = target.trim();
    match args.map(str::trim).filter(|a| !a.is_empty()) {
        None => opener::open(target).with_context(|| t!("error.cannot_launch", target)),
        Some(args) if programs::shell_execute(target, args) => Ok(()),
        Some(_) => bail!(t!("error.cannot_launch", target)),
    }
}

/// Rejects empty targets and absolute paths that do not exist.
/// Bare exe names and URIs are left to the shell to resolve.
pub fn check_target(target: &str) -> anyhow::Result<()> {
    let target = target.trim();
    if target.is_empty() {
        bail!(t!("error.no_target"));
    }
    let path = Path::new(target);
    if path.is_absolute() && !path.exists() {
        bail!(t!("error.file_not_found", target));
    }
    Ok(())
}

/// Whether `target` can launch on this PC: a URI whose scheme is registered, an existing
/// absolute path, or an exe name found on the `PATH`.
pub fn is_available(target: &str) -> bool {
    let target = target.trim();
    if let Some((scheme, _)) = target.split_once("://") {
        return is_scheme(scheme) && programs::class_exists(scheme);
    }
    let path = Path::new(target);
    if path.is_absolute() {
        return path.exists();
    }
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| dir.join(target).symlink_metadata().is_ok())
    })
}

/// RFC 3986 scheme: a letter, then letters, digits, `+`, `-` or `.`. Keeps the registry
/// lookup to one key under `HKEY_CLASSES_ROOT`.
fn is_scheme(scheme: &str) -> bool {
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Why a launch target from someone else (an imported backup) deserves a second look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    /// `\\server\share\…`, as target or argument: runs a file from another computer.
    NetworkPath,
    /// A shell, a script host or a script file: whatever its arguments say runs.
    Script,
    /// A URI scheme other than the launchers' (`ms-msdt:`, `search-ms:`, `file:`…).
    UnknownScheme,
}

/// Programs whose arguments are themselves a program, by lowercase file stem.
const SCRIPT_HOSTS: [&str; 13] = [
    "bash",
    "cmd",
    "conhost",
    "cscript",
    "forfiles",
    "msiexec",
    "mshta",
    "powershell",
    "powershell_ise",
    "pwsh",
    "regsvr32",
    "rundll32",
    "wscript",
];

/// Files the shell runs as scripts or installers rather than opening.
const SCRIPT_EXTENSIONS: [&str; 11] = [
    "bat", "cmd", "hta", "js", "jse", "msi", "ps1", "scr", "vbe", "vbs", "wsf",
];

/// URI schemes of the game launchers (`shell:` only for `shell:AppsFolder\…`).
const SAFE_SCHEMES: [&str; 9] = [
    "battlenet",
    "com.epicgames.launcher",
    "goggalaxy",
    "http",
    "https",
    "origin",
    "origin2",
    "steam",
    "uplay",
];

/// `None` for an ordinary program or launcher link. Only judges what the strings say: the
/// user's own apps can be anything, this is for targets that come from elsewhere.
pub fn risk(target: &str, args: Option<&str>) -> Option<Risk> {
    let target = target.trim();
    let network = |text: &str| text.starts_with(r"\\") || text.starts_with("//");
    if network(target) || args.is_some_and(|a| a.contains(r"\\")) {
        return Some(Risk::NetworkPath);
    }
    // `C:\…` is a drive, not a one-letter scheme.
    if let Some((scheme, rest)) = target.split_once(':')
        && scheme.len() > 1
        && is_scheme(scheme)
    {
        let scheme = scheme.to_ascii_lowercase();
        let apps_folder = scheme == "shell"
            && rest
                .get(..11)
                .is_some_and(|s| s.eq_ignore_ascii_case(r"AppsFolder\"));
        let safe = apps_folder || SAFE_SCHEMES.contains(&scheme.as_str());
        return (!safe).then_some(Risk::UnknownScheme);
    }
    let path = Path::new(target);
    let lower = |part: Option<&std::ffi::OsStr>| part.map(|p| p.to_string_lossy().to_lowercase());
    let script = lower(path.file_stem()).is_some_and(|s| SCRIPT_HOSTS.contains(&s.as_str()))
        || lower(path.extension()).is_some_and(|e| SCRIPT_EXTENSIONS.contains(&e.as_str()));
    script.then_some(Risk::Script)
}

/// Process to track for a target: its file name when it is an exe, `None` for URIs
/// (launchers spawn another process, to be set by hand).
pub fn watch_exe_for(target: &str) -> Option<String> {
    let path = Path::new(target.trim());
    let is_exe = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"));
    if !is_exe || target.contains("://") {
        return None;
    }
    Some(path.file_name()?.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_target_is_rejected() {
        assert!(launch("  ", None).is_err());
    }

    #[test]
    fn missing_absolute_path_is_rejected() {
        assert!(check_target(r"C:\nope\does-not-exist.exe").is_err());
        assert!(check_target("notepad.exe").is_ok());
        assert!(check_target("steam://rungameid/1145360").is_ok());
    }

    #[test]
    fn availability_checks_path_and_uri_schemes() {
        assert!(is_available("notepad.exe"));
        assert!(is_available(r"C:\Windows\explorer.exe"));
        assert!(!is_available("no-such-app-cmdboard.exe"));
        assert!(!is_available(r"C:\nope\does-not-exist.exe"));
        assert!(is_available("http://example.com")); // always registered
        assert!(!is_available("no-such-scheme-cmdboard://x"));
        // Not a scheme: no other registry key is looked up.
        assert!(!is_available(r"http\shell://x"));
        assert!(!is_available("://x"));
        assert!(!is_available("1http://x"));
    }

    #[test]
    fn risky_targets_are_recognized() {
        use Risk::{NetworkPath, Script, UnknownScheme};
        for (target, args, expected) in [
            // What the scans and the forms produce.
            (r"C:\Games\Hades\Hades.exe", Some("-dx12"), None),
            ("notepad.exe", None, None),
            ("steam://rungameid/1145360", None, None),
            ("com.epicgames.launcher://apps/x?action=launch", None, None),
            ("uplay://launch/635/0", None, None),
            (
                r"shell:AppsFolder\Microsoft.WindowsNotepad_8wekyb3d8bbwe!App",
                None,
                None,
            ),
            ("https://example.com", None, None),
            (r"D:\Jeux\cmd-tools\game.exe", None, None),
            // From someone else's backup.
            (r"\\attacker\share\Game.exe", None, Some(NetworkPath)),
            ("//attacker/share/Game.exe", None, Some(NetworkPath)),
            (
                "explorer.exe",
                Some(r"\\attacker\share\x.exe"),
                Some(NetworkPath),
            ),
            ("powershell.exe", Some("-c iwr x | iex"), Some(Script)),
            (r"C:\Windows\System32\CMD.EXE", Some("/c x"), Some(Script)),
            ("rundll32", Some("x.dll,Run"), Some(Script)),
            ("mshta.exe", None, Some(Script)),
            (r"C:\Users\me\Downloads\setup.BAT", None, Some(Script)),
            (r"C:\x\payload.vbs", None, Some(Script)),
            ("ms-msdt:/id PCWDiagnostic", None, Some(UnknownScheme)),
            ("search-ms:query=x", None, Some(UnknownScheme)),
            ("file:///C:/x.exe", None, Some(UnknownScheme)),
            (r"shell:startup", None, Some(UnknownScheme)),
        ] {
            assert_eq!(risk(target, args), expected, "{target} {args:?}");
        }
    }

    #[test]
    fn watch_exe_from_target() {
        assert_eq!(
            watch_exe_for(r"C:\Program Files\VS Code\Code.EXE").as_deref(),
            Some("Code.EXE")
        );
        assert_eq!(watch_exe_for("wt.exe").as_deref(), Some("wt.exe"));
        assert_eq!(watch_exe_for("steam://rungameid/1145360"), None);
        assert_eq!(watch_exe_for(r"C:\Games\readme.txt"), None);
    }
}
