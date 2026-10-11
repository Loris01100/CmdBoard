//! Time played before `CmdBoard` (plan section 11): imported once per app from Steam and
//! given as XP, without a session, so stats, streak and session rewards stay what
//! `CmdBoard` saw.

use rusqlite::params;

use super::db::Database;
use super::{to_i64, to_u64};
use crate::core::xp;

/// What an import gave an app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PastPlaytime {
    pub secs: u64,
    pub xp: u32,
}

impl Database {
    /// Apps never given their past play time: `(id, launch_target)`.
    pub fn apps_without_past(&self) -> anyhow::Result<Vec<(i64, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, launch_target FROM apps WHERE past_secs IS NULL ORDER BY id")?;
        let apps = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        Ok(apps)
    }

    /// Records `played_secs` (Steam's count) less what the app's finished sessions
    /// already count, as Steam counted them too, and adds its XP, in one transaction.
    /// `None` if the app was already imported, or no longer exists.
    pub fn import_past_playtime(
        &self,
        app_id: i64,
        played_secs: u64,
    ) -> anyhow::Result<Option<PastPlaytime>> {
        let tx = self.conn.unchecked_transaction()?;
        let tracked: Option<i64> = match tx.query_row(
            "SELECT (SELECT COALESCE(SUM(duration_s), 0) FROM sessions
                     WHERE app_id = ?1 AND ended_at IS NOT NULL)
             FROM apps WHERE id = ?1 AND past_secs IS NULL",
            [app_id],
            |r| r.get(0),
        ) {
            Ok(secs) => Some(secs),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(e.into()),
        };
        let Some(tracked) = tracked else {
            return Ok(None);
        };
        let secs = played_secs.saturating_sub(to_u64(tracked));
        let xp = xp::xp_for_past_minutes(secs / 60);
        tx.execute(
            "UPDATE apps SET past_secs = ?2, total_xp = total_xp + ?3 WHERE id = ?1",
            params![app_id, to_i64(secs), xp],
        )?;
        tx.commit()?;
        Ok(Some(PastPlaytime { secs, xp }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::models::NewApp;

    #[test]
    fn past_playtime_is_imported_once_without_what_sessions_counted() {
        let db = Database::open_in_memory().unwrap();
        let category_id = db.add_category("Jeux").unwrap();
        let app = |name: &str| NewApp {
            name: name.into(),
            launch_target: "steam://rungameid/1245620".into(),
            watch_exe: Some("eldenring.exe".into()),
            category_id,
            launch_args: None,
        };
        let elden = db.add_app(&app("Elden Ring")).unwrap();
        let hades = db.add_app(&app("Hades")).unwrap();
        // An hour CmdBoard tracked, which Steam counts too; an open one does not count.
        let session = db.start_session(elden, 1_000).unwrap();
        db.end_session(session, 4_600, 3_600).unwrap();
        db.start_session(elden, 9_000).unwrap();
        assert_eq!(db.apps_without_past().unwrap().len(), 2);

        let imported = db.import_past_playtime(elden, 3 * 3_600 + 59).unwrap();
        assert_eq!(
            imported,
            Some(PastPlaytime {
                secs: 7_259,
                xp: 120
            })
        );
        assert_eq!(db.import_past_playtime(elden, 99_999).unwrap(), None); // once
        // Less than tracked: nothing, but imported all the same.
        let imported = db.import_past_playtime(hades, 0).unwrap();
        assert_eq!(imported, Some(PastPlaytime { secs: 0, xp: 0 }));
        assert_eq!(db.import_past_playtime(404, 60).unwrap(), None);

        assert!(db.apps_without_past().unwrap().is_empty());
        let xp = |id| {
            db.apps()
                .unwrap()
                .iter()
                .find(|a| a.id == id)
                .unwrap()
                .total_xp
        };
        assert_eq!((xp(elden), xp(hades)), (120, 0));
    }
}
