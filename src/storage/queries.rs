//! CRUD on top of `Database`. Every read returns owned models that `App` caches.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use rusqlite::params;

use super::db::Database;
use super::models::{AppEntry, Category, ClosedSession, NewApp, Profile};
use crate::core::xp;

/// Shorter sessions (a quick launch and close) are not recorded.
pub const MIN_SESSION_SECS: u64 = 60;

impl Database {
    pub fn categories(&self) -> anyhow::Result<Vec<Category>> {
        let mut stmt = self.conn.prepare("SELECT id, name FROM categories ORDER BY id")?;
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
            .with_context(|| format!("impossible d'ajouter la catégorie « {name} »"))?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Fails while the category still holds apps.
    pub fn delete_category(&self, id: i64) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM categories WHERE id = ?1", [id])
            .context("la catégorie contient encore des apps")?;
        Ok(())
    }

    /// All apps sorted by name, with time played, last session and rewards aggregated.
    pub fn apps(&self) -> anyhow::Result<Vec<AppEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.id, a.name, a.launch_target, a.watch_exe, a.category_id, a.total_xp,
                (SELECT COALESCE(SUM(duration_s), 0) FROM sessions s
                    WHERE s.app_id = a.id AND s.ended_at IS NOT NULL),
                (SELECT MAX(ended_at) FROM sessions s WHERE s.app_id = a.id),
                (SELECT COUNT(*) FROM unlocked_rewards u JOIN rewards r ON r.id = u.reward_id
                    WHERE r.app_id = a.id)
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
            .with_context(|| format!("impossible d'ajouter « {} »", app.name))?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn move_app(&self, app_id: i64, category_id: i64) -> anyhow::Result<()> {
        self.conn
            .execute(
                "UPDATE apps SET category_id = ?2 WHERE id = ?1",
                [app_id, category_id],
            )
            .context("impossible de déplacer l'app")?;
        Ok(())
    }

    /// Also deletes the app's sessions and rewards (`ON DELETE CASCADE`).
    pub fn delete_app(&self, app_id: i64) -> anyhow::Result<()> {
        self.conn.execute("DELETE FROM apps WHERE id = ?1", [app_id])?;
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
            self.conn.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
            return Ok(false);
        }
        self.conn.execute(
            "UPDATE sessions SET ended_at = ?2, duration_s = ?3 WHERE id = ?1",
            params![id, ended_at, secs as i64],
        )?;
        Ok(true)
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

    /// Overwrites an app's XP (`:xp`), outside of any session.
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
                .query_row("SELECT COALESCE(SUM(total_xp), 0) FROM apps", [], |r| r.get(0))?;
        let (level, xp) = xp::level_from_total(total_xp);

        let xp_today: u32 = self.conn.query_row(
            "SELECT COALESCE(SUM(xp_gained), 0) FROM sessions
             WHERE ended_at IS NOT NULL
               AND date(ended_at, 'unixepoch', 'localtime') = date(?1, 'unixepoch', 'localtime')",
            [now],
            |r| r.get(0),
        )?;

        let day = "CAST(julianday(date(?1, 'unixepoch', 'localtime')) AS INTEGER)";
        let today: i64 = self.conn.query_row(&format!("SELECT {day}"), [now], |r| r.get(0))?;
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

    /// Latest unlocked rewards, newest first, as "Name (App)" or "Name" for global ones.
    pub fn recent_rewards(&self, limit: u32) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.name, a.name FROM unlocked_rewards u
             JOIN rewards r ON r.id = u.reward_id
             LEFT JOIN apps a ON a.id = r.app_id
             ORDER BY u.unlocked_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |r| {
            let reward: String = r.get(0)?;
            let app: Option<String> = r.get(1)?;
            Ok(match app {
                Some(app) => format!("{reward} ({app})"),
                None => reward,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
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
                "INSERT INTO rewards (app_id, code, name, rule) VALUES (?1, 'marathon', 'Marathon', 'true')",
                [app],
            )
            .unwrap();
        db.conn
            .execute("INSERT INTO unlocked_rewards (reward_id, unlocked_at) VALUES (1, 20000)", [])
            .unwrap();

        let hades = &db.apps().unwrap()[0];
        assert_eq!(hades.total_secs, 5_400);
        assert_eq!(hades.last_played, Some(20_000));
        assert_eq!((hades.level, hades.xp), (2, 50));
        assert_eq!(hades.rewards, 1);
        assert_eq!(db.recent_rewards(3).unwrap(), ["Marathon (Hades)"]);
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
            [ClosedSession { session_id: checkpointed, app_id: app, secs: 600 }]
        );
        assert_eq!(session_rows(&db), [(Some(1_600), 600), (Some(9_000), 120)]);
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
            .query_row("SELECT xp_gained FROM sessions WHERE id = ?1", [id], |r| r.get(0))
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
