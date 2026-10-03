#[derive(Debug, Clone)]
pub struct Category {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct AppEntry {
    pub name: String,
    pub category_id: i64,
    pub level: u32,
    /// XP earned within the current level.
    pub xp: u32,
    pub total_secs: u64,
    // Replaced by a real timestamp once sessions are stored (step 4).
    pub last_played: Option<String>,
    pub rewards: u32,
}

#[derive(Debug, Clone)]
pub struct Profile {
    pub level: u32,
    pub xp: u32,
    pub streak_days: u32,
    pub xp_today: u32,
}
