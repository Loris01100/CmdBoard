//! Storage screen: drives and installed programs, read in a short-lived thread each time
//! the screen opens, and uninstalling. The folder browser is in `folders`.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Context;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::TableState;

use super::folders::{Folders, drive_entry};
use super::{App, MsgKind, Outcome, clamp, step};
use crate::command::Command;
use crate::event::AppEvent;
use crate::launcher::programs::{self, Disk, Program};

/// The lists are kept in display order as they change, rather than sorted at each draw:
/// a folder being measured sends one event per subfolder.
#[derive(Debug, Default)]
pub struct StorageScreen {
    pub disks: Vec<Disk>,
    /// By name.
    pub programs: Vec<Program>,
    /// Indices in `programs`, in display order.
    program_order: Vec<usize>,
    pub state: TableState,
    /// Programs of this drive only; `None`: every drive.
    pub disk: Option<char>,
    /// Smallest first instead of biggest. Only `toggle_storage_order` changes it, as the
    /// lists are reordered then.
    pub(super) ascending: bool,
    pub scanning: bool,
    /// `Some`: the folder browser replaces the programs.
    pub folders: Option<Folders>,
    /// Folder sizes measured so far, kept while browsing back and forth.
    pub(super) folder_sizes: HashMap<PathBuf, u64>,
}

impl StorageScreen {
    pub fn ascending(&self) -> bool {
        self.ascending
    }

    pub fn on_scanned(&mut self, disks: Vec<Disk>, programs: Vec<Program>) {
        self.scanning = false;
        self.disks = disks;
        self.programs = programs;
        self.sort_programs();
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
            self.sort_entries();
            let selected = self.selected_entry().map(|e| e.path.clone());
            self.select_entry(selected.as_deref());
        }
    }

    /// Programs of the chosen drive, by size (unknown sizes last), ties by name.
    pub fn visible_programs(&self) -> Vec<&Program> {
        self.program_order
            .iter()
            .filter_map(|&i| self.programs.get(i))
            .filter(|p| self.disk.is_none() || p.drive == self.disk)
            .collect()
    }

    fn sort_programs(&mut self) {
        let mut order: Vec<usize> = (0..self.programs.len()).collect();
        // `programs` is sorted by name and the sort is stable.
        order.sort_by(|&a, &b| {
            by_size(self.programs[a].size, self.programs[b].size, self.ascending)
        });
        self.program_order = order;
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
    /// anew. For what changed outside `CmdBoard`: an uninstaller that finished, a file
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
            self.open_folder(dir, selected.as_deref());
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
        self.storage.sort_programs();
        self.storage.sort_entries();
        self.storage.reset_selection();
        self.storage.select_entry(None);
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

/// Biggest first (or smallest, if `ascending`), unknown sizes always last.
pub(super) fn by_size(a: Option<u64>, b: Option<u64>, ascending: bool) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) if ascending => x.cmp(&y),
        (Some(x), Some(y)) => y.cmp(&x),
        (a, b) => b.is_some().cmp(&a.is_some()),
    }
}
