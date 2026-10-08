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

/// A reward that can still be unlocked for the app being checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reward {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub description: String,
    pub rule: String,
    /// Unlocked once per app rather than once overall.
    pub per_app: bool,
}

/// A reward as listed on the Rewards screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewardView {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub rule: String,
    pub per_app: bool,
    /// The only app this reward applies to, if any.
    pub app: Option<String>,
    /// Oldest first. Empty while locked.
    pub unlocks: Vec<Unlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unlock {
    /// App it was unlocked for, `None` for a global reward.
    pub app: Option<String>,
    /// Local date, "dd/mm/yyyy".
    pub date: String,
}

/// Days shown by the activity heatmap of the Stats screen: 12 weeks.
pub const ACTIVITY_DAYS: usize = 84;

/// Everything the Stats screen shows, for all apps or one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    /// Most recent first, capped.
    pub sessions: Vec<SessionRow>,
    pub session_count: u32,
    pub total_secs: u64,
    pub longest_secs: u64,
    /// `(category, seconds)`, most played first.
    pub by_category: Vec<(String, u64)>,
    /// `(app, seconds)`, most played first.
    pub by_app: Vec<(String, u64)>,
    /// Seconds played per day over the last `ACTIVITY_DAYS` days, oldest first, today last.
    pub daily: Vec<u64>,
    /// Today, in local days since 1970-01-01 (a Thursday).
    pub today: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub app: String,
    /// Local start time, "dd/mm/yyyy hh:mm".
    pub started: String,
    pub duration_secs: u64,
    pub xp: u32,
}

/// An event of the dashboard activity ticker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Activity {
    Session {
        app: String,
        secs: u64,
        xp: u32,
        /// When it ended.
        at: i64,
    },
    /// "Name (App)", or "Name" for a global reward.
    Reward { name: String, at: i64 },
}

/// A session closed by `close_orphan_sessions`, still to be awarded its XP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedSession {
    pub session_id: i64,
    pub secs: u64,
}

/// What a closed session earned.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionOutcome {
    pub xp: u32,
    pub rewards: Vec<RewardUnlocked>,
    /// Rewards whose rule could not be evaluated.
    pub rule_errors: Vec<RuleError>,
}

/// A reward a session just unlocked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewardUnlocked {
    pub name: String,
    pub description: String,
    /// App it was unlocked for, `None` for a global reward.
    pub app: Option<String>,
}

/// A reward rule that could not be evaluated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleError {
    pub code: String,
    pub error: String,
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
