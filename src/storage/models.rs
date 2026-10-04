#[derive(Debug, Clone)]
pub struct Category {
    pub id: i64,
    pub name: String,
}

/// An app as shown in the dashboard: the `apps` row plus stats aggregated from sessions.
#[derive(Debug, Clone)]
pub struct AppEntry {
    pub id: i64,
    pub name: String,
    /// Exe path or URI (`steam://...`) passed to the shell.
    pub launch_target: String,
    /// Process to track; may differ from `launch_target` for launchers.
    pub watch_exe: Option<String>,
    pub category_id: i64,
    pub total_xp: u32,
    pub level: u32,
    /// XP earned within the current level.
    pub xp: u32,
    pub total_secs: u64,
    /// End of the last finished session, Unix seconds.
    pub last_played: Option<i64>,
    pub rewards: u32,
}

/// Fields needed to create an app.
#[derive(Debug, Clone)]
pub struct NewApp {
    pub name: String,
    pub launch_target: String,
    pub watch_exe: Option<String>,
    pub category_id: i64,
}

/// A session closed by `close_orphan_sessions`, still to be awarded its XP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedSession {
    pub session_id: i64,
    pub app_id: i64,
    pub secs: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Profile {
    /// Sum of every app's `total_xp`.
    pub total_xp: u32,
    pub level: u32,
    pub xp: u32,
    pub streak_days: u32,
    pub xp_today: u32,
}
