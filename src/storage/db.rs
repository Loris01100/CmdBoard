use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use rusqlite::Connection;

use super::models::NewApp;
use crate::launcher::launch;

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
    // v2: rewards unlocked once per app (`scope = 'app'`), plus the starter rewards.
    // A reward tied to one app (`app_id` set) is per-app too.
    "ALTER TABLE rewards ADD COLUMN scope TEXT NOT NULL DEFAULT 'global';
    CREATE TABLE unlocked_rewards_v2 (
        id          INTEGER PRIMARY KEY,
        reward_id   INTEGER NOT NULL REFERENCES rewards(id) ON DELETE CASCADE,
        app_id      INTEGER REFERENCES apps(id) ON DELETE CASCADE,
        unlocked_at INTEGER NOT NULL,
        session_id  INTEGER REFERENCES sessions(id) ON DELETE SET NULL
    );
    INSERT INTO unlocked_rewards_v2 (reward_id, app_id, unlocked_at, session_id)
        SELECT u.reward_id, r.app_id, u.unlocked_at, u.session_id
        FROM unlocked_rewards u JOIN rewards r ON r.id = u.reward_id;
    DROP TABLE unlocked_rewards;
    ALTER TABLE unlocked_rewards_v2 RENAME TO unlocked_rewards;
    CREATE UNIQUE INDEX unlocked_rewards_once ON unlocked_rewards(reward_id, IFNULL(app_id, 0));
    INSERT INTO rewards (code, name, description, rule, scope) VALUES
        ('premiers_pas', 'Premiers pas', 'Terminer une première session', 'total_sessions >= 1', 'global'),
        ('marathon', 'Marathon', 'Jouer 3 h d''affilée', 'session_minutes >= 180', 'app'),
        ('noctambule', 'Noctambule', 'Commencer une session entre minuit et 5 h', 'session_hour < 5', 'global'),
        ('habitue', 'Habitué', 'Cumuler 10 h sur une app', 'app_hours >= 10', 'app'),
        ('passionne', 'Passionné', 'Cumuler 50 h sur une app', 'app_hours >= 50', 'app'),
        ('veteran', 'Vétéran', 'Atteindre le niveau 5 sur une app', 'app_level >= 5', 'app'),
        ('regulier', 'Régulier', 'Jouer 3 jours de suite', 'streak_days >= 3', 'global'),
        ('assidu', 'Assidu', 'Jouer 7 jours de suite', 'streak_days >= 7', 'global'),
        ('touche_a_tout', 'Touche-à-tout', 'Lancer 5 apps différentes en 7 jours', 'apps_this_week >= 5', 'global'),
        ('centurion', 'Centurion', 'Cumuler 100 h au total', 'total_hours >= 100', 'global'),
        ('expert', 'Expert', 'Atteindre le niveau global 10', 'level >= 10', 'global');",
    // v3: long streak rewards (7 days is `assidu`, from v2).
    "INSERT INTO rewards (code, name, description, rule, scope) VALUES
        ('inarretable', 'Inarrêtable', 'Jouer 30 jours de suite', 'streak_days >= 30', 'global'),
        ('legende', 'Légende', 'Jouer 365 jours de suite', 'streak_days >= 365', 'global');",
    // v4: `:clear sessions` hides sessions from the history; stats still count them.
    "ALTER TABLE sessions ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;",
    // v5: when an open session was last saved, so the next CmdBoard process (the UI or
    // the background tracker) resumes it if it was handed over moments ago.
    "ALTER TABLE sessions ADD COLUMN checkpoint_at INTEGER;",
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
            .with_context(|| t!("error.cannot_create", path = dir.display()))?;
        Self::open(&dir.join("cmdboard.db"))
    }

    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| t!("error.cannot_open", path = path.display()))?;
        let (db, previous_version) = Self::init(conn)?;
        if previous_version == 0 {
            db.seed_defaults(launch::is_available)?;
        }
        Ok(db)
    }

    /// Migrated but empty database, for tests.
    #[cfg(test)]
    pub fn open_in_memory() -> anyhow::Result<Self> {
        Ok(Self::init(Connection::open_in_memory()?)?.0)
    }

    /// Raw SQL, for tests that need data the API does not create (e.g. a broken rule).
    #[cfg(test)]
    pub fn execute_for_tests(&self, sql: &str) {
        self.conn.execute_batch(sql).unwrap();
    }

    fn init(mut conn: Connection) -> anyhow::Result<(Self, usize)> {
        conn.pragma_update(None, "foreign_keys", true)?;
        let previous_version = migrate(&mut conn)?;
        Ok((Self { conn }, previous_version))
    }

    /// Starter content so a fresh install has something to launch. Only apps whose
    /// target passes `available` are added; their categories are created regardless.
    pub fn seed_defaults(&self, available: impl Fn(&str) -> bool) -> anyhow::Result<()> {
        let games = self.add_category(&t!("seed.games"))?;
        let dev = self.add_category("Dev")?;
        let tools = self.add_category(&t!("seed.tools"))?;
        let apps = [
            (
                "Steam".into(),
                "steam://open/main",
                Some("steam.exe"),
                games,
            ),
            (
                "Windows Terminal".into(),
                "wt.exe",
                Some("WindowsTerminal.exe"),
                dev,
            ),
            (
                t!("seed.notepad"),
                "notepad.exe",
                Some("Notepad.exe"),
                tools,
            ),
            (
                t!("seed.calculator"),
                "calc.exe",
                Some("CalculatorApp.exe"),
                tools,
            ),
            (t!("seed.explorer"), "explorer.exe", None, tools),
        ];
        for (name, target, watch, category_id) in apps {
            if !available(target) {
                continue;
            }
            self.add_app(&NewApp {
                name,
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
    let base = directories::BaseDirs::new().with_context(|| t!("error.no_appdata"))?;
    Ok(base.config_dir().join("CmdBoard"))
}

/// Applies the missing migrations, each in its own transaction.
/// Returns the schema version found before migrating (0 for a new database).
fn migrate(conn: &mut Connection) -> anyhow::Result<usize> {
    let current = user_version(conn)?;
    if current > MIGRATIONS.len() {
        bail!(t!(
            "error.db_too_new",
            version = current,
            known = MIGRATIONS.len()
        ));
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)
            .with_context(|| t!("error.migration_failed", number = i + 1))?;
        set_user_version(&tx, i + 1)?;
        tx.commit()?;
    }
    Ok(current)
}

fn user_version(conn: &Connection) -> rusqlite::Result<usize> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    Ok(usize::try_from(version).unwrap_or(0))
}

fn set_user_version(conn: &Connection, version: usize) -> rusqlite::Result<()> {
    conn.pragma_update(
        None,
        "user_version",
        i64::try_from(version).unwrap_or(i64::MAX),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_is_under_appdata() {
        assert!(data_dir().unwrap().ends_with("CmdBoard"));
    }

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
    fn v1_unlocked_rewards_survive_v2() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.execute_batch(
            "INSERT INTO categories (name) VALUES ('Jeux');
             INSERT INTO apps (name, launch_target, category_id) VALUES ('Hades', 'x', 1);
             INSERT INTO rewards (app_id, code, name, rule) VALUES (1, 'old', 'Ancienne', 'level >= 1');
             INSERT INTO unlocked_rewards (reward_id, unlocked_at) VALUES (1, 1000);",
        )
        .unwrap();
        set_user_version(&conn, 1).unwrap();

        migrate(&mut conn).unwrap();
        let (app_id, at): (i64, i64) = conn
            .query_row(
                "SELECT app_id, unlocked_at FROM unlocked_rewards",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((app_id, at), (1, 1000));
        let rewards: i64 = conn
            .query_row("SELECT COUNT(*) FROM rewards", [], |r| r.get(0))
            .unwrap();
        assert!(rewards > 1); // starter rewards added
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
        set_user_version(&conn, MIGRATIONS.len() + 1).unwrap();
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
