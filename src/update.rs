//! `:update` and the daily passive check, against GitHub Releases (plan section 18).
//! Network work runs in a short-lived thread that answers with `AppEvent::UpdateFinished`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread;

use anyhow::Context;
use self_update::backends::github::Update;

use crate::event::AppEvent;

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// The passive check runs at most once per this many seconds.
const CHECK_EVERY_SECS: i64 = 24 * 3600;

/// The archive `dist` publishes for us; the `.msi` and `.sha256` assets must not match.
const ZIP_ASSET: &str = "cmdboard-x86_64-pc-windows-msvc.zip";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Passive check at startup: only reports a newer version.
    Check,
    /// `:update`: replaces the exe, unless the install is managed by MSI/winget.
    Install,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    UpToDate,
    /// A newer version exists. `managed`: installed under Program Files, update with winget.
    Available {
        version: String,
        managed: bool,
    },
    /// The exe was replaced; the new version runs after a restart.
    Installed {
        version: String,
    },
}

pub fn spawn(tx: Sender<AppEvent>, action: Action) {
    thread::spawn(move || {
        let result = run(action).map_err(|e| format!("{e:#}"));
        let _ = tx.send(AppEvent::UpdateFinished { action, result });
    });
}

fn run(action: Action) -> anyhow::Result<Outcome> {
    let updater = Update::configure()
        .repo_owner("Loris01100")
        .repo_name("CmdBoard")
        .bin_name("cmdboard")
        .current_version(CURRENT)
        .asset_matcher(|assets| assets.iter().find(|a| a.name() == ZIP_ASSET).cloned())
        // We own the terminal: no output, no stdin prompt.
        .show_output(false)
        .show_download_progress(false)
        .no_confirm(true)
        .build()?;
    let releases = updater
        .get_latest_release()
        .context("impossible de joindre GitHub")?;
    if !releases.is_update_available()? {
        return Ok(Outcome::UpToDate);
    }
    let version = releases.latest().map_or("?", |r| r.version()).to_string();
    let managed = is_managed_install(&std::env::current_exe()?, &program_files());
    if action == Action::Check || managed {
        return Ok(Outcome::Available { version, managed });
    }
    // `self_update` gets past Windows' lock on the running exe (via `self_replace`).
    updater.update().context("échec de la mise à jour")?;
    Ok(Outcome::Installed { version })
}

fn program_files() -> Vec<PathBuf> {
    ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
        .iter()
        .filter_map(|var| std::env::var_os(var).map(PathBuf::from))
        .collect()
}

/// MSI and winget install under Program Files: replacing the exe there needs admin
/// rights and would desync winget.
fn is_managed_install(exe: &Path, program_files: &[PathBuf]) -> bool {
    let exe = exe.to_string_lossy().to_lowercase();
    program_files.iter().any(|dir| {
        let dir = dir.to_string_lossy().to_lowercase();
        let dir = dir.trim_end_matches('\\');
        !dir.is_empty() && exe.starts_with(&format!("{dir}\\"))
    })
}

/// Whether the passive check should run, given when it last ran (unix seconds).
pub fn check_due(last: Option<i64>, now: i64) -> bool {
    last.is_none_or(|last| now - last >= CHECK_EVERY_SECS || now < last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_files_installs_are_managed() {
        let dirs = [
            PathBuf::from(r"C:\Program Files"),
            PathBuf::from(r"C:\Program Files (x86)\"),
        ];
        let managed = |exe: &str| is_managed_install(Path::new(exe), &dirs);
        assert!(managed(r"C:\Program Files\cmdboard\bin\cmdboard.exe"));
        assert!(managed(r"c:\program files (x86)\CmdBoard\cmdboard.exe"));
        assert!(!managed(r"C:\Users\me\.cargo\bin\cmdboard.exe"));
        assert!(!managed(r"C:\Program Files Extra\cmdboard.exe"));
        assert!(!is_managed_install(
            Path::new(r"C:\x\cmdboard.exe"),
            &[PathBuf::new()]
        ));
    }

    #[test]
    fn passive_check_runs_once_a_day() {
        let day = CHECK_EVERY_SECS;
        assert!(check_due(None, 1000));
        assert!(!check_due(Some(1000), 1000 + day - 1));
        assert!(check_due(Some(1000), 1000 + day));
        assert!(check_due(Some(1000 + day), 1000)); // clock went back
    }
}
