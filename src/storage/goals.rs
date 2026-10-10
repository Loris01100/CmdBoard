//! Goals and limits (`:goal`, `:limit`), and the time played this day and week they
//! are measured against. Days and weeks follow the local time zone, like the profile.

use rusqlite::params;

use super::db::Database;
use super::models::{Goal, GoalTarget, Usage};
use super::queries::unix_now;
use super::to_u64;
use crate::core::goals::{GoalKind, Period};

/// Local Julian day number of a Unix time column or parameter, as in `stats`.
fn day(column: &str) -> String {
    format!("CAST(julianday(date({column}, 'unixepoch', 'localtime')) AS INTEGER)")
}

/// The same for the Monday of its week (`weekday 0` is the next Sunday, or today).
fn monday(column: &str) -> String {
    format!(
        "CAST(julianday(date({column}, 'unixepoch', 'localtime', 'weekday 0', '-6 days')) AS INTEGER)"
    )
}

/// Unix time of the local midnight starting the day of a Unix time column or parameter,
/// moved by the date `modifiers` (each preceded by a comma).
fn midnight(column: &str, modifiers: &str) -> String {
    format!(
        "CAST(strftime('%s', date({column}, 'unixepoch', 'localtime'{modifiers}), 'utc') AS INTEGER)"
    )
}

fn target_ids(target: GoalTarget) -> (Option<i64>, Option<i64>) {
    match target {
        GoalTarget::All => (None, None),
        GoalTarget::App(id) => (Some(id), None),
        GoalTarget::Category(id) => (None, Some(id)),
    }
}

impl Database {
    /// Sets the goal or limit of `target`, replacing the previous one of that kind.
    pub fn set_goal(
        &self,
        kind: GoalKind,
        target: GoalTarget,
        minutes: u32,
        period: Period,
    ) -> anyhow::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        self.remove_goal(kind, target)?;
        let (app_id, category_id) = target_ids(target);
        self.conn.execute(
            "INSERT INTO goals (kind, app_id, category_id, minutes, period)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![kind.code(), app_id, category_id, minutes, period.code()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Returns whether `target` had a goal or limit of that kind.
    pub fn remove_goal(&self, kind: GoalKind, target: GoalTarget) -> anyhow::Result<bool> {
        let (app_id, category_id) = target_ids(target);
        let removed = self.conn.execute(
            "DELETE FROM goals WHERE kind = ?1 AND app_id IS ?2 AND category_id IS ?3",
            params![kind.code(), app_id, category_id],
        )?;
        Ok(removed > 0)
    }

    /// Every goal and limit, in the order they were set.
    pub fn goals(&self) -> anyhow::Result<Vec<Goal>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, app_id, category_id, minutes, period FROM goals ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            let kind: String = r.get(1)?;
            let period: String = r.get(5)?;
            let target = match (r.get(2)?, r.get(3)?) {
                (Some(app), _) => GoalTarget::App(app),
                (None, Some(category)) => GoalTarget::Category(category),
                (None, None) => GoalTarget::All,
            };
            Ok((r.get(0)?, kind, target, r.get(4)?, period))
        })?;
        let mut goals = Vec::new();
        for row in rows {
            let (id, kind, target, minutes, period) = row?;
            // The table's CHECKs make these known; skip anything else rather than fail.
            if let (Some(kind), Some(period)) = (GoalKind::parse(&kind), Period::parse(&period)) {
                goals.push(Goal {
                    id,
                    kind,
                    target,
                    minutes,
                    period,
                });
            }
        }
        Ok(goals)
    }

    /// Time played today and this week, by app. Sessions count on the day they ended.
    pub fn usage(&self) -> anyhow::Result<Usage> {
        self.usage_at(unix_now())
    }

    pub(super) fn usage_at(&self, now: i64) -> anyhow::Result<Usage> {
        // Day numbers for `Usage`, and the Unix times at which today and this week began,
        // so the sessions are found by `ended_at` through its index.
        let (day_number, week, day_start, week_start): (i64, i64, i64, i64) = self.conn.query_row(
            &format!(
                "SELECT {}, {}, {}, {}",
                day("?1"),
                monday("?1"),
                midnight("?1", ""),
                midnight("?1", ", 'weekday 0', '-6 days'"),
            ),
            [now],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        let mut stmt = self.conn.prepare(
            "SELECT app_id, SUM(CASE WHEN ended_at >= ?1 THEN duration_s ELSE 0 END),
                SUM(duration_s)
             FROM sessions WHERE ended_at >= ?2
             GROUP BY app_id",
        )?;
        let by_app = stmt
            .query_map([day_start, week_start], |r| {
                Ok((r.get(0)?, (to_u64(r.get(1)?), to_u64(r.get(2)?))))
            })?
            .collect::<Result<_, _>>()?;
        Ok(Usage {
            day: day_number,
            week,
            by_app,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::models::NewApp;

    fn db_with_app() -> (Database, i64, i64) {
        let db = Database::open_in_memory().unwrap();
        let category = db.add_category("Jeux").unwrap();
        let app = db
            .add_app(&NewApp {
                name: "Hades".into(),
                launch_target: "hades.exe".into(),
                watch_exe: None,
                category_id: category,
                launch_args: None,
            })
            .unwrap();
        (db, category, app)
    }

    #[test]
    fn one_goal_and_one_limit_per_target() {
        let (db, category, app) = db_with_app();
        db.set_goal(GoalKind::Goal, GoalTarget::App(app), 600, Period::Week)
            .unwrap();
        db.set_goal(GoalKind::Goal, GoalTarget::App(app), 60, Period::Day)
            .unwrap(); // replaces it
        db.set_goal(GoalKind::Limit, GoalTarget::App(app), 120, Period::Day)
            .unwrap();
        db.set_goal(
            GoalKind::Limit,
            GoalTarget::Category(category),
            180,
            Period::Day,
        )
        .unwrap();
        db.set_goal(GoalKind::Limit, GoalTarget::All, 240, Period::Day)
            .unwrap();
        let goals = db.goals().unwrap();
        let summary: Vec<_> = goals
            .iter()
            .map(|g| (g.kind, g.target, g.minutes, g.period))
            .collect();
        assert_eq!(
            summary,
            [
                (GoalKind::Goal, GoalTarget::App(app), 60, Period::Day),
                (GoalKind::Limit, GoalTarget::App(app), 120, Period::Day),
                (
                    GoalKind::Limit,
                    GoalTarget::Category(category),
                    180,
                    Period::Day
                ),
                (GoalKind::Limit, GoalTarget::All, 240, Period::Day),
            ]
        );

        assert!(db.remove_goal(GoalKind::Limit, GoalTarget::All).unwrap());
        assert!(!db.remove_goal(GoalKind::Limit, GoalTarget::All).unwrap());
        assert_eq!(db.goals().unwrap().len(), 3);

        // Removing the app removes its goals; its category's stay.
        db.execute_for_tests(&format!("DELETE FROM apps WHERE id = {app}"));
        assert_eq!(db.goals().unwrap().len(), 1);
    }

    #[test]
    fn usage_counts_today_and_this_week() {
        let (db, _, app) = db_with_app();
        // Thursday 2026-10-08, 12:00 local, then sessions ending on various days.
        let noon = |date: &str| -> i64 {
            db.conn
                .query_row(
                    "SELECT CAST(strftime('%s', ?1 || ' 12:00', 'utc') AS INTEGER)",
                    [date],
                    |r| r.get(0),
                )
                .unwrap()
        };
        let now = noon("2026-10-08");
        for (date, secs) in [
            ("2026-10-08", 600),  // today
            ("2026-10-05", 1200), // Monday: this week
            ("2026-10-04", 3000), // Sunday: last week
        ] {
            let id = db.start_session(app, noon(date) - 7200).unwrap();
            db.close_session(id, noon(date), secs).unwrap();
        }
        let open = db.start_session(app, now).unwrap(); // running: not counted
        db.checkpoint_session(open, 900).unwrap();

        let usage = db.usage_at(now).unwrap();
        assert_eq!(usage.by_app[&app], (600, 1800));
        assert_eq!(usage.day - usage.week, 3); // Thursday is 3 days after Monday

        // On a Monday, the week starts today.
        let monday = db.usage_at(noon("2026-10-05")).unwrap();
        assert_eq!(monday.day, monday.week);
    }

    #[test]
    fn usage_starts_at_local_midnight() {
        let (db, _, app) = db_with_app();
        let local = |time: &str| -> i64 {
            db.conn
                .query_row(
                    "SELECT CAST(strftime('%s', ?1, 'utc') AS INTEGER)",
                    [time],
                    |r| r.get(0),
                )
                .unwrap()
        };
        for (ended, secs) in [
            ("2026-10-04 23:59:59", 60u64), // Sunday: last week
            ("2026-10-05 00:00:00", 120),   // Monday midnight: this week
            ("2026-10-07 23:59:59", 240),   // Wednesday: this week, not today
            ("2026-10-08 00:00:00", 480),   // Thursday midnight: today
        ] {
            let start = local(ended) - i64::try_from(secs).unwrap();
            let id = db.start_session(app, start).unwrap();
            db.close_session(id, local(ended), secs).unwrap();
        }
        let usage = db.usage_at(local("2026-10-08 12:00:00")).unwrap();
        assert_eq!(usage.by_app[&app], (480, 840));

        let plan: String = db
            .conn
            .query_row(
                "EXPLAIN QUERY PLAN SELECT * FROM sessions WHERE ended_at >= 0",
                [],
                |r| r.get(3),
            )
            .unwrap();
        assert!(plan.contains("sessions_ended_at"), "{plan}");
    }
}
