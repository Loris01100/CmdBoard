use std::path::Path;

use anyhow::{Context, bail};

/// Hands `target` (exe path, exe name on the PATH, or URI such as `steam://...`) to the
/// Windows shell, like double-clicking it. Returns once the process is started.
pub fn launch(target: &str) -> anyhow::Result<()> {
    check_target(target)?;
    let target = target.trim();
    opener::open(target).with_context(|| format!("impossible de lancer {target}"))
}

/// Rejects empty targets and absolute paths that do not exist.
/// Bare exe names and URIs are left to the shell to resolve.
pub fn check_target(target: &str) -> anyhow::Result<()> {
    let target = target.trim();
    if target.is_empty() {
        bail!("aucune cible de lancement");
    }
    let path = Path::new(target);
    if path.is_absolute() && !path.exists() {
        bail!("fichier introuvable : {target}");
    }
    Ok(())
}

/// Whether `target` can launch on this PC: a URI whose scheme is registered, an existing
/// absolute path, or an exe name found on the `PATH`.
pub fn is_available(target: &str) -> bool {
    let target = target.trim();
    if let Some((scheme, _)) = target.split_once("://") {
        return std::process::Command::new("reg")
            .args(["query", &format!(r"HKCR\{scheme}")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    }
    let path = Path::new(target);
    if path.is_absolute() {
        return path.exists();
    }
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| dir.join(target).symlink_metadata().is_ok())
    })
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
        assert!(launch("  ").is_err());
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
