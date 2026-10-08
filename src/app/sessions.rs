//! Sessions reported by the tracker: checkpoints while they run, then what closing one
//! earned, shown as XP bars filling up and level-up or reward popups. The awarding itself
//! is one storage transaction (`Database::close_session`).

use std::time::{Duration, Instant};

use super::{App, Message, Mode, MsgKind};
use crate::core::xp;
use crate::popup::{LevelUp, Popup, RewardUnlocked};
use crate::storage::{
    models::{AppEntry, RuleError},
    unix_now,
};
use crate::tracker;

/// How often running sessions save their time played, in case of a crash.
const CHECKPOINT_EVERY: Duration = Duration::from_secs(60);

/// How many ticks an XP bar takes to fill up to its new value (2 s at 250 ms).
pub const XP_ANIM_FRAMES: u64 = 8;

/// A session in progress, keyed by app id in `App::active_sessions`.
#[derive(Debug, Clone, Copy)]
pub struct ActiveSession {
    /// Row in `sessions`, open until the session ends.
    pub session_id: i64,
    pub started: Instant,
    /// Time really played, as last reported by the tracker (idle time left out).
    pub played: Duration,
    /// The user has been idle long enough that time stopped counting.
    pub idle: bool,
    /// When `played` was reported.
    reported: Instant,
    pub(super) last_checkpoint: Instant,
}

impl ActiveSession {
    /// Play time to display: the last report, plus the time since while it counts.
    /// Capped at one poll, so the timer never runs ahead of the next report.
    pub fn shown_secs(&self) -> u64 {
        let since = if self.idle {
            Duration::ZERO
        } else {
            self.reported.elapsed().min(tracker::POLL)
        };
        (self.played + since).as_secs()
    }
}

/// An XP total moving from `from` to `to`, started at tick `start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XpAnim {
    pub from: u32,
    pub to: u32,
    pub start: u64,
}

impl XpAnim {
    /// The total to show at tick `frame`.
    pub fn value(&self, frame: u64) -> u32 {
        xp::animate(
            self.from,
            self.to,
            frame.saturating_sub(self.start),
            XP_ANIM_FRAMES,
        )
    }

    fn is_done(&self, frame: u64) -> bool {
        frame.saturating_sub(self.start) >= XP_ANIM_FRAMES
    }
}

impl App {
    /// Each tick redraws (live timer, animations); running sessions also checkpoint here.
    pub fn on_tick(&mut self) {
        self.frame_count += 1;
        let frame = self.frame_count;
        self.xp_anims.retain(|_, anim| !anim.is_done(frame));
        if self.profile_anim.is_some_and(|anim| anim.is_done(frame)) {
            self.profile_anim = None;
        }
        self.show_pending_popup();
        self.checkpoint_sessions();
    }

    fn checkpoint_sessions(&mut self) {
        for session in self.active_sessions.values_mut() {
            if session.last_checkpoint.elapsed() < CHECKPOINT_EVERY {
                continue;
            }
            session.last_checkpoint = Instant::now();
            let secs = session.played.as_secs();
            if let Err(e) = self.db.checkpoint_session(session.session_id, secs) {
                self.message = Some((format!("{e:#}"), MsgKind::Error));
            }
        }
    }

    pub fn on_session_start(&mut self, app_id: i64) {
        if self.active_sessions.contains_key(&app_id) {
            return;
        }
        // The app may have been removed since the tracker's last poll.
        let Some(name) = self.app_name(app_id) else {
            return;
        };
        match self.db.start_session(app_id, unix_now()) {
            Ok(session_id) => {
                let now = Instant::now();
                self.active_sessions.insert(
                    app_id,
                    ActiveSession {
                        session_id,
                        started: now,
                        played: Duration::ZERO,
                        idle: false,
                        reported: now,
                        last_checkpoint: now,
                    },
                );
                self.message = Some((t!("session.started", name), MsgKind::Info));
            }
            Err(e) => self.message = Some((format!("{e:#}"), MsgKind::Error)),
        }
    }

    pub fn on_session_progress(&mut self, app_id: i64, played: Duration, idle: bool) {
        if let Some(session) = self.active_sessions.get_mut(&app_id) {
            session.played = played;
            session.idle = idle;
            session.reported = Instant::now();
        }
    }

    /// `secs` is measured by the tracker, from detection to disappearance, idle time
    /// left out.
    pub fn on_session_end(&mut self, app_id: i64, secs: u64) {
        let Some(session) = self.active_sessions.remove(&app_id) else {
            return;
        };
        let Some(before) = self.find_app_by_id(app_id).map(|a| a.total_xp) else {
            return; // removed meanwhile
        };
        let profile_before = self.profile.total_xp;
        let result = self
            .db
            .close_session(session.session_id, unix_now(), secs)
            .and_then(|outcome| self.reload().map(|()| outcome));
        let Some(name) = self.app_name(app_id) else {
            return;
        };
        let message = match result {
            Ok(Some(outcome)) => {
                let text = t!("session.ended", name, minutes = secs / 60, xp = outcome.xp);
                self.on_xp_changed(app_id, before, profile_before, outcome.xp);
                self.queue_rewards(outcome.rewards);
                with_rule_errors(text, MsgKind::Success, &outcome.rule_errors)
            }
            Ok(None) => (t!("session.too_short", name), MsgKind::Info),
            Err(e) => (format!("{e:#}"), MsgKind::Error),
        };
        self.message = Some(message);
    }

    /// At startup, closes the sessions a crash left open and rewards them.
    pub fn close_orphan_sessions(&mut self) -> anyhow::Result<()> {
        let outcomes = self.db.recover_orphan_sessions()?;
        if outcomes.is_empty() {
            return Ok(());
        }
        self.reload()?;
        let text = t!("session.recovered", count = outcomes.len());
        let (mut unlocked, mut errors) = (Vec::new(), Vec::new());
        for outcome in outcomes {
            unlocked.extend(outcome.rewards);
            errors.extend(outcome.rule_errors);
        }
        self.queue_rewards(unlocked);
        self.message = Some(with_rule_errors(text, MsgKind::Info, &errors));
        Ok(())
    }

    /// On quit, closes running sessions as if their apps had stopped.
    pub(super) fn end_all_sessions(&mut self) -> anyhow::Result<()> {
        let sessions: Vec<_> = self.active_sessions.drain().collect();
        for (_, session) in sessions {
            let secs = session.played.as_secs();
            self.db
                .close_session(session.session_id, unix_now(), secs)?;
        }
        Ok(())
    }

    fn queue_rewards(&mut self, unlocked: Vec<RewardUnlocked>) {
        self.pending_popups
            .extend(unlocked.into_iter().map(Popup::RewardUnlocked));
        self.show_pending_popup();
    }

    /// After an app's XP changed (data already reloaded): animates its bar and the
    /// profile's, and queues a level-up popup if a level went up.
    pub(super) fn on_xp_changed(
        &mut self,
        app_id: i64,
        app_before: u32,
        profile_before: u32,
        gained: u32,
    ) {
        let Some(entry) = self.find_app_by_id(app_id) else {
            return;
        };
        let (name, app_after, app_level) = (entry.name.clone(), entry.total_xp, entry.level);
        let frame = self.frame_count;

        // Start from what is on screen, in case a previous animation is still running.
        let from = self
            .xp_anims
            .get(&app_id)
            .map_or(app_before, |a| a.value(frame));
        self.xp_anims.insert(
            app_id,
            XpAnim {
                from,
                to: app_after,
                start: frame,
            },
        );
        let from = self.profile_anim.map_or(profile_before, |a| a.value(frame));
        self.profile_anim = Some(XpAnim {
            from,
            to: self.profile.total_xp,
            start: frame,
        });

        let went_up =
            |before: u32, level: u32| (level > xp::level_from_total(before).0).then_some(level);
        let level_up = LevelUp {
            app: name,
            app_level: went_up(app_before, app_level),
            global_level: went_up(profile_before, self.profile.level),
            gained,
        };
        if level_up.app_level.is_some() || level_up.global_level.is_some() {
            self.pending_popups.push_back(Popup::LevelUp(level_up));
            self.show_pending_popup();
        }
    }

    /// Opens the next queued popup, unless the user is busy typing or answering another.
    pub(super) fn show_pending_popup(&mut self) {
        if self.mode == Mode::Normal
            && let Some(popup) = self.pending_popups.pop_front()
        {
            self.mode = Mode::Popup(popup);
        }
    }

    /// `(level, xp within that level)` to display for an app, following its animation.
    pub fn shown_app_xp(&self, entry: &AppEntry) -> (u32, u32) {
        match self.xp_anims.get(&entry.id) {
            Some(anim) => xp::level_from_total(anim.value(self.frame_count)),
            None => (entry.level, entry.xp),
        }
    }

    /// `(level, xp within that level)` to display for the profile, following its animation.
    pub fn shown_profile_xp(&self) -> (u32, u32) {
        match self.profile_anim {
            Some(anim) => xp::level_from_total(anim.value(self.frame_count)),
            None => (self.profile.level, self.profile.xp),
        }
    }
}

/// `text`, or, if a reward rule broke, `text` and the first error, as an error.
fn with_rule_errors(text: String, kind: MsgKind, errors: &[RuleError]) -> Message {
    match errors.first() {
        Some(e) => {
            let error = t!("session.rule_error", code = e.code, error = e.error);
            (format!("{text} · {error}"), MsgKind::Error)
        }
        None => (text, kind),
    }
}
