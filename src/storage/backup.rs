//! `:export` / `:import`: apps and finished sessions as JSON, for backups, external
//! analysis, or moving to another PC. Importing merges, so it never loses data and
//! importing the same file twice changes nothing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::db::Database;

/// Bumped when the file format changes incompatibly.
const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct Backup {
    version: u32,
    /// Unix seconds.
    exported_at: i64,
    apps: Vec<BackupApp>,
    sessions: Vec<BackupSession>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupApp {
    name: String,
    category: String,
    launch_target: String,
    watch_exe: Option<String>,
    /// Includes XP no session accounts for (given by hand in older versions).
    total_xp: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupSession {
    app: String,
    /// Unix seconds.
    started_at: i64,
    ended_at: i64,
    duration_s: i64,
    xp_gained: u32,
}

/// What an import added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Imported {
    pub apps: usize,
    pub sessions: usize,
}

impl Database {
    /// Writes every app and finished session to `path`. Returns `(apps, sessions)`.
    pub fn export_to(&self, path: &Path) -> anyhow::Result<(usize, usize)> {
        let backup = self.backup()?;
        let json = serde_json::to_string_pretty(&backup)?;
        std::fs::write(path, json)
            .with_context(|| t!("error.cannot_write", path = path.display()))?;
        Ok((backup.apps.len(), backup.sessions.len()))
    }

    /// Merges a file written by `export_to`. Apps are matched by name: an existing app
    /// keeps its target and category, and gains the XP of its newly imported sessions.
    /// A session already present (same app, same start) is skipped.
    pub fn import_from(&self, path: &Path) -> anyhow::Result<Imported> {
        self.merge(&read_backup(path)?)
    }

    /// The apps importing `path` would add, as `(name, launch_target)`. A backup can come
    /// from someone else and its targets then launch from the dashboard, so they are shown
    /// before importing.
    pub fn import_preview(&self, path: &Path) -> anyhow::Result<Vec<(String, String)>> {
        let mut added = Vec::new();
        for app in read_backup(path)?.apps {
            let exists: bool = self.conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM apps WHERE name = ?1)",
                [&app.name],
                |r| r.get(0),
            )?;
            if !exists {
                added.push((app.name, app.launch_target));
            }
        }
        Ok(added)
    }

    /// `Documents\cmdboard-<yyyy-mm-dd>.json`, or in `%APPDATA%\CmdBoard` without Documents.
    pub fn default_export_path(&self) -> anyhow::Result<PathBuf> {
        let dir = match directories::UserDirs::new().and_then(|u| u.document_dir().map(Into::into))
        {
            Some(dir) => dir,
            None => super::db::data_dir()?,
        };
        let date: String =
            self.conn
                .query_row("SELECT strftime('%Y-%m-%d', 'now', 'localtime')", [], |r| {
                    r.get(0)
                })?;
        Ok(dir.join(format!("cmdboard-{date}.json")))
    }

    fn backup(&self) -> anyhow::Result<Backup> {
        let mut stmt = self.conn.prepare(
            "SELECT a.name, c.name, a.launch_target, a.watch_exe, a.total_xp
             FROM apps a JOIN categories c ON c.id = a.category_id ORDER BY a.id",
        )?;
        let apps = stmt
            .query_map([], |r| {
                Ok(BackupApp {
                    name: r.get(0)?,
                    category: r.get(1)?,
                    launch_target: r.get(2)?,
                    watch_exe: r.get(3)?,
                    total_xp: r.get(4)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        let mut stmt = self.conn.prepare(
            "SELECT a.name, s.started_at, s.ended_at, s.duration_s, s.xp_gained
             FROM sessions s JOIN apps a ON a.id = s.app_id
             WHERE s.ended_at IS NOT NULL ORDER BY s.started_at",
        )?;
        let sessions = stmt
            .query_map([], |r| {
                Ok(BackupSession {
                    app: r.get(0)?,
                    started_at: r.get(1)?,
                    ended_at: r.get(2)?,
                    duration_s: r.get(3)?,
                    xp_gained: r.get(4)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(Backup {
            version: FORMAT_VERSION,
            exported_at: super::unix_now(),
            apps,
            sessions,
        })
    }

    fn merge(&self, backup: &Backup) -> anyhow::Result<Imported> {
        let tx = self.conn.unchecked_transaction()?;
        let mut imported = Imported {
            apps: 0,
            sessions: 0,
        };
        // Lowercased name -> (id, created by this import).
        let mut ids: HashMap<String, (i64, bool)> = HashMap::new();
        for app in &backup.apps {
            let existing: Option<i64> = tx
                .query_row("SELECT id FROM apps WHERE name = ?1", [&app.name], |r| {
                    r.get(0)
                })
                .optional()?;
            let entry = match existing {
                Some(id) => (id, false),
                None => {
                    tx.execute(
                        "INSERT OR IGNORE INTO categories (name) VALUES (?1)",
                        [&app.category],
                    )?;
                    tx.execute(
                        "INSERT INTO apps (name, launch_target, watch_exe, category_id, total_xp)
                         SELECT ?1, ?2, ?3, id, ?5 FROM categories WHERE name = ?4",
                        params![
                            app.name,
                            app.launch_target,
                            app.watch_exe,
                            app.category,
                            app.total_xp
                        ],
                    )?;
                    imported.apps += 1;
                    (tx.last_insert_rowid(), true)
                }
            };
            ids.insert(app.name.to_lowercase(), entry);
        }
        for session in &backup.sessions {
            let Some(&(app_id, created)) = ids.get(&session.app.to_lowercase()) else {
                continue;
            };
            let added = tx.execute(
                "INSERT INTO sessions (app_id, started_at, ended_at, duration_s, xp_gained)
                 SELECT ?1, ?2, ?3, ?4, ?5 WHERE NOT EXISTS
                    (SELECT 1 FROM sessions WHERE app_id = ?1 AND started_at = ?2)",
                params![
                    app_id,
                    session.started_at,
                    session.ended_at,
                    session.duration_s,
                    session.xp_gained
                ],
            )?;
            imported.sessions += added;
            // A created app got its total XP above, sessions included.
            if added > 0 && !created {
                tx.execute(
                    "UPDATE apps SET total_xp = total_xp + ?2 WHERE id = ?1",
                    params![app_id, session.xp_gained],
                )?;
            }
        }
        tx.commit()?;
        Ok(imported)
    }
}

fn read_backup(path: &Path) -> anyhow::Result<Backup> {
    let text = std::fs::read_to_string(path)
        .with_context(|| t!("error.cannot_read", path = path.display()))?;
    let backup: Backup = serde_json::from_str(&text)
        .with_context(|| t!("error.bad_backup", path = path.display()))?;
    if backup.version != FORMAT_VERSION {
        bail!(t!("error.backup_version", version = backup.version));
    }
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_export_path_is_dated_json() {
        let db = Database::open_in_memory().unwrap();
        let path = db.default_export_path().unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("cmdboard-") && name.ends_with(".json"),
            "{name}"
        );
        assert_eq!(name.len(), "cmdboard-yyyy-mm-dd.json".len());
    }

    #[test]
    fn sessions_of_unknown_apps_are_skipped() {
        let file =
            std::env::temp_dir().join(format!("cmdboard-backup-ghost-{}.json", std::process::id()));
        let json = r#"{"version":1,"exported_at":0,"apps":[],"sessions":[
            {"app":"Ghost","started_at":1,"ended_at":601,"duration_s":600,"xp_gained":10}]}"#;
        std::fs::write(&file, json).unwrap();
        let db = Database::open_in_memory().unwrap();
        assert_eq!(
            db.import_from(&file).unwrap(),
            Imported {
                apps: 0,
                sessions: 0
            }
        );
        std::fs::remove_file(&file).unwrap();
    }
    use crate::storage::models::NewApp;

    fn db_with_session(app: &str, category: &str, started_at: i64, xp: u32) -> Database {
        let db = Database::open_in_memory().unwrap();
        let category_id = db.add_category(category).unwrap();
        let app_id = db
            .add_app(&NewApp {
                name: app.into(),
                launch_target: format!(r"C:\Games\{app}.exe"),
                watch_exe: Some(format!("{app}.exe")),
                category_id,
            })
            .unwrap();
        let session = db.start_session(app_id, started_at).unwrap();
        db.end_session(session, started_at + 600, 600).unwrap();
        db.add_session_xp(session, xp).unwrap();
        db
    }

    fn app_xp(db: &Database, name: &str) -> u32 {
        db.apps()
            .unwrap()
            .into_iter()
            .find(|a| a.name == name)
            .unwrap()
            .total_xp
    }

    #[test]
    fn export_then_import_merges_without_duplicates() {
        let dir = std::env::temp_dir().join(format!("cmdboard-backup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("export.json");

        let old_pc = db_with_session("Hades", "Jeux", 1_000, 50);
        old_pc.set_app_xp(old_pc.apps().unwrap()[0].id, 80).unwrap(); // + XP no session accounts for
        assert_eq!(old_pc.export_to(&file).unwrap(), (1, 1));

        // The new PC already has Hades (other session) and lacks the "Jeux" category.
        let new_pc = db_with_session("hades", "Games", 5_000, 20);
        assert!(new_pc.import_preview(&file).unwrap().is_empty()); // known, whatever the case
        let fresh = Database::open_in_memory().unwrap();
        assert_eq!(
            fresh.import_preview(&file).unwrap(),
            [("Hades".to_string(), r"C:\Games\Hades.exe".to_string())]
        );
        let imported = new_pc.import_from(&file).unwrap();
        assert_eq!(
            imported,
            Imported {
                apps: 0,
                sessions: 1
            }
        );
        assert_eq!(app_xp(&new_pc, "hades"), 70);

        fresh.import_from(&file).unwrap();
        let apps = fresh.apps().unwrap();
        assert_eq!(apps[0].launch_target, r"C:\Games\Hades.exe");
        assert_eq!(apps[0].total_xp, 80);
        assert_eq!(apps[0].total_secs, 600);
        assert!(fresh.categories().unwrap().iter().any(|c| c.name == "Jeux"));

        // Twice: nothing more.
        let again = fresh.import_from(&file).unwrap();
        assert_eq!(
            again,
            Imported {
                apps: 0,
                sessions: 0
            }
        );
        assert_eq!(app_xp(&fresh, "Hades"), 80);

        std::fs::write(
            &file,
            r#"{"version": 99, "exported_at": 0, "apps": [], "sessions": []}"#,
        )
        .unwrap();
        assert!(fresh.import_from(&file).is_err());
        std::fs::write(&file, "not json").unwrap();
        assert!(fresh.import_from(&file).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(fresh.import_from(&file).is_err());
    }
}
