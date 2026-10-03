use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use rusqlite::Connection;

use super::models::NewApp;

/// Schema migrations, applied in order. `PRAGMA user_version` holds how many have run.
/// Never edit a migration that has shipped: append a new one instead.
const MIGRATIONS: &[&str] = &[
    // v1: initial schema (plan section 3). Timestamps are Unix seconds.
    "CREATE TABLE categories (
        id    INTEGER PRIMARY KEY,
        name  TEXT NOT NULL UNIQUE COLLATE NOCASE,
        color TEXT,
        icon  TEXT
    );
    CREATE TABLE apps (
        id            INTEGER PRIMARY KEY,
        name          TEXT NOT NULL UNIQUE COLLATE NOCASE,
        launch_target TEXT NOT NULL,
        watch_exe     TEXT,
        icon          TEXT,
        category_id   INTEGER NOT NULL REFERENCES categories(id) ON DELETE RESTRICT,
        total_xp      INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE sessions (
        id         INTEGER PRIMARY KEY,
        app_id     INTEGER NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
        started_at INTEGER NOT NULL,
        ended_at   INTEGER,
        duration_s INTEGER NOT NULL DEFAULT 0,
        xp_gained  INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX sessions_app_id ON sessions(app_id);
    CREATE TABLE rewards (
        id          INTEGER PRIMARY KEY,
        app_id      INTEGER REFERENCES apps(id) ON DELETE CASCADE,
        code        TEXT NOT NULL,
        name        TEXT NOT NULL,
        description TEXT NOT NULL DEFAULT '',
        rule        TEXT NOT NULL,
        UNIQUE (app_id, code)
    );
    CREATE TABLE unlocked_rewards (
        reward_id   INTEGER PRIMARY KEY REFERENCES rewards(id) ON DELETE CASCADE,
        unlocked_at INTEGER NOT NULL,
        session_id  INTEGER REFERENCES sessions(id) ON DELETE SET NULL
    );",
];

pub struct Database {
    pub(super) conn: Connection,
}

impl Database {
    /// Opens (or creates) the user database in `%APPDATA%\CmdBoard`.
    /// A brand-new database is filled with a few starter apps.
    pub fn open_default() -> anyhow::Result<Self> {
        let dir = data_dir()?;
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("impossible de créer {}", dir.display()))?;
        Self::open(&dir.join("cmdboard.db"))
    }

    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("impossible d'ouvrir {}", path.display()))?;
        let (db, previous_version) = Self::init(conn)?;
        if previous_version == 0 {
            db.seed_defaults()?;
        }
        Ok(db)
    }

    /// Migrated but empty database, for tests.
    #[cfg(test)]
    pub fn open_in_memory() -> anyhow::Result<Self> {
        Ok(Self::init(Connection::open_in_memory()?)?.0)
    }

    fn init(mut conn: Connection) -> anyhow::Result<(Self, usize)> {
        conn.pragma_update(None, "foreign_keys", true)?;
        let previous_version = migrate(&mut conn)?;
        Ok((Self { conn }, previous_version))
    }

    /// Starter content so a fresh install has something to launch.
    pub fn seed_defaults(&self) -> anyhow::Result<()> {
        let games = self.add_category("Jeux")?;
        let dev = self.add_category("Dev")?;
        let tools = self.add_category("Outils")?;
        let apps = [
            ("Steam", "steam://open/main", Some("steam.exe"), games),
            ("Windows Terminal", "wt.exe", Some("WindowsTerminal.exe"), dev),
            ("Bloc-notes", "notepad.exe", Some("Notepad.exe"), tools),
            ("Calculatrice", "calc.exe", Some("CalculatorApp.exe"), tools),
            ("Explorateur", "explorer.exe", None, tools),
        ];
        for (name, target, watch, category_id) in apps {
            self.add_app(&NewApp {
                name: name.into(),
                launch_target: target.into(),
                watch_exe: watch.map(Into::into),
                category_id,
            })?;
        }
        Ok(())
    }
}

/// `%APPDATA%\CmdBoard`: database, `config.toml` and user themes.
pub fn data_dir() -> anyhow::Result<PathBuf> {
    let base = directories::BaseDirs::new().context("dossier %APPDATA% introuvable")?;
    Ok(base.config_dir().join("CmdBoard"))
}

/// Applies the missing migrations, each in its own transaction.
/// Returns the schema version found before migrating (0 for a new database).
fn migrate(conn: &mut Connection) -> anyhow::Result<usize> {
    let current = user_version(conn)?;
    if current > MIGRATIONS.len() {
        bail!(
            "base de données en version {current}, créée par une version plus récente de CmdBoard \
             (cette version connaît {})",
            MIGRATIONS.len()
        );
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)
            .with_context(|| format!("migration {} échouée", i + 1))?;
        set_user_version(&tx, i + 1)?;
        tx.commit()?;
    }
    Ok(current)
}

fn user_version(conn: &Connection) -> rusqlite::Result<usize> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    Ok(version.max(0) as usize)
}

fn set_user_version(conn: &Connection, version: usize) -> rusqlite::Result<()> {
    conn.pragma_update(None, "user_version", version as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_version_migrates_to_latest() {
        // Start from each historical version, built by replaying the earlier migrations.
        for start in 0..=MIGRATIONS.len() {
            let mut conn = Connection::open_in_memory().unwrap();
            for sql in &MIGRATIONS[..start] {
                conn.execute_batch(sql).unwrap();
            }
            set_user_version(&conn, start).unwrap();

            assert_eq!(migrate(&mut conn).unwrap(), start);
            assert_eq!(user_version(&conn).unwrap(), MIGRATIONS.len());
        }
    }

    #[test]
    fn migrating_twice_is_a_no_op() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        assert_eq!(migrate(&mut conn).unwrap(), MIGRATIONS.len());
    }

    #[test]
    fn newer_database_is_refused() {
        let mut conn = Connection::open_in_memory().unwrap();
        set_user_version(&conn, MIGRATIONS.len() + 1)
            .unwrap();
        assert!(migrate(&mut conn).is_err());
    }

    #[test]
    fn fresh_file_is_seeded_once() {
        let dir = std::env::temp_dir().join(format!("cmdboard-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("seed.db");
        let _ = std::fs::remove_file(&path);

        let count = |db: &Database| db.apps().unwrap().len();
        let first = count(&Database::open(&path).unwrap());
        assert!(first > 0);
        assert_eq!(count(&Database::open(&path).unwrap()), first);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
