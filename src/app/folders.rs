//! Folder browser of the Storage screen: lists a folder in a short-lived thread, then
//! measures its subfolders, one `FolderSized` event each, kept in display order as they
//! come; sends files and folders to the Recycle Bin.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{self, AtomicBool},
};

use anyhow::bail;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::TableState;

use super::storage::{StorageScreen, by_size};
use super::{App, MsgKind, Outcome};
use crate::command::Command;
use crate::event::AppEvent;
use crate::launcher::{
    folders::{self, Entry},
    programs::{self, Disk},
};

/// Folder browser of the Storage screen.
#[derive(Debug)]
pub struct Folders {
    /// `None`: the list of drives.
    pub dir: Option<PathBuf>,
    /// In display order (`entry_order`).
    pub entries: Vec<Entry>,
    pub state: TableState,
    /// Still reading `dir`.
    pub listing: bool,
    /// Entry to select once listed (the folder we came back from).
    select: Option<PathBuf>,
    /// Set when leaving `dir`: its measures stop.
    cancel: Arc<AtomicBool>,
    /// Subfolders being measured: percentage of their children done.
    pub progress: HashMap<PathBuf, u8>,
}

impl StorageScreen {
    /// Folder browser entries, by size like the programs (unmeasured last).
    pub fn visible_entries(&self) -> &[Entry] {
        match &self.folders {
            Some(folders) => &folders.entries,
            None => &[],
        }
    }

    pub(super) fn sort_entries(&mut self) {
        let ascending = self.ascending;
        if let Some(folders) = &mut self.folders {
            folders.entries.sort_by(|a, b| entry_order(a, b, ascending));
        }
    }

    pub(super) fn selected_entry(&self) -> Option<&Entry> {
        let i = self.folders.as_ref()?.state.selected()?;
        self.visible_entries().get(i)
    }

    /// Selects the entry at `path`, else the first one.
    pub(super) fn select_entry(&mut self, path: Option<&Path>) {
        let entries = self.visible_entries();
        let index = path
            .and_then(|p| entries.iter().position(|e| e.path == p))
            .or((!entries.is_empty()).then_some(0));
        if let Some(folders) = &mut self.folders {
            folders.state.select(index);
        }
    }

    fn stop_measures(&self) {
        if let Some(folders) = &self.folders {
            folders.cancel.store(true, atomic::Ordering::Relaxed);
        }
    }

    /// Fills in a listed folder with the sizes already known. Returns the subfolders still
    /// to measure, and their cancel flag; `None` if the browser left `dir` meanwhile.
    fn fill_listing(
        &mut self,
        dir: &Path,
        mut entries: Vec<Entry>,
    ) -> Option<(Vec<PathBuf>, Arc<AtomicBool>)> {
        let folders = self.folders.as_mut()?;
        if folders.dir.as_deref() != Some(dir) {
            return None;
        }
        for entry in entries.iter_mut().filter(|e| e.is_dir) {
            entry.size = self.folder_sizes.get(&entry.path).copied();
        }
        entries.sort_by(|a, b| entry_order(a, b, self.ascending));
        let todo = entries
            .iter()
            .filter(|e| e.is_dir && e.size.is_none())
            .map(|e| e.path.clone())
            .collect();
        folders.entries = entries;
        folders.listing = false;
        let (select, cancel) = (folders.select.take(), folders.cancel.clone());
        self.select_entry(select.as_deref());
        Some((todo, cancel))
    }

    pub fn on_folder_progress(&mut self, path: PathBuf, percent: u8) {
        if let Some(folders) = &mut self.folders {
            folders.progress.insert(path, percent);
        }
    }

    /// Keeps the selection on the same entry while the list reorders.
    pub fn on_folder_sized(&mut self, path: PathBuf, size: u64) {
        let selected = self.selected_entry().map(|e| e.path.clone());
        let ascending = self.ascending;
        if let Some(folders) = &mut self.folders {
            if let Some(i) = folders.entries.iter().position(|e| e.path == path) {
                // Moved to its new place: the others stay in order, no full sort.
                let mut entry = folders.entries.remove(i);
                entry.size = Some(size);
                let at = folders
                    .entries
                    .partition_point(|e| entry_order(e, &entry, ascending).is_lt());
                folders.entries.insert(at, entry);
            }
            folders.progress.remove(&path);
        }
        self.folder_sizes.insert(path, size);
        self.select_entry(selected.as_deref());
    }

    /// Forgets the measured sizes of `path`, of what it holds and of the folders
    /// holding it; `None`: every size.
    pub(super) fn forget_sizes(&mut self, path: Option<&Path>) {
        match path {
            Some(path) => self
                .folder_sizes
                .retain(|p, _| !path.starts_with(p) && !p.starts_with(path)),
            None => self.folder_sizes.clear(),
        }
    }

    /// After `path` went to the Recycle Bin: drops it, and the sizes it made stale.
    fn forget(&mut self, path: &Path) {
        self.forget_sizes(Some(path));
        let selected = self.selected_entry().map(|e| e.path.clone());
        if let Some(folders) = &mut self.folders {
            folders.entries.retain(|e| e.path != path);
        }
        self.select_entry(selected.as_deref());
    }
}

impl App {
    /// Folder browser: Enter/→ open, Backspace/← back, `d`/Del to the Recycle Bin (or
    /// uninstall, for a program's folder).
    pub(super) fn folder_key(&self, key: KeyEvent) -> Option<Command> {
        Some(match key.code {
            KeyCode::Char('f') => Command::ToggleFolders,
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => Command::OpenFolder,
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => Command::ParentFolder,
            KeyCode::Char('s') => Command::ToggleStorageOrder,
            KeyCode::Char('r') => Command::RefreshStorage,
            KeyCode::Char('d') | KeyCode::Delete => {
                self.storage.folders.as_ref()?.dir.as_ref()?; // drives cannot be deleted
                let entry = self.storage.selected_entry()?;
                match programs::installed_in(&self.storage.programs, &entry.path) {
                    Some(program) => Command::Uninstall {
                        program: program.name.clone(),
                        confirmed: false,
                    },
                    None => Command::Trash {
                        path: entry.path.clone(),
                        confirmed: false,
                    },
                }
            }
            _ => return None,
        })
    }

    pub(super) fn toggle_folders(&mut self) {
        if self.storage.folders.is_some() {
            self.storage.stop_measures();
            self.storage.folders = None;
            return;
        }
        // Starts on the chosen drive, else on the list of drives.
        let root = self
            .storage
            .disk
            .map(|letter| PathBuf::from(format!("{letter}:\\")));
        self.open_folder(root, None);
    }

    pub(super) fn open_selected_folder(&mut self) {
        if let Some(entry) = self.storage.selected_entry().filter(|e| e.is_dir) {
            let path = entry.path.clone();
            self.open_folder(Some(path), None);
        }
    }

    pub(super) fn parent_folder(&mut self) {
        if let Some(dir) = self.storage.folders.as_ref().and_then(|f| f.dir.clone()) {
            // `C:\` has no parent: back to the drives.
            self.open_folder(dir.parent().map(Path::to_path_buf), Some(&dir));
        }
    }

    /// Shows `dir` (`None`: the drives) and reads it in a short-lived thread. Stops the
    /// measures of the folder being left.
    pub(super) fn open_folder(&mut self, dir: Option<PathBuf>, select: Option<&Path>) {
        self.storage.stop_measures();
        let entries = match &dir {
            Some(_) => Vec::new(),
            None => self.storage.disks.iter().map(drive_entry).collect(),
        };
        self.storage.folders = Some(Folders {
            listing: dir.is_some(),
            dir: dir.clone(),
            entries,
            state: TableState::default(),
            select: select.map(Path::to_path_buf),
            cancel: Arc::new(AtomicBool::new(false)),
            progress: HashMap::new(),
        });
        self.storage.sort_entries();
        self.storage.select_entry(select);
        if let Some(dir) = dir {
            self.spawn(move || {
                let entries = folders::list(&dir);
                AppEvent::FolderListed { dir, entries }
            });
        }
    }

    /// Shows a listed folder, then measures its other subfolders in a short-lived
    /// thread, one `FolderSized` each.
    pub fn on_folder_listed(&mut self, dir: &Path, entries: Vec<Entry>) {
        let Some((todo, cancel)) = self.storage.fill_listing(dir, entries) else {
            return; // left meanwhile
        };
        let Some(events) = self.events.clone() else {
            return;
        };
        std::thread::spawn(move || {
            use rayon::prelude::*;
            todo.into_par_iter().for_each(|path| {
                let progress = |percent| {
                    let path = path.clone();
                    let _ = events.send(AppEvent::FolderProgress { path, percent });
                };
                if let Some(size) = folders::dir_size_with_progress(&path, &cancel, progress) {
                    let _ = events.send(AppEvent::FolderSized { path, size });
                }
            });
        });
    }

    pub(super) fn trash(&mut self, path: PathBuf, confirmed: bool) -> Outcome {
        // Refused before asking; `folders::trash` checks again.
        if folders::is_protected(&path) {
            bail!(t!("storage.protected", path = path.display()));
        }
        if !confirmed {
            let message = t!("storage.confirm_trash", path = path.display());
            return self.confirm(
                message,
                Command::Trash {
                    path,
                    confirmed: true,
                },
            );
        }
        let text = t!("storage.trashing", path = path.display());
        let started = self.spawn(move || {
            let result = folders::trash(&path).map_err(|e| format!("{e:#}"));
            AppEvent::Trashed { path, result }
        });
        if !started {
            bail!(t!("storage.unavailable"));
        }
        Ok(Some((text, MsgKind::Info)))
    }

    pub(super) fn on_trashed(&mut self, path: &Path, result: Result<(), String>) {
        self.message = Some(match result {
            Ok(()) => {
                self.storage.forget(path);
                self.start_storage_scan(); // the drive gauges
                (
                    t!("storage.trashed", path = path.display()),
                    MsgKind::Success,
                )
            }
            Err(error) => (error, MsgKind::Error),
        });
    }
}

/// A drive in the folder browser, its size what is used on it.
pub(super) fn drive_entry(disk: &Disk) -> Entry {
    Entry {
        name: format!("{}:", disk.letter),
        path: PathBuf::from(format!("{}:\\", disk.letter)),
        is_dir: true,
        size: Some(disk.total - disk.free.min(disk.total)),
    }
}

/// Folder browser order: by size, ties by name ignoring case.
fn entry_order(a: &Entry, b: &Entry, ascending: bool) -> Ordering {
    by_size(a.size, b.size, ascending).then_with(|| lowercase(&a.name).cmp(lowercase(&b.name)))
}

fn lowercase(name: &str) -> impl Iterator<Item = char> + '_ {
    name.chars().flat_map(char::to_lowercase)
}
