//! Tracking while the UI is closed (plan section 12). On quit, the UI starts a
//! `cmdboard --background` process and hands its running sessions over; that process
//! tracks, checkpoints and awards sessions without a terminal, and hands them back when
//! the UI starts again. A session saved moments ago whose app still runs is resumed
//! rather than closed, so a handover never splits it.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use super::App;
use super::sessions::ActiveSession;
use crate::event::AppEvent;
use crate::instance;
use crate::storage::unix_now;
use crate::tracker;

/// An open session saved longer ago was left by a crash or a shutdown, not handed over:
/// it is closed at its last checkpoint instead of resumed.
const HANDOVER_WINDOW_SECS: i64 = 5 * 60;

/// How often the background tracker looks for a stop request.
const STOP_POLL: Duration = Duration::from_secs(1);

impl App {
    /// At startup: resumes the sessions handed over by the other process whose app still
    /// runs, and closes the others like after a crash. Returns the time the resumed ones
    /// were played, by app, for the tracker.
    pub fn recover_sessions(&mut self) -> anyhow::Result<HashMap<i64, Duration>> {
        let running = tracker::running_now(&self.watch_list());
        self.resume_sessions(&running, unix_now())
    }

    /// `recover_sessions` with the apps `running` at `now`.
    pub(super) fn resume_sessions(
        &mut self,
        running: &HashSet<i64>,
        now: i64,
    ) -> anyhow::Result<HashMap<i64, Duration>> {
        let mut resumed = HashMap::new();
        let mut ids = Vec::new();
        for open in self.db.open_sessions()? {
            let recent = open
                .checkpoint_at
                .is_some_and(|at| now - at <= HANDOVER_WINDOW_SECS);
            if !recent || !running.contains(&open.app_id) || resumed.contains_key(&open.app_id) {
                continue;
            }
            let played = Duration::from_secs(open.secs);
            self.active_sessions
                .insert(open.app_id, ActiveSession::new(open.session_id, played));
            resumed.insert(open.app_id, played);
            ids.push(open.session_id);
        }
        self.close_orphan_sessions(&ids)?;
        // Reached before the handover: not announced again.
        self.check_goals(false);
        Ok(resumed)
    }

    /// On quit: starts the background tracker (`exe`, the one `CmdBoard` was started from,
    /// so an update installed meanwhile runs) and hands it the running sessions. Without
    /// `background`, apps to watch, or if it cannot start, closes them instead.
    /// Returns whether the background tracker was started.
    pub fn quit_sessions(&mut self, background: bool, exe: &Path) -> anyhow::Result<bool> {
        let started =
            background && !self.watch_list().is_empty() && instance::spawn_background(exe).is_ok();
        if started {
            // It waits for this process to quit before reading the sessions.
            self.hand_over_sessions()?;
        } else {
            self.end_all_sessions()?;
        }
        Ok(started)
    }

    /// Saves the running sessions and leaves them open for the next process.
    pub(super) fn hand_over_sessions(&mut self) -> anyhow::Result<()> {
        for (_, session) in self.active_sessions.drain() {
            self.db
                .checkpoint_session(session.session_id, session.played.as_secs())?;
        }
        Ok(())
    }

    /// Background tracker loop: handles tracker events without drawing, until `stop`
    /// says so (the UI starting) or the tracker dies, then hands the sessions over.
    pub fn run_background(
        &mut self,
        events: &Receiver<AppEvent>,
        stop: impl Fn() -> bool,
    ) -> anyhow::Result<()> {
        while !stop() {
            match events.recv_timeout(STOP_POLL) {
                Ok(event) => self.handle(event),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.checkpoint_sessions();
        }
        self.hand_over_sessions()
    }
}
