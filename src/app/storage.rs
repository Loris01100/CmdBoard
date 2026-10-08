//! Storage screen: drives and installed programs, read in a short-lived thread each time
//! the screen opens, uninstalling, and the folder browser that measures folder sizes.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{self, AtomicBool},
};

use anyhow::{Context, bail};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::TableState;

use super::{App, MsgKind, Outcome, clamp, step};
use crate::command::Command;
use crate::event::AppEvent;
use crate::launcher::{
    folders::{self, Entry},
    programs::{self, Disk, Program},
};

#[derive(Debug, Default)]
pub struct StorageScreen {
    pub disks: Vec<Disk>,
    pub programs: Vec<Program>,
    pub state: TableState,
    /// Programs of this drive only; `None`: every drive.
    pub disk: Option<char>,
    /// Smallest programs first instead of biggest.
    pub ascending: bool,
    pub scanning: bool,
    /// `Some`: the folder browser replaces the programs.
    pub folders: Option<Folders>,
    /// Folder sizes measured so far, kept while browsing back and forth.
    pub(super) folder_sizes: HashMap<PathBuf, u64>,
}

/// Folder browser of the Storage screen.
#[derive(Debug)]
pub struct Folders {
    /// `None`: the list of drives.
    pub dir: Option<PathBuf>,
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
    pub fn on_scanned(&mut self, disks: Vec<Disk>, programs: Vec<Program>) {
        self.scanning = false;
        self.disks = disks;
        self.programs = programs;
        if self
            .disk
            .is_some_and(|letter| !self.disks.iter().any(|d| d.letter == letter))
        {
            self.disk = None; // drive unplugged
        }
        let visible = self.visible_programs().len();
        self.state.select(clamp(self.state.selected(), visible));
        // The drives list of the browser shows what is used on each drive.
        if let Some(folders) = self.folders.as_mut().filter(|f| f.dir.is_none()) {
            folders.entries = self.disks.iter().map(drive_entry).collect();
            let selected = self.selected_entry().map(|e| e.path.clone());
            self.select_entry(selected.as_deref());
        }
    }

    /// Programs of the chosen drive, by size (unknown sizes last), ties by name.
    pub fn visible_programs(&self) -> Vec<&Program> {
        let mut list: Vec<&Program> = self
            .programs
            .iter()
            .filter(|p| self.disk.is_none() || p.drive == self.disk)
            .collect();
        // `programs` is sorted by name and the sort is stable.
        list.sort_by(|a, b| by_size(a.size, b.size, self.ascending));
        list
    }

    /// Folder browser entries, by size like the programs (unmeasured last).
    pub fn visible_entries(&self) -> Vec<&Entry> {
        let Some(folders) = &self.folders else {
            return Vec::new();
        };
        let mut list: Vec<&Entry> = folders.entries.iter().collect();
        list.sort_by(|a, b| by_size(a.size, b.size, self.ascending));
        list
    }

    pub(super) fn selected_entry(&self) -> Option<&Entry> {
        let i = self.folders.as_ref()?.state.selected()?;
        self.visible_entries().get(i).copied()
    }

    /// Selects the entry at `path`, else the first one.
    fn select_entry(&mut self, path: Option<&Path>) {
        let entries = self.visible_entries();
        let index = path
            .and_then(|p| entries.iter().position(|e| e.path == p))
            .or((!entries.is_empty()).then_some(0));
        if let Some(folders) = &mut self.folders {
            folders.state.select(index);
        }
    }

    fn reset_selection(&mut self) {
        let selected = (!self.visible_programs().is_empty()).then_some(0);
        self.state = TableState::default().with_selected(selected);
    }

    pub(super) fn move_selection(&mut self, forward: bool) {
        if self.folders.is_some() {
            let len = self.visible_entries().len();
            if let Some(folders) = &mut self.folders {
                folders
                    .state
                    .select(step(folders.state.selected(), len, forward));
            }
        } else {
            let len = self.visible_programs().len();
            self.state.select(step(self.state.selected(), len, forward));
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
        entries.sort_by_cached_key(|e| e.name.to_lowercase());
        for entry in entries.iter_mut().filter(|e| e.is_dir) {
            entry.size = self.folder_sizes.get(&entry.path).copied();
        }
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
        if let Some(folders) = &mut self.folders {
            if let Some(entry) = folders.entries.iter_mut().find(|e| e.path == path) {
                entry.size = Some(size);
            }
            folders.progress.remove(&path);
        }
        self.folder_sizes.insert(path, size);
        self.select_entry(selected.as_deref());
    }

    /// Forgets the measured sizes of `path`, of what it holds and of the folders
    /// holding it; `None`: every size.
    fn forget_sizes(&mut self, path: Option<&Path>) {
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
    /// Storage screen: Tab/←→ pick the drive, `s` the order, `d`/Del uninstall,
    /// `f` the folder browser.
    pub(super) fn storage_key(&self, key: KeyEvent) -> Option<Command> {
        if self.storage.folders.is_some() {
            return self.folder_key(key);
        }
        Some(match key.code {
            KeyCode::Char('f') => Command::ToggleFolders,
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                Command::CycleDisk { forward: true }
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                Command::CycleDisk { forward: false }
            }
            KeyCode::Char('s') => Command::ToggleStorageOrder,
            KeyCode::Char('r') => Command::RefreshStorage,
            KeyCode::Char('d') | KeyCode::Delete => {
                let i = self.storage.state.selected()?;
                Command::Uninstall {
                    program: self.storage.visible_programs().get(i)?.name.clone(),
                    confirmed: false,
                }
            }
            _ => return None,
        })
    }

    /// Folder browser: Enter/→ open, Backspace/← back, `d`/Del to the Recycle Bin (or
    /// uninstall, for a program's folder).
    fn folder_key(&self, key: KeyEvent) -> Option<Command> {
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

    /// Reads the disks and installed programs in a short-lived thread.
    pub(super) fn start_storage_scan(&mut self) {
        if !self.storage.scanning {
            self.storage.scanning = self.spawn(|| {
                let (disks, programs) = rayon::join(programs::disks, programs::installed);
                AppEvent::StorageScanned { disks, programs }
            });
        }
    }

    /// `r`: reads the drives, the programs and the shown folder again, its sizes measured
    /// anew. For what changed outside CmdBoard: an uninstaller that finished, a file
    /// deleted elsewhere, the Recycle Bin emptied.
    pub(super) fn refresh_storage(&mut self) {
        self.start_storage_scan();
        let Some(folders) = &self.storage.folders else {
            return;
        };
        let dir = folders.dir.clone();
        self.storage.forget_sizes(dir.as_deref());
        if dir.is_some() {
            let selected = self.storage.selected_entry().map(|e| e.path.clone());
            self.open_folder(dir, selected);
        }
    }

    pub(super) fn cycle_disk(&mut self, forward: bool) {
        // `None` (every drive) sits before the first drive.
        let mut choices = vec![None];
        choices.extend(self.storage.disks.iter().map(|d| Some(d.letter)));
        let current = choices.iter().position(|&c| c == self.storage.disk);
        let next = step(current, choices.len(), forward).unwrap_or(0);
        self.storage.disk = choices[next];
        self.storage.reset_selection();
    }

    pub(super) fn toggle_storage_order(&mut self) {
        self.storage.ascending = !self.storage.ascending;
        self.storage.reset_selection();
        self.storage.select_entry(None);
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
            self.open_folder(dir.parent().map(Path::to_path_buf), Some(dir));
        }
    }

    /// Shows `dir` (`None`: the drives) and reads it in a short-lived thread. Stops the
    /// measures of the folder being left.
    pub(super) fn open_folder(&mut self, dir: Option<PathBuf>, select: Option<PathBuf>) {
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
            select: select.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
            progress: HashMap::new(),
        });
        self.storage.select_entry(select.as_deref());
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

    pub(super) fn uninstall(&mut self, program: &str, confirmed: bool) -> Outcome {
        let lower = program.to_lowercase();
        let found = self
            .storage
            .programs
            .iter()
            .find(|p| p.name.to_lowercase() == lower)
            .with_context(|| t!("storage.unknown_program", name = program))?;
        let name = found.name.clone();
        if !confirmed {
            // The exact command: any program running as the user can rewrite it.
            let message = t!("storage.confirm_uninstall", name, command = found.uninstall);
            let program = name;
            return self.confirm(
                message,
                Command::Uninstall {
                    program,
                    confirmed: true,
                },
            );
        }
        programs::uninstall(found)?;
        Ok(Some((t!("storage.uninstalling", name), MsgKind::Info)))
    }
}

/// A drive in the folder browser, its size what is used on it.
fn drive_entry(disk: &Disk) -> Entry {
    Entry {
        name: format!("{}:", disk.letter),
        path: PathBuf::from(format!("{}:\\", disk.letter)),
        is_dir: true,
        size: Some(disk.total - disk.free.min(disk.total)),
    }
}

/// Biggest first (or smallest, if `ascending`), unknown sizes always last.
fn by_size(a: Option<u64>, b: Option<u64>, ascending: bool) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) if ascending => x.cmp(&y),
        (Some(x), Some(y)) => y.cmp(&x),
        (a, b) => b.is_some().cmp(&a.is_some()),
    }
}
