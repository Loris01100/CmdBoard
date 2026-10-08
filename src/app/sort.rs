//! Order of the apps panel, chosen with `:sort` or `s`.

use crate::storage::models::AppEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppSort {
    #[default]
    Name,
    Xp,
    Recent,
    Time,
}

impl AppSort {
    pub const ALL: [AppSort; 4] = [AppSort::Name, AppSort::Xp, AppSort::Recent, AppSort::Time];

    pub fn name(self) -> &'static str {
        match self {
            AppSort::Name => "name",
            AppSort::Xp => "xp",
            AppSort::Recent => "recent",
            AppSort::Time => "time",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        let name = name.to_lowercase();
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    pub fn label(self) -> String {
        match self {
            AppSort::Name => t!("sort.name"),
            AppSort::Xp => t!("sort.xp"),
            AppSort::Recent => t!("sort.recent"),
            AppSort::Time => t!("sort.time"),
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&s| s == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// Name ascending; the others biggest or latest first, ties by name.
    pub fn apply(self, apps: &mut [AppEntry]) {
        apps.sort_by_cached_key(|a| a.name.to_lowercase());
        match self {
            AppSort::Name => {}
            AppSort::Xp => apps.sort_by_key(|a| std::cmp::Reverse(a.total_xp)),
            AppSort::Recent => apps.sort_by_key(|a| std::cmp::Reverse(a.last_played)),
            AppSort::Time => apps.sort_by_key(|a| std::cmp::Reverse(a.total_secs)),
        }
    }
}
