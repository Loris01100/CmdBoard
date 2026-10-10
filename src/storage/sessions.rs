//! Sessions: start, checkpoints, and closing one together with what it earned.
//!
//! Closing is a single transaction: an error or a crash leaves the session open, so the
//! orphan recovery at the next start awards it. A session is never closed without its XP
//! and rewards.

use rusqlite::params;

use super::db::Database;
use super::models::{
    ClosedSession, OpenSession, Reward, RewardUnlocked, RuleError, SessionOutcome,
};
use super::queries::unix_now;
use super::{to_i64, to_u64};
use crate::core::{
    rewards::{self, Facts},
    xp,
};
use crate::i18n;

/// Shorter sessions (a quick launch and close) are not recorded.
pub const MIN_SESSION_SECS: u64 = 60;

impl Database {
    /// Opens a session (`ended_at` stays NULL until it ends). Returns its id.
    pub fn start_session(&self, app_id: i64, started_at: i64) -> anyhow::Result<i64> {
        self.conn.execute(
            "INSERT INTO sessions (app_id, started_at, checkpoint_at) VALUES (?1, ?2, ?2)",
            [app_id, started_at],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Saves the time played so far, so a crash loses at most one checkpoint interval.
    pub fn checkpoint_session(&self, id: i64, secs: u64) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET duration_s = ?2, checkpoint_at = ?3
             WHERE id = ?1 AND ended_at IS NULL",
            params![id, to_i64(secs), unix_now()],
        )?;
        Ok(())
    }

    /// Sessions not closed yet, oldest first.
    pub fn open_sessions(&self) -> anyhow::Result<Vec<OpenSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, app_id, duration_s, checkpoint_at FROM sessions
             WHERE ended_at IS NULL ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(OpenSession {
                session_id: r.get(0)?,
                app_id: r.get(1)?,
                secs: to_u64(r.get(2)?),
                checkpoint_at: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Closes a session and awards its XP and rewards, all or nothing. Returns `None` if
    /// it was too short to be recorded.
    pub fn close_session(
        &self,
        id: i64,
        ended_at: i64,
        secs: u64,
    ) -> anyhow::Result<Option<SessionOutcome>> {
        let tx = self.conn.unchecked_transaction()?;
        let outcome = if self.end_session(id, ended_at, secs)? {
            Some(self.reward_session(id, secs, ended_at)?)
        } else {
            None
        };
        tx.commit()?;
        Ok(outcome)
    }

    /// Closes the open sessions, except the `resumed` ones, at their last checkpoint and
    /// awards them, all or nothing.
    pub fn recover_orphan_sessions(&self, resumed: &[i64]) -> anyhow::Result<Vec<SessionOutcome>> {
        let now = unix_now();
        let tx = self.conn.unchecked_transaction()?;
        let outcomes = self
            .close_orphan_sessions(resumed)?
            .into_iter()
            .map(|s| self.reward_session(s.session_id, s.secs, now))
            .collect::<anyhow::Result<_>>()?;
        tx.commit()?;
        Ok(outcomes)
    }

    /// Hides every finished session from the history; stats still count them.
    /// Returns how many were hidden.
    pub fn hide_sessions(&self) -> anyhow::Result<usize> {
        Ok(self.conn.execute(
            "UPDATE sessions SET hidden = 1 WHERE ended_at IS NOT NULL AND NOT hidden",
            [],
        )?)
    }

    /// Deletes every finished session, so stats start over; running ones stay.
    /// App XP and rewards are kept. Returns how many were deleted.
    pub fn clear_sessions(&self) -> anyhow::Result<usize> {
        Ok(self
            .conn
            .execute("DELETE FROM sessions WHERE ended_at IS NOT NULL", [])?)
    }

    /// Sets `ended_at`, without awarding anything: only inside `close_session`'s
    /// transaction, or in tests. Sessions shorter than `MIN_SESSION_SECS` are dropped
    /// instead. Returns whether the session was kept.
    pub(super) fn end_session(&self, id: i64, ended_at: i64, secs: u64) -> anyhow::Result<bool> {
        if secs < MIN_SESSION_SECS {
            self.conn
                .execute("DELETE FROM sessions WHERE id = ?1", [id])?;
            return Ok(false);
        }
        self.conn.execute(
            "UPDATE sessions SET ended_at = ?2, duration_s = ?3 WHERE id = ?1",
            params![id, ended_at, to_i64(secs)],
        )?;
        Ok(true)
    }

    /// Closes the open sessions, except the `resumed` ones, at their last checkpoint,
    /// without awarding them.
    pub(super) fn close_orphan_sessions(
        &self,
        resumed: &[i64],
    ) -> anyhow::Result<Vec<ClosedSession>> {
        let resumed = serde_json::to_string(resumed)?;
        self.conn.execute(
            "DELETE FROM sessions WHERE ended_at IS NULL AND duration_s < ?1
               AND id NOT IN (SELECT value FROM json_each(?2))",
            params![to_i64(MIN_SESSION_SECS), resumed],
        )?;
        let kept = self
            .conn
            .prepare(
                "UPDATE sessions SET ended_at = started_at + duration_s
                 WHERE ended_at IS NULL AND id NOT IN (SELECT value FROM json_each(?1))
                 RETURNING id, duration_s",
            )?
            .query_map([resumed], |r| {
                Ok(ClosedSession {
                    session_id: r.get(0)?,
                    secs: to_u64(r.get(1)?),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(kept)
    }

    /// XP first, so rules see the new levels, then rewards. The streak includes today,
    /// now that the session is closed.
    fn reward_session(
        &self,
        session_id: i64,
        secs: u64,
        now: i64,
    ) -> anyhow::Result<SessionOutcome> {
        let streak = self.profile_at(now)?.streak_days;
        let xp = xp::xp_for_session(u32::try_from(secs / 60).unwrap_or(u32::MAX), streak);
        if xp > 0 {
            self.add_session_xp(session_id, xp)?;
        }
        let (rewards, rule_errors) = self.unlock_rewards(session_id, now)?;
        Ok(SessionOutcome {
            xp,
            rewards,
            rule_errors,
        })
    }

    /// Unlocks the rewards whose rule the session passes. A broken rule is reported, not
    /// fatal: the other rewards still count.
    fn unlock_rewards(
        &self,
        session_id: i64,
        now: i64,
    ) -> anyhow::Result<(Vec<RewardUnlocked>, Vec<RuleError>)> {
        let (app_id, facts) = self.session_facts_at(session_id, now)?;
        let app_name: String =
            self.conn
                .query_row("SELECT name FROM apps WHERE id = ?1", [app_id], |r| {
                    r.get(0)
                })?;
        let (mut unlocked, mut errors) = (Vec::new(), Vec::new());
        for reward in self.pending_rewards(app_id)? {
            match rewards::evaluate(&reward.rule, &facts) {
                Ok(true) => {
                    let for_app = reward.per_app.then_some(app_id);
                    self.unlock_reward(reward.id, for_app, session_id, now)?;
                    unlocked.push(RewardUnlocked {
                        name: reward.name,
                        description: reward.description,
                        app: reward.per_app.then(|| app_name.clone()),
                    });
                }
                Ok(false) => {}
                Err(error) => errors.push(RuleError {
                    code: reward.code,
                    error,
                }),
            }
        }
        Ok((unlocked, errors))
    }

    /// Records the XP a closed session earned and adds it to its app.
    pub(super) fn add_session_xp(&self, session_id: i64, xp: u32) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET xp_gained = ?2 WHERE id = ?1",
            [session_id, i64::from(xp)],
        )?;
        self.conn.execute(
            "UPDATE apps SET total_xp = total_xp + ?2
             WHERE id = (SELECT app_id FROM sessions WHERE id = ?1)",
            [session_id, i64::from(xp)],
        )?;
        Ok(())
    }

    /// What reward rules can test about a closed session. Returns the session's app too.
    pub(super) fn session_facts_at(
        &self,
        session_id: i64,
        now: i64,
    ) -> anyhow::Result<(i64, Facts)> {
        let (app_id, secs, hour): (i64, i64, i64) = self.conn.query_row(
            "SELECT app_id, duration_s,
                CAST(strftime('%H', started_at, 'unixepoch', 'localtime') AS INTEGER)
             FROM sessions WHERE id = ?1",
            [session_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let (app_secs, app_sessions, app_xp): (i64, i64, u32) = self.conn.query_row(
            "SELECT
                (SELECT COALESCE(SUM(duration_s), 0) FROM sessions
                    WHERE app_id = ?1 AND ended_at IS NOT NULL),
                (SELECT COUNT(*) FROM sessions WHERE app_id = ?1 AND ended_at IS NOT NULL),
                total_xp
             FROM apps WHERE id = ?1",
            [app_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let (total_secs, total_sessions, apps_this_week): (i64, i64, i64) = self.conn.query_row(
            "SELECT COALESCE(SUM(duration_s), 0), COUNT(*),
                COUNT(DISTINCT CASE WHEN ended_at > ?1 - 7 * 86400 THEN app_id END)
             FROM sessions WHERE ended_at IS NOT NULL",
            [now],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let profile = self.profile_at(now)?;
        let facts = Facts {
            session_minutes: (secs / 60) as f64,
            session_hour: hour as f64,
            app_hours: app_secs as f64 / 3600.0,
            app_sessions: app_sessions as f64,
            app_level: f64::from(xp::level_from_total(app_xp).0),
            level: f64::from(profile.level),
            streak_days: f64::from(profile.streak_days),
            total_hours: total_secs as f64 / 3600.0,
            total_sessions: total_sessions as f64,
            apps_this_week: apps_this_week as f64,
        };
        Ok((app_id, facts))
    }

    /// Rewards `app_id` can still unlock: global ones nobody unlocked yet, and per-app
    /// ones not unlocked for this app. Rewards tied to another app are skipped.
    pub(super) fn pending_rewards(&self, app_id: i64) -> anyhow::Result<Vec<Reward>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.code, r.name, r.description, r.rule,
                r.scope = 'app' OR r.app_id IS NOT NULL AS per_app
             FROM rewards r
             WHERE (r.app_id IS NULL OR r.app_id = ?1)
               AND NOT EXISTS (
                   SELECT 1 FROM unlocked_rewards u WHERE u.reward_id = r.id
                     AND (u.app_id = ?1 OR NOT (r.scope = 'app' OR r.app_id IS NOT NULL)))
             ORDER BY r.id",
        )?;
        let rows = stmt.query_map([app_id], |r| {
            let code: String = r.get(1)?;
            Ok(Reward {
                id: r.get(0)?,
                name: i18n::reward_text(&code, "name", r.get(2)?),
                description: i18n::reward_text(&code, "description", r.get(3)?),
                rule: r.get(4)?,
                per_app: r.get(5)?,
                code,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// `app_id` is the app a per-app reward is unlocked for, `None` for a global one.
    pub(super) fn unlock_reward(
        &self,
        reward_id: i64,
        app_id: Option<i64>,
        session_id: i64,
        unlocked_at: i64,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO unlocked_rewards (reward_id, app_id, unlocked_at, session_id)
             VALUES (?1, ?2, ?3, ?4)",
            params![reward_id, app_id, unlocked_at, session_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::models::NewApp;

    fn db_with_app() -> (Database, i64) {
        let db = Database::open_in_memory().unwrap();
        let category_id = db.add_category("Jeux").unwrap();
        let app = db
            .add_app(&NewApp {
                name: "Hades".into(),
                launch_target: "hades.exe".into(),
                watch_exe: Some("hades.exe".into()),
                category_id,
            })
            .unwrap();
        (db, app)
    }

    /// `(ended_at, xp_gained)` of every session.
    fn rows(db: &Database) -> Vec<(Option<i64>, u32)> {
        let mut stmt = db
            .conn
            .prepare("SELECT ended_at, xp_gained FROM sessions ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn closing_awards_xp_and_rewards() {
        let (db, app) = db_with_app();
        let id = db.start_session(app, 1_000).unwrap();
        let outcome = db.close_session(id, 1_000 + 600, 600).unwrap().unwrap();
        assert_eq!(outcome.xp, 15); // 10 min + 5 for the streak
        assert_eq!(outcome.rewards[0].name, "Premiers pas");
        assert!(outcome.rule_errors.is_empty());
        assert_eq!(rows(&db), [(Some(1_600), 15)]);

        let short = db.start_session(app, 5_000).unwrap();
        assert_eq!(db.close_session(short, 5_010, 10).unwrap(), None);
        assert_eq!(rows(&db).len(), 1); // dropped
    }

    #[test]
    fn a_failure_while_awarding_leaves_the_session_open() {
        let (db, app) = db_with_app();
        db.execute_for_tests(
            "CREATE TRIGGER fail BEFORE INSERT ON unlocked_rewards
             BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
        );
        let id = db.start_session(app, 1_000).unwrap();
        db.checkpoint_session(id, 600).unwrap();
        assert!(db.close_session(id, 1_600, 600).is_err());
        assert_eq!(rows(&db), [(None, 0)]); // neither closed nor credited
        assert_eq!(db.apps().unwrap()[0].total_xp, 0);
        assert!(db.recover_orphan_sessions(&[]).is_err()); // the same failure: still open
        assert_eq!(rows(&db), [(None, 0)]);

        // Once the cause is gone, the next start recovers it.
        db.execute_for_tests("DROP TRIGGER fail");
        let outcomes = db.recover_orphan_sessions(&[]).unwrap();
        assert_eq!(outcomes.len(), 1);
        // 10 min; no streak bonus: the session ended in 1970, not today.
        assert_eq!(rows(&db), [(Some(1_600), 10)]);
        assert_eq!(db.apps().unwrap()[0].total_xp, 10);
    }

    #[test]
    fn resumed_sessions_stay_open_through_recovery() {
        let (db, app) = db_with_app();
        let resumed = db.start_session(app, 1_000).unwrap();
        let crashed = db.start_session(app, 2_000).unwrap();
        db.checkpoint_session(resumed, 600).unwrap();
        db.checkpoint_session(crashed, 600).unwrap();
        let open = db.open_sessions().unwrap();
        assert_eq!(open.len(), 2);
        assert!(
            open[0]
                .checkpoint_at
                .is_some_and(|at| at >= unix_now() - 60)
        );

        assert_eq!(db.recover_orphan_sessions(&[resumed]).unwrap().len(), 1);
        let open = db.open_sessions().unwrap();
        assert_eq!(
            (open.len(), open[0].session_id, open[0].secs),
            (1, resumed, 600)
        );
        assert_eq!(rows(&db), [(None, 0), (Some(2_600), 10)]);
    }

    #[test]
    fn a_broken_rule_is_reported_with_its_code() {
        let (db, app) = db_with_app();
        db.execute_for_tests("UPDATE rewards SET rule = 'hours >= 1' WHERE code = 'marathon'");
        let id = db.start_session(app, 1_000).unwrap();
        let outcome = db.close_session(id, 1_600, 600).unwrap().unwrap();
        assert_eq!(outcome.rule_errors[0].code, "marathon");
        assert!(!outcome.rewards.is_empty()); // the others still unlock
    }
}
