//! Event thread: keyboard input plus a regular `Tick`, sent to the UI thread.

use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEvent, KeyEventKind};

use crate::launcher::{
    folders::Entry,
    programs::{Disk, Program},
    scan::Shortcut,
};
use crate::update;

/// Drives animations and the live session timer. A tick only redraws when something on
/// screen moves (`App::on_tick`).
pub const TICK: Duration = Duration::from_millis(250);

/// Everything the UI thread reacts to, whichever thread it comes from.
#[derive(Debug)]
pub enum AppEvent {
    Key(KeyEvent),
    /// The terminal was resized: the screen is drawn again.
    Resize,
    Tick,
    /// A watched process appeared (sent by the tracker).
    SessionStarted {
        app_id: i64,
    },
    /// Time really played so far in a running session, and whether the user is idle
    /// (sent by the tracker at every poll).
    SessionProgress {
        app_id: i64,
        played: Duration,
        idle: bool,
    },
    /// The watched process is gone after `secs` seconds of play (sent by the tracker).
    SessionEnded {
        app_id: i64,
        secs: u64,
    },
    /// An update check or install finished (sent by a short-lived thread).
    UpdateFinished {
        action: update::Action,
        result: Result<update::Outcome, String>,
    },
    /// Installed apps found by a short-lived scan thread, for the "add app" picker.
    ShortcutsScanned(Vec<Shortcut>),
    /// Disks and installed programs read by a short-lived thread, for the Storage screen.
    StorageScanned {
        disks: Vec<Disk>,
        programs: Vec<Program>,
    },
    /// What a folder holds (folder browser), its subfolders not measured yet.
    FolderListed {
        dir: std::path::PathBuf,
        entries: Vec<Entry>,
    },
    /// Percentage of a subfolder's direct children measured so far.
    FolderProgress {
        path: std::path::PathBuf,
        percent: u8,
    },
    /// One subfolder measured.
    FolderSized {
        path: std::path::PathBuf,
        size: u64,
    },
    /// A file or folder sent to the Recycle Bin, or the error.
    Trashed {
        path: std::path::PathBuf,
        result: Result<(), String>,
    },
    /// A benchmark of the Optimization screen finished (sent by a short-lived thread).
    BenchFinished {
        bench: crate::optimize::Bench,
        heavy: bool,
        result: Result<crate::optimize::Score, String>,
    },
}

/// Reads the keyboard and resizes, and sends a `Tick` every `TICK`. Stops once the UI thread is gone.
pub fn spawn(tx: Sender<AppEvent>) {
    thread::spawn(move || {
        let mut next_tick = Instant::now() + TICK;
        loop {
            let timeout = next_tick.saturating_duration_since(Instant::now());
            let event = match event::poll(timeout) {
                // Windows sends both Press and Release: only forward Press.
                Ok(true) => match event::read() {
                    Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                        Some(AppEvent::Key(key))
                    }
                    Ok(Event::Resize(..)) => Some(AppEvent::Resize),
                    Ok(_) => None,
                    Err(_) => return,
                },
                Ok(false) => {
                    next_tick = Instant::now() + TICK;
                    Some(AppEvent::Tick)
                }
                Err(_) => return,
            };
            if let Some(event) = event
                && tx.send(event).is_err()
            {
                return;
            }
        }
    });
}
