//! Folder browser of the Storage screen: lists a folder, measures its subfolders on
//! demand, and sends items to the Recycle Bin.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

use anyhow::bail;
use rayon::prelude::*;
use windows_sys::Win32::UI::Shell::{
    FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_WANTNUKEWARNING, SHFILEOPSTRUCTW,
    SHFileOperationW,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    /// Bytes. `None` for a folder not measured yet.
    pub size: Option<u64>,
}

/// Files (with their size) and subfolders (unmeasured) of `dir`. Links and junctions are
/// skipped: they point elsewhere, and some loop (`Application Data`).
pub fn list(dir: &Path) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    read.flatten()
        .filter_map(|e| {
            let kind = e.file_type().ok()?;
            if kind.is_symlink() {
                return None;
            }
            Some(Entry {
                name: e.file_name().to_string_lossy().into_owned(),
                path: e.path(),
                is_dir: kind.is_dir(),
                size: if kind.is_dir() {
                    None
                } else {
                    Some(e.metadata().map_or(0, |m| m.len()))
                },
            })
        })
        .collect()
}

/// Total size of the files under `dir`, in parallel. Unreadable folders count as empty.
/// `None` once `cancel` is set (the user left the folder).
pub fn dir_size(dir: &Path, cancel: &AtomicBool) -> Option<u64> {
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        return Some(0);
    };
    read.flatten()
        .par_bridge()
        .map(|e| entry_size(&e, cancel))
        .sum()
}

/// `dir_size`, calling `progress` with the percentage of `dir`'s direct children measured
/// each time it goes up.
// ponytail: progress counts children, not bytes; one huge subfolder can sit at 95 %.
pub fn dir_size_with_progress(
    dir: &Path,
    cancel: &AtomicBool,
    progress: impl Fn(u8) + Sync,
) -> Option<u64> {
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        return Some(0);
    };
    let children: Vec<_> = read.flatten().collect();
    let (done, shown) = (AtomicUsize::new(0), AtomicU8::new(0));
    children
        .par_iter()
        .map(|e| {
            let size = entry_size(e, cancel);
            let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
            let percent = u8::try_from(finished * 100 / children.len()).unwrap_or(100);
            if shown.fetch_max(percent, Ordering::Relaxed) < percent {
                progress(percent);
            }
            size
        })
        .sum()
}

fn entry_size(e: &std::fs::DirEntry, cancel: &AtomicBool) -> Option<u64> {
    match e.file_type() {
        Ok(kind) if kind.is_symlink() => Some(0),
        Ok(kind) if kind.is_dir() => dir_size(&e.path(), cancel),
        _ => Some(e.metadata().map_or(0, |m| m.len())),
    }
}

/// Sends a file or folder to the Recycle Bin. Windows warns before deleting for good
/// something too big for the bin. Blocks until done: call it off the UI thread.
pub fn trash(path: &Path) -> anyhow::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    if is_protected(path) {
        bail!(t!("storage.protected", path = path.display()));
    }
    // A double-nul-terminated list of one path.
    let from: Vec<u16> = path.as_os_str().encode_wide().chain([0, 0]).collect();
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        fFlags: u16::try_from(FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_WANTNUKEWARNING)?,
        ..Default::default()
    };
    // SAFETY: `from` outlives the call; the other pointers are null.
    let status = unsafe { SHFileOperationW(&raw mut op) };
    if status != 0 || op.fAnyOperationsAborted != 0 {
        bail!(t!("storage.trash_failed", path = path.display()));
    }
    Ok(())
}

/// Whether `path` must stay out of the Recycle Bin: a drive root, anything inside
/// Windows or `CmdBoard`'s data (its database), or a folder holding Program Files,
/// `ProgramData`, the user folder or `CmdBoard` itself. Paths are resolved first, so short
/// names, `..` and junctions do not get around it.
pub fn is_protected(path: &Path) -> bool {
    let env = |var| std::env::var_os(var).map(PathBuf::from);
    let inside: Vec<PathBuf> = [env("SystemRoot"), crate::storage::db::data_dir().ok()]
        .into_iter()
        .flatten()
        .collect();
    let mut kept: Vec<PathBuf> = [
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "ProgramData",
        "USERPROFILE",
        "PUBLIC",
    ]
    .into_iter()
    .filter_map(env)
    .collect();
    kept.extend(std::env::current_exe().ok());
    let resolve_all = |dirs: &[PathBuf]| dirs.iter().map(|d| resolve(d)).collect::<Vec<_>>();
    protected_by(&resolve(path), &resolve_all(&inside), &resolve_all(&kept))
}

/// `path` is a root or relative, is inside one of `inside`, or holds one of `inside`
/// or `kept`.
fn protected_by(path: &Path, inside: &[PathBuf], kept: &[PathBuf]) -> bool {
    !path.is_absolute()
        || path.parent().is_none()
        || inside.iter().any(|dir| path.starts_with(dir))
        || inside.iter().chain(kept).any(|dir| dir.starts_with(path))
}

/// The real path (long names, links followed) in lowercase, as Windows ignores case.
/// A path that does not exist stays as written.
fn resolve(path: &Path) -> PathBuf {
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = real.to_string_lossy().to_lowercase();
    // `canonicalize` answers `\\?\C:\…`; the variables hold `C:\…`.
    let text = match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with(r"unc\") => rest.to_string(),
        _ => text,
    };
    PathBuf::from(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_cancelled_folders() {
        let missing = Path::new(r"C:\cmdboard-does-not-exist");
        assert!(list(missing).is_empty());
        let (go, stop) = (AtomicBool::new(false), AtomicBool::new(true));
        assert_eq!(dir_size_with_progress(missing, &go, |_| {}), Some(0));
        assert_eq!(dir_size_with_progress(missing, &stop, |_| {}), None);
        assert!(trash(missing).is_err());
    }

    #[test]
    fn critical_folders_are_protected() {
        let p = PathBuf::from;
        let inside = [p(r"c:\windows"), p(r"c:\users\me\appdata\roaming\cmdboard")];
        let kept = [p(r"c:\program files"), p(r"c:\users\me")];
        let protected = |path: &str| protected_by(Path::new(path), &inside, &kept);
        for path in [
            r"c:\",
            r"d:\",
            r"windows\system32",
            r"c:\windows",
            r"c:\windows\system32\drivers",
            r"c:\users\me\appdata\roaming\cmdboard\cmdboard.db",
            r"c:\users\me\appdata\roaming",
            r"c:\users\me",
            r"c:\users",
            r"c:\program files",
        ] {
            assert!(protected(path), "{path}");
        }
        for path in [
            r"c:\program files\old game",
            r"c:\users\me\downloads",
            r"c:\users\me\appdata\roaming\other",
            r"c:\windowsapps-backup",
            r"d:\games",
        ] {
            assert!(!protected(path), "{path}");
        }

        // The real folders, however they are written.
        let windows = PathBuf::from(std::env::var("SystemRoot").unwrap());
        let upper = PathBuf::from(windows.to_string_lossy().to_uppercase());
        for path in [
            upper,
            windows.join("System32"),
            windows.join(r"System32\.."),
            PathBuf::from(std::env::var("USERPROFILE").unwrap()),
            std::env::current_exe().unwrap(),
            PathBuf::from(r"C:\"),
        ] {
            assert!(is_protected(&path), "{}", path.display());
        }
        assert!(!is_protected(&std::env::temp_dir().join("cmdboard-free")));
        let error = trash(&windows).unwrap_err().to_string();
        assert!(error.contains("protégé"), "{error}");
    }

    #[test]
    fn lists_and_measures_a_folder() {
        let root = std::env::temp_dir().join(format!("cmdboard-folders-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(r"sub\deeper")).unwrap();
        std::fs::write(root.join("a.txt"), [0u8; 100]).unwrap();
        std::fs::write(root.join(r"sub\b.bin"), [0u8; 1000]).unwrap();
        std::fs::write(root.join(r"sub\deeper\c.bin"), [0u8; 24]).unwrap();

        let mut entries = list(&root);
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(entries.len(), 2);
        assert_eq!((entries[0].is_dir, entries[0].size), (false, Some(100)));
        assert_eq!((entries[1].is_dir, entries[1].size), (true, None));

        let cancel = AtomicBool::new(false);
        assert_eq!(dir_size(&root.join("sub"), &cancel), Some(1024));
        assert_eq!(dir_size(&root, &cancel), Some(1124));
        assert_eq!(dir_size(&root.join("missing"), &cancel), Some(0));
        let seen = std::sync::Mutex::new(Vec::new());
        let size = dir_size_with_progress(&root, &cancel, |p| seen.lock().unwrap().push(p));
        assert_eq!(size, Some(1124));
        // Two children: 50 then 100, unless both finish together.
        assert_eq!(seen.into_inner().unwrap().iter().max(), Some(&100));
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(dir_size(&root, &cancel), None);

        std::fs::remove_dir_all(&root).unwrap();
    }
}
