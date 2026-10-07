//! CRUD on top of `Database`. Every read returns owned models that `App` caches.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use rusqlite::params;

use super::db::Database;
use super::models::{
    ACTIVITY_DAYS, Activity, AppEntry, Category, ClosedSession, NewApp, Profile, Reward,
    RewardView, SessionRow, Stats, Unlock,
};
use crate::core::{rewards::Facts, xp};
use crate::i18n;

/// Shorter sessions (a quick launch and close) are not recorded.
pub const MIN_SESSION_SECS: u64 = 60;

/// Sessions listed on the Stats screen.
const STATS_SESSIONS: u32 = 200;

impl Database {
    pub fn categories(&self) -> anyhow::Result<Vec<Category>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name FROM categories ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok(Category {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn add_category(&self, name: &str) -> anyhow::Result<i64> {
        self.conn
            .execute("INSERT INTO categories (name) VALUES (?1)", [name])
            .with_context(|| t!("error.cannot_add_category", name))?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Fails while the category still holds apps.
    pub fn delete_category(&self, id: i64) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM categories WHERE id = ?1", [id])
            .with_context(|| t!("error.category_has_apps"))?;
        Ok(())
    }

    /// All apps sorted by name, with time played, last session and rewards aggregated.
    pub fn apps(&self) -> anyhow::Result<Vec<AppEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.id, a.name, a.launch_target, a.watch_exe, a.category_id, a.total_xp,
                (SELECT COALESCE(SUM(duration_s), 0) FROM sessions s
                    WHERE s.app_id = a.id AND s.ended_at IS NOT NULL),
                (SELECT MAX(ended_at) FROM sessions s WHERE s.app_id = a.id),
                (SELECT COUNT(*) FROM unlocked_rewards u WHERE u.app_id = a.id)
             FROM apps a
             ORDER BY a.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| {
            let total_xp = r.get(5)?;
            let (level, xp) = xp::level_from_total(total_xp);
            Ok(AppEntry {
                id: r.get(0)?,
                name: r.get(1)?,
                launch_target: r.get(2)?,
                watch_exe: r.get(3)?,
                category_id: r.get(4)?,
                total_xp,
                level,
                xp,
                total_secs: r.get::<_, i64>(6)?.max(0) as u64,
                last_played: r.get(7)?,
                rewards: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn add_app(&self, app: &NewApp) -> anyhow::Result<i64> {
        self.conn
            .execute(
                "INSERT INTO apps (name, launch_target, watch_exe, category_id)
                 VALUES (?1, ?2, ?3, ?4)",
                params![app.name, app.launch_target, app.watch_exe, app.category_id],
            )
            .with_context(|| t!("error.cannot_add_app", name = app.name))?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn move_app(&self, app_id: i64, category_id: i64) -> anyhow::Result<()> {
        self.conn
            .execute(
                "UPDATE apps SET category_id = ?2 WHERE id = ?1",
                [app_id, category_id],
            )
            .with_context(|| t!("error.cannot_move_app"))?;
        Ok(())
    }

    /// Also deletes the app's sessions and rewards (`ON DELETE CASCADE`).
    pub fn delete_app(&self, app_id: i64) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM apps WHERE id = ?1", [app_id])?;
        Ok(())
    }

    /// Opens a session (`ended_at` stays NULL until it ends). Returns its id.
    pub fn start_session(&self, app_id: i64, started_at: i64) -> anyhow::Result<i64> {
        self.conn.execute(
            "INSERT INTO sessions (app_id, started_at) VALUES (?1, ?2)",
            [app_id, started_at],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Saves the time played so far, so a crash loses at most one checkpoint interval.
    pub fn checkpoint_session(&self, id: i64, secs: u64) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET duration_s = ?2 WHERE id = ?1 AND ended_at IS NULL",
            params![id, secs as i64],
        )?;
        Ok(())
    }

    /// Closes a session. Sessions shorter than `MIN_SESSION_SECS` are dropped instead.
    /// Returns whether the session was kept.
    pub fn end_session(&self, id: i64, ended_at: i64, secs: u64) -> anyhow::Result<bool> {
        if secs < MIN_SESSION_SECS {
            self.conn
                .execute("DELETE FROM sessions WHERE id = ?1", [id])?;
            return Ok(false);
        }
        self.conn.execute(
            "UPDATE sessions SET ended_at = ?2, duration_s = ?3 WHERE id = ?1",
            params![id, ended_at, secs as i64],
        )?;
        Ok(true)
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

    /// Closes the sessions a crash left open, at their last checkpoint.
    /// Returns the kept ones, so they can still earn their XP.
    pub fn close_orphan_sessions(&self) -> anyhow::Result<Vec<ClosedSession>> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM sessions WHERE ended_at IS NULL AND duration_s < ?1",
            [MIN_SESSION_SECS as i64],
        )?;
        let kept = tx
            .prepare(
                "UPDATE sessions SET ended_at = started_at + duration_s WHERE ended_at IS NULL
                 RETURNING id, app_id, duration_s",
            )?
            .query_map([], |r| {
                Ok(ClosedSession {
                    session_id: r.get(0)?,
                    app_id: r.get(1)?,
                    secs: r.get::<_, i64>(2)?.max(0) as u64,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        tx.commit()?;
        Ok(kept)
    }

    /// Records the XP a closed session earned and adds it to its app.
    pub fn add_session_xp(&self, session_id: i64, xp: u32) -> anyhow::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE sessions SET xp_gained = ?2 WHERE id = ?1",
            [session_id, xp as i64],
        )?;
        tx.execute(
            "UPDATE apps SET total_xp = total_xp + ?2
             WHERE id = (SELECT app_id FROM sessions WHERE id = ?1)",
            [session_id, xp as i64],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Overwrites an app's XP, outside of any session.
    #[cfg(test)]
    pub fn set_app_xp(&self, app_id: i64, total_xp: u32) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE apps SET total_xp = ?2 WHERE id = ?1",
            [app_id, total_xp as i64],
        )?;
        Ok(())
    }

    pub fn profile(&self) -> anyhow::Result<Profile> {
        self.profile_at(unix_now())
    }

    /// Global profile as of `now` (Unix seconds). Days follow the local time zone.
    fn profile_at(&self, now: i64) -> anyhow::Result<Profile> {
        let total_xp: u32 =
            self.conn
                .query_row("SELECT COALESCE(SUM(total_xp), 0) FROM apps", [], |r| {
                    r.get(0)
                })?;
        let (level, xp) = xp::level_from_total(total_xp);

        let xp_today: u32 = self.conn.query_row(
            "SELECT COALESCE(SUM(xp_gained), 0) FROM sessions
             WHERE ended_at IS NOT NULL
               AND date(ended_at, 'unixepoch', 'localtime') = date(?1, 'unixepoch', 'localtime')",
            [now],
            |r| r.get(0),
        )?;

        let day = "CAST(julianday(date(?1, 'unixepoch', 'localtime')) AS INTEGER)";
        let today: i64 = self
            .conn
            .query_row(&format!("SELECT {day}"), [now], |r| r.get(0))?;
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT CAST(julianday(date(ended_at, 'unixepoch', 'localtime')) AS INTEGER) AS d
             FROM sessions WHERE ended_at IS NOT NULL ORDER BY d DESC",
        )?;
        let days = stmt
            .query_map([], |r| r.get(0))?
            .collect::<Result<Vec<i64>, _>>()?;

        Ok(Profile {
            total_xp,
            level,
            xp,
            streak_days: xp::streak_days(&days, today),
            xp_today,
        })
    }

    /// Latest finished sessions (not hidden) and unlocked rewards, newest first.
    /// A reward comes before the session that unlocked it.
    pub fn activity(&self, limit: u32) -> anyhow::Result<Vec<Activity>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.code, r.name, a.name, u.unlocked_at FROM unlocked_rewards u
             JOIN rewards r ON r.id = u.reward_id
             LEFT JOIN apps a ON a.id = u.app_id
             ORDER BY u.unlocked_at DESC, u.id DESC LIMIT ?1",
        )?;
        let rewards = stmt.query_map([limit], |r| {
            let reward = i18n::reward_text(&r.get::<_, String>(0)?, "name", r.get(1)?);
            let app: Option<String> = r.get(2)?;
            let name = match app {
                Some(app) => format!("{reward} ({app})"),
                None => reward,
            };
            Ok(Activity::Reward {
                name,
                at: r.get(3)?,
            })
        })?;
        let mut events = rewards.collect::<Result<Vec<_>, _>>()?;

        let mut stmt = self.conn.prepare(
            "SELECT a.name, s.duration_s, s.xp_gained, s.ended_at FROM sessions s
             JOIN apps a ON a.id = s.app_id
             WHERE s.ended_at IS NOT NULL AND NOT s.hidden
             ORDER BY s.ended_at DESC, s.id DESC LIMIT ?1",
        )?;
        let sessions = stmt.query_map([limit], |r| {
            Ok(Activity::Session {
                app: r.get(0)?,
                secs: r.get::<_, i64>(1)? as u64,
                xp: r.get(2)?,
                at: r.get(3)?,
            })
        })?;
        for session in sessions {
            events.push(session?);
        }
        // Stable: at the same time, the reward stays first.
        events.sort_by_key(|e| match e {
            Activity::Session { at, .. } | Activity::Reward { at, .. } => std::cmp::Reverse(*at),
        });
        events.truncate(limit as usize);
        Ok(events)
    }

    /// What reward rules can test about a closed session. Returns the session's app too.
    pub fn session_facts(&self, session_id: i64) -> anyhow::Result<(i64, Facts)> {
        self.session_facts_at(session_id, unix_now())
    }

    fn session_facts_at(&self, session_id: i64, now: i64) -> anyhow::Result<(i64, Facts)> {
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
            app_level: xp::level_from_total(app_xp).0 as f64,
            level: profile.level as f64,
            streak_days: profile.streak_days as f64,
            total_hours: total_secs as f64 / 3600.0,
            total_sessions: total_sessions as f64,
            apps_this_week: apps_this_week as f64,
        };
        Ok((app_id, facts))
    }

    /// Rewards `app_id` can still unlock: global ones nobody unlocked yet, and per-app
    /// ones not unlocked for this app. Rewards tied to another app are skipped.
    pub fn pending_rewards(&self, app_id: i64) -> anyhow::Result<Vec<Reward>> {
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
    pub fn unlock_reward(
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

    /// Stats of closed sessions, for one app or (`None`) all of them.
    pub fn stats(&self, app_id: Option<i64>) -> anyhow::Result<Stats> {
        self.stats_at(app_id, unix_now())
    }

    fn stats_at(&self, app_id: Option<i64>, now: i64) -> anyhow::Result<Stats> {
        let mut stmt = self.conn.prepare(
            "SELECT a.name, strftime('%d/%m/%Y %H:%M', s.started_at, 'unixepoch', 'localtime'),
                s.duration_s, s.xp_gained
             FROM sessions s JOIN apps a ON a.id = s.app_id
             WHERE s.ended_at IS NOT NULL AND NOT s.hidden AND (?1 IS NULL OR s.app_id = ?1)
             ORDER BY s.started_at DESC, s.id DESC LIMIT ?2",
        )?;
        let sessions = stmt
            .query_map(params![app_id, STATS_SESSIONS], |r| {
                Ok(SessionRow {
                    app: r.get(0)?,
                    started: r.get(1)?,
                    duration_secs: r.get::<_, i64>(2)?.max(0) as u64,
                    xp: r.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let (session_count, total_secs, longest_secs): (u32, i64, i64) = self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(duration_s), 0), COALESCE(MAX(duration_s), 0)
             FROM sessions WHERE ended_at IS NOT NULL AND (?1 IS NULL OR app_id = ?1)",
            [app_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;

        // Time per `name`, most played first.
        let totals = |name: &str, group: &str| -> anyhow::Result<Vec<(String, u64)>> {
            let mut stmt = self.conn.prepare(&format!(
                "SELECT {name}, SUM(s.duration_s) AS secs
                 FROM sessions s JOIN apps a ON a.id = s.app_id
                 JOIN categories c ON c.id = a.category_id
                 WHERE s.ended_at IS NOT NULL AND (?1 IS NULL OR s.app_id = ?1)
                 GROUP BY {group} ORDER BY secs DESC, {name}"
            ))?;
            Ok(stmt
                .query_map([app_id], |r| {
                    Ok((r.get(0)?, r.get::<_, i64>(1)?.max(0) as u64))
                })?
                .collect::<Result<Vec<_>, _>>()?)
        };
        let by_category = totals("c.name", "c.id")?;
        let by_app = totals("a.name", "a.id")?;

        // Local day numbers (Julian days), compared with today's.
        let day = |column: &str| {
            format!("CAST(julianday(date({column}, 'unixepoch', 'localtime')) AS INTEGER)")
        };
        let today: i64 = self
            .conn
            .query_row(&format!("SELECT {}", day("?1")), [now], |r| r.get(0))?;
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} AS d, SUM(duration_s) FROM sessions
             WHERE ended_at IS NOT NULL AND (?1 IS NULL OR app_id = ?1)
             GROUP BY d HAVING d > ?2 - {ACTIVITY_DAYS} AND d <= ?2",
            day("ended_at")
        ))?;
        let mut daily = vec![0; ACTIVITY_DAYS];
        let rows = stmt.query_map(params![app_id, today], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (d, secs) = row?;
            daily[ACTIVITY_DAYS - 1 - (today - d) as usize] = secs.max(0) as u64;
        }

        Ok(Stats {
            sessions,
            session_count,
            total_secs: total_secs.max(0) as u64,
            longest_secs: longest_secs.max(0) as u64,
            by_category,
            by_app,
            daily,
            // Julian day of 1970-01-01, cast down like `today`.
            today: today - 2_440_587,
        })
    }

    /// Every reward with its unlocks, in definition order.
    pub fn reward_views(&self) -> anyhow::Result<Vec<RewardView>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.name, r.description, r.rule,
                r.scope = 'app' OR r.app_id IS NOT NULL, a.name, r.code
             FROM rewards r LEFT JOIN apps a ON a.id = r.app_id
             ORDER BY r.id",
        )?;
        let mut views = stmt
            .query_map([], |r| {
                let code: String = r.get(6)?;
                Ok(RewardView {
                    id: r.get(0)?,
                    name: i18n::reward_text(&code, "name", r.get(1)?),
                    description: i18n::reward_text(&code, "description", r.get(2)?),
                    rule: r.get(3)?,
                    per_app: r.get(4)?,
                    app: r.get(5)?,
                    unlocks: Vec::new(),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut stmt = self.conn.prepare(
            "SELECT u.reward_id, a.name,
                strftime('%d/%m/%Y', u.unlocked_at, 'unixepoch', 'localtime')
             FROM unlocked_rewards u LEFT JOIN apps a ON a.id = u.app_id
             ORDER BY u.unlocked_at, u.id",
        )?;
        let unlocks = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                Unlock {
                    app: r.get(1)?,
                    date: r.get(2)?,
                },
            ))
        })?;
        for unlock in unlocks {
            let (reward_id, unlock) = unlock?;
            if let Some(view) = views.iter_mut().find(|v| v.id == reward_id) {
                view.unlocks.push(unlock);
            }
        }
        Ok(views)
    }
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    fn db_with_app() -> (Database, i64, i64) {
        let db = Database::open_in_memory().unwrap();
        let cat = db.add_category("Jeux").unwrap();
        let app = db
            .add_app(&NewApp {
                name: "Hades".into(),
                launch_target: "steam://rungameid/1145360".into(),
                watch_exe: Some("Hades.exe".into()),
                category_id: cat,
            })
            .unwrap();
        (db, cat, app)
    }

    fn session(db: &Database, app: i64, ended_at: i64, secs: i64, xp: u32) {
        db.conn
            .execute(
                "INSERT INTO sessions (app_id, started_at, ended_at, duration_s, xp_gained)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![app, ended_at - secs, ended_at, secs, xp],
            )
            .unwrap();
    }

    #[test]
    fn add_and_list() {
        let (db, cat, _) = db_with_app();
        let cats = db.categories().unwrap();
        assert_eq!(cats.len(), 1);
        assert_eq!(cats[0].name, "Jeux");

        let apps = db.apps().unwrap();
        assert_eq!(apps.len(), 1);
        let hades = &apps[0];
        assert_eq!(hades.category_id, cat);
        assert_eq!(hades.watch_exe.as_deref(), Some("Hades.exe"));
        assert_eq!((hades.level, hades.xp, hades.total_secs), (1, 0, 0));
        assert_eq!(hades.last_played, None);
    }

    #[test]
    fn names_are_unique_ignoring_case() {
        let (db, cat, _) = db_with_app();
        assert!(db.add_category("JEUX").is_err());
        let dup = NewApp {
            name: "hades".into(),
            launch_target: "x".into(),
            watch_exe: None,
            category_id: cat,
        };
        assert!(db.add_app(&dup).is_err());
    }

    #[test]
    fn move_and_delete() {
        let (db, games, app) = db_with_app();
        let dev = db.add_category("Dev").unwrap();
        db.move_app(app, dev).unwrap();
        assert_eq!(db.apps().unwrap()[0].category_id, dev);

        db.delete_category(games).unwrap(); // now empty
        assert!(db.delete_category(dev).is_err()); // still holds Hades

        session(&db, app, 1_000, 60, 1);
        db.delete_app(app).unwrap();
        assert!(db.apps().unwrap().is_empty());
        let sessions: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sessions, 0);
    }

    #[test]
    fn app_stats_aggregate_sessions_and_rewards() {
        let (db, _, app) = db_with_app();
        session(&db, app, 10_000, 3_600, 60);
        session(&db, app, 20_000, 1_800, 30);
        db.conn
            .execute("UPDATE apps SET total_xp = 150 WHERE id = ?1", [app])
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO rewards (app_id, code, name, rule) VALUES (?1, 'hades_only', 'Fan', 'level >= 1')",
                [app],
            )
            .unwrap();
        let reward = db.conn.last_insert_rowid();
        db.unlock_reward(reward, Some(app), 1, 20_000).unwrap();

        let hades = &db.apps().unwrap()[0];
        assert_eq!(hades.total_secs, 5_400);
        assert_eq!(hades.last_played, Some(20_000));
        assert_eq!((hades.level, hades.xp), (2, 50));
        assert_eq!(hades.rewards, 1);
        let fan = Activity::Reward {
            name: "Fan (Hades)".into(),
            at: 20_000,
        };
        assert_eq!(db.activity(1).unwrap(), [fan]);
    }

    #[test]
    fn activity_mixes_sessions_and_rewards_newest_first() {
        let (db, _, app) = db_with_app();
        session(&db, app, 1_000, 600, 10);
        db.hide_sessions().unwrap();
        session(&db, app, 2_000, 600, 10);
        session(&db, app, 3_000, 1_200, 20);
        let premiers_pas = reward_id(&db, "premiers_pas");
        db.unlock_reward(premiers_pas, None, 1, 2_000).unwrap();

        let session = |secs, xp, at| Activity::Session {
            app: "Hades".into(),
            secs,
            xp,
            at,
        };
        let reward = Activity::Reward {
            name: "Premiers pas".into(),
            at: 2_000,
        };
        // The hidden session is left out; the reward precedes its session.
        assert_eq!(
            db.activity(5).unwrap(),
            [session(1_200, 20, 3_000), reward, session(600, 10, 2_000)]
        );
        assert_eq!(db.activity(1).unwrap().len(), 1);
    }

    fn reward_id(db: &Database, code: &str) -> i64 {
        db.conn
            .query_row("SELECT id FROM rewards WHERE code = ?1", [code], |r| {
                r.get(0)
            })
            .unwrap()
    }

    fn pending_codes(db: &Database, app: i64) -> Vec<String> {
        db.pending_rewards(app)
            .unwrap()
            .into_iter()
            .map(|r| r.code)
            .collect()
    }

    #[test]
    fn rewards_unlock_once_globally_or_per_app() {
        let (db, cat, hades) = db_with_app();
        let celeste = db
            .add_app(&NewApp {
                name: "Celeste".into(),
                launch_target: "celeste.exe".into(),
                watch_exe: None,
                category_id: cat,
            })
            .unwrap();
        let pending = db.pending_rewards(hades).unwrap();
        let marathon = pending.iter().find(|r| r.code == "marathon").unwrap();
        assert!(marathon.per_app);
        assert!(
            !pending
                .iter()
                .find(|r| r.code == "premiers_pas")
                .unwrap()
                .per_app
        );

        let s = db.start_session(hades, 1_000).unwrap();
        db.unlock_reward(reward_id(&db, "premiers_pas"), None, s, 1_000)
            .unwrap();
        db.unlock_reward(reward_id(&db, "marathon"), Some(hades), s, 1_000)
            .unwrap();
        for app in [hades, celeste] {
            assert!(!pending_codes(&db, app).contains(&"premiers_pas".to_string()));
        }
        assert!(!pending_codes(&db, hades).contains(&"marathon".to_string()));
        assert!(pending_codes(&db, celeste).contains(&"marathon".to_string()));

        // The same unlock twice is refused by the database.
        assert!(
            db.unlock_reward(reward_id(&db, "premiers_pas"), None, s, 2_000)
                .is_err()
        );
        assert!(
            db.unlock_reward(reward_id(&db, "marathon"), Some(hades), s, 2_000)
                .is_err()
        );

        let views = db.reward_views().unwrap();
        let view = views.iter().find(|v| v.name == "Marathon").unwrap();
        assert_eq!(view.unlocks.len(), 1);
        assert_eq!(view.unlocks[0].app.as_deref(), Some("Hades"));
        assert_eq!(db.apps().unwrap()[1].rewards, 1); // Hades (sorted after Celeste)
    }

    #[test]
    fn rewards_tied_to_another_app_are_skipped() {
        let (db, cat, hades) = db_with_app();
        let other = db
            .add_app(&NewApp {
                name: "Celeste".into(),
                launch_target: "celeste.exe".into(),
                watch_exe: None,
                category_id: cat,
            })
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO rewards (app_id, code, name, rule) VALUES (?1, 'fan', 'Fan', 'level >= 1')",
                [hades],
            )
            .unwrap();
        assert!(pending_codes(&db, hades).contains(&"fan".to_string()));
        assert!(!pending_codes(&db, other).contains(&"fan".to_string()));
    }

    #[test]
    fn stats_aggregate_sessions_by_day_and_category() {
        let (db, _, hades) = db_with_app();
        let dev = db.add_category("Dev").unwrap();
        let code = db
            .add_app(&NewApp {
                name: "Code".into(),
                launch_target: "code.exe".into(),
                watch_exe: None,
                category_id: dev,
            })
            .unwrap();
        // Noon UTC, like `profile_counts_today_and_streak`.
        let now = 20_000 * DAY + DAY / 2;
        session(&db, hades, now - 60, 3_600, 60);
        session(&db, hades, now - 2 * DAY, 1_800, 30);
        session(&db, code, now - 60, 600, 10);
        session(&db, code, now - 100 * DAY, 600, 10); // outside the activity chart

        let all = db.stats_at(None, now).unwrap();
        assert_eq!(all.session_count, 4);
        assert_eq!((all.total_secs, all.longest_secs), (6_600, 3_600));
        assert_eq!(
            all.by_category,
            [("Jeux".to_string(), 5_400), ("Dev".to_string(), 1_200)]
        );
        assert_eq!(
            all.by_app,
            [("Hades".to_string(), 5_400), ("Code".to_string(), 1_200)]
        );
        assert_eq!(all.daily.len(), ACTIVITY_DAYS);
        assert_eq!(all.daily[ACTIVITY_DAYS - 1], 4_200); // today
        assert_eq!(all.daily[ACTIVITY_DAYS - 3], 1_800); // two days ago
        assert_eq!(all.daily.iter().sum::<u64>(), 6_000);
        assert_eq!(all.today, 20_000);
        assert_eq!(all.sessions[0].app, "Code"); // most recently started first

        let only = db.stats_at(Some(code), now).unwrap();
        assert_eq!(only.session_count, 2);
        assert!(only.sessions.iter().all(|s| s.app == "Code"));
        assert_eq!(only.by_category, [("Dev".to_string(), 1_200)]);
    }

    #[test]
    fn session_facts_measure_the_closed_session() {
        let (db, _, app) = db_with_app();
        let now = 20_000 * DAY + DAY / 2;
        session(&db, app, now - 3 * DAY, 3_600, 0);
        let id = db.start_session(app, now - 7_200).unwrap();
        db.end_session(id, now, 7_200).unwrap();
        db.add_session_xp(id, 150).unwrap();

        let (app_id, facts) = db.session_facts_at(id, now).unwrap();
        assert_eq!(app_id, app);
        assert_eq!(facts.session_minutes, 120.0);
        assert_eq!(facts.app_hours, 3.0);
        assert_eq!(facts.app_sessions, 2.0);
        assert_eq!(facts.app_level, 2.0);
        assert_eq!(facts.total_sessions, 2.0);
        assert_eq!(facts.apps_this_week, 1.0);
        assert_eq!(facts.streak_days, 1.0);
        assert!((0.0..24.0).contains(&facts.session_hour));
    }

    fn session_rows(db: &Database) -> Vec<(Option<i64>, i64)> {
        let mut stmt = db
            .conn
            .prepare("SELECT ended_at, duration_s FROM sessions ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn session_lifecycle() {
        let (db, _, app) = db_with_app();
        let id = db.start_session(app, 1_000).unwrap();
        assert_eq!(session_rows(&db), [(None, 0)]);
        assert_eq!(db.apps().unwrap()[0].total_secs, 0); // open sessions don't count yet

        db.checkpoint_session(id, 120).unwrap();
        assert_eq!(session_rows(&db), [(None, 120)]);

        assert!(db.end_session(id, 1_300, 300).unwrap());
        assert_eq!(session_rows(&db), [(Some(1_300), 300)]);
        db.checkpoint_session(id, 999).unwrap(); // closed: ignored
        let hades = &db.apps().unwrap()[0];
        assert_eq!((hades.total_secs, hades.last_played), (300, Some(1_300)));
    }

    #[test]
    fn short_sessions_are_dropped() {
        let (db, _, app) = db_with_app();
        let id = db.start_session(app, 1_000).unwrap();
        assert!(!db.end_session(id, 1_010, 10).unwrap());
        assert!(session_rows(&db).is_empty());
    }

    #[test]
    fn orphans_close_at_last_checkpoint() {
        let (db, _, app) = db_with_app();
        let checkpointed = db.start_session(app, 1_000).unwrap();
        db.checkpoint_session(checkpointed, 600).unwrap();
        db.start_session(app, 5_000).unwrap(); // crashed before its first checkpoint
        session(&db, app, 9_000, 120, 0); // already closed: untouched

        assert_eq!(
            db.close_orphan_sessions().unwrap(),
            [ClosedSession {
                session_id: checkpointed,
                app_id: app,
                secs: 600
            }]
        );
        assert_eq!(session_rows(&db), [(Some(1_600), 600), (Some(9_000), 120)]);
    }

    #[test]
    fn clearing_history_then_stats_keeps_running_sessions_and_xp() {
        let (db, _, app) = db_with_app();
        let id = db.start_session(app, 1_000).unwrap();
        db.end_session(id, 2_000, 1_000).unwrap();
        db.add_session_xp(id, 120).unwrap();
        db.start_session(app, 5_000).unwrap(); // still running

        assert_eq!(db.hide_sessions().unwrap(), 1);
        let stats = db.stats(None).unwrap();
        assert!(stats.sessions.is_empty());
        assert_eq!(stats.session_count, 1); // still counted

        assert_eq!(db.clear_sessions().unwrap(), 1);
        assert_eq!(session_rows(&db), [(None, 0)]);
        assert_eq!(db.apps().unwrap()[0].total_xp, 120);
        assert_eq!(db.stats(None).unwrap().session_count, 0);
    }

    #[test]
    fn session_xp_goes_to_its_app() {
        let (db, _, app) = db_with_app();
        let id = db.start_session(app, 1_000).unwrap();
        db.end_session(id, 2_000, 1_000).unwrap();
        db.add_session_xp(id, 120).unwrap();

        let hades = &db.apps().unwrap()[0];
        assert_eq!((hades.total_xp, hades.level, hades.xp), (120, 2, 20));
        let gained: u32 = db
            .conn
            .query_row("SELECT xp_gained FROM sessions WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(gained, 120);

        db.set_app_xp(app, 5).unwrap();
        assert_eq!(db.apps().unwrap()[0].total_xp, 5);
        assert_eq!(db.profile().unwrap().total_xp, 5);
    }

    #[test]
    fn profile_counts_today_and_streak() {
        let (db, _, app) = db_with_app();
        // Noon UTC: one minute earlier is still the same local day in nearly every time zone.
        let now = 20_000 * DAY + DAY / 2;
        session(&db, app, now - 60, 600, 40);
        session(&db, app, now - DAY, 600, 10);
        session(&db, app, now - 2 * DAY, 600, 10);
        session(&db, app, now - 5 * DAY, 600, 10);

        let profile = db.profile_at(now).unwrap();
        assert_eq!(profile.xp_today, 40);
        assert_eq!(profile.streak_days, 3);
        assert_eq!(profile.level, 1);
    }
}
