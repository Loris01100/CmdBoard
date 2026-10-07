//! Folder browser of the Storage screen: lists a folder, measures its subfolders on
//! demand, and sends items to the Recycle Bin.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

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
        .map(|e| match e.file_type() {
            Ok(kind) if kind.is_symlink() => Some(0),
            Ok(kind) if kind.is_dir() => dir_size(&e.path(), cancel),
            _ => Some(e.metadata().map_or(0, |m| m.len())),
        })
        .sum()
}

/// Sends a file or folder to the Recycle Bin. Windows warns before deleting for good
/// something too big for the bin. Blocks until done: call it off the UI thread.
pub fn trash(path: &Path) -> anyhow::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    // A double-nul-terminated list of one path.
    let from: Vec<u16> = path.as_os_str().encode_wide().chain([0, 0]).collect();
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_WANTNUKEWARNING) as u16,
        ..Default::default()
    };
    // SAFETY: `from` outlives the call; the other pointers are null.
    let status = unsafe { SHFileOperationW(&mut op) };
    if status != 0 || op.fAnyOperationsAborted != 0 {
        bail!(t!("storage.trash_failed", path = path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(dir_size(&root, &cancel), None);

        std::fs::remove_dir_all(&root).unwrap();
    }
}
