//! Windows notifications (plan section 11): a limit reached, and a break suggested after
//! long play. Seen while in a game, from the UI and the background tracker alike, as
//! both handle tracker events through `App::handle`, which sends them.

use std::time::Duration;

use super::App;
use crate::config::Config;
use crate::core::goals;
use crate::notify::{self, Level, Notice};
use crate::storage::models::Goal;

/// Notifications waiting to be sent, and what to send.
#[derive(Debug, Default)]
pub struct Notices {
    /// `notifications` in `config.toml`. Off until attached, so in tests too.
    pub(super) enabled: bool,
    /// `break_minutes` in `config.toml`.
    pub(super) break_after: Option<Duration>,
    /// Shows a notice. `None` in tests, which read `queue` instead.
    send: Option<fn(Notice)>,
    pub(super) queue: Vec<Notice>,
}

impl App {
    /// At startup, in the UI and in the background tracker.
    pub fn attach_notifications(&mut self, config: &Config) {
        self.notices = Notices {
            enabled: config.notifications,
            break_after: config.break_after(),
            send: Some(notify::show),
            queue: Vec::new(),
        };
    }

    /// "Limit reached" / "Games: 2h / 2h today", the first time in its period.
    pub(super) fn notify_limit(&mut self, goal: &Goal, secs: u64) {
        let body = t!(
            "notify.limit_body",
            target = self.goal_target_name(goal.target),
            used = goals::format_hm(secs),
            total = goals::format_hm(u64::from(goal.minutes) * 60),
            period = super::goals::this_period(goal.period)
        );
        self.push_notice(Level::Warning, t!("notify.limit_title"), body);
    }

    /// After each tracker report: suggests a break each time `break_after` more is
    /// played since the session started or the user was last idle, which counts as a
    /// break. A session resumed after a handover is not reminded again of a time it had
    /// already been reminded of.
    pub(super) fn check_break(&mut self, app_id: i64) {
        let Some(every) = self.notices.break_after.filter(|_| self.notices.enabled) else {
            return;
        };
        let Some(session) = self.active_sessions.get_mut(&app_id) else {
            return;
        };
        if session.idle {
            session.stretch_from = session.played;
            session.next_break = Some(session.played + every);
            return;
        }
        let stretch = session.played.saturating_sub(session.stretch_from);
        let from = session.stretch_from;
        let next = *session.next_break.get_or_insert_with(|| {
            let reminded = stretch.as_secs() / every.as_secs().max(1);
            from + Duration::from_secs(every.as_secs().saturating_mul(reminded + 1))
        });
        if session.played < next {
            return;
        }
        session.next_break = Some(next + every);
        let Some(name) = self.app_name(app_id) else {
            return;
        };
        let body = t!(
            "notify.break_body",
            name,
            time = goals::format_hm(stretch.as_secs())
        );
        self.push_notice(Level::Info, t!("notify.break_title"), body);
    }

    fn push_notice(&mut self, level: Level, title: String, body: String) {
        if self.notices.enabled {
            self.notices.queue.push(Notice { level, title, body });
        }
    }

    /// Sends the queued notifications, once the event that queued them is handled.
    pub(super) fn send_notices(&mut self) {
        if let Some(send) = self.notices.send {
            self.notices.queue.drain(..).for_each(send);
        }
    }
}
