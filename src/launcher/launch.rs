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
