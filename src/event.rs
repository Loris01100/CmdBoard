//! Event thread: keyboard input plus a regular `Tick`, sent to the UI thread.

use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEvent, KeyEventKind};

use crate::launcher::scan::Shortcut;
use crate::update;

/// Drives animations and the live session timer.
pub const TICK: Duration = Duration::from_millis(250);

/// Everything the UI thread reacts to, whichever thread it comes from.
#[derive(Debug)]
pub enum AppEvent {
    Key(KeyEvent),
    Tick,
    /// A watched process appeared (sent by the tracker).
    SessionStarted {
        app_id: i64,
    },
    /// The watched process is gone after running `secs` seconds (sent by the tracker).
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
}

/// Reads the keyboard and sends a `Tick` every `TICK`. Stops once the UI thread is gone.
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
