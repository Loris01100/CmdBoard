/// XP needed to go from `level` to `level + 1`.
pub fn xp_to_next_level(level: u32) -> u32 {
    (100.0 * (level.max(1) as f32).powf(1.5)) as u32
}

/// Progress through the current level, between 0.0 and 1.0.
pub fn level_progress(level: u32, xp: u32) -> f64 {
    (xp as f64 / xp_to_next_level(level) as f64).clamp(0.0, 1.0)
}

/// Splits accumulated XP into `(level, xp within that level)`. Levels start at 1.
pub fn level_from_total(total_xp: u32) -> (u32, u32) {
    let (mut level, mut rest) = (1, total_xp);
    while rest >= xp_to_next_level(level) {
        rest -= xp_to_next_level(level);
        level += 1;
    }
    (level, rest)
}

/// Consecutive active days ending today or yesterday.
/// `days` are day numbers (e.g. Julian days), sorted most recent first, without duplicates.
pub fn streak_days(days: &[i64], today: i64) -> u32 {
    let Some(&first) = days.first() else { return 0 };
    if today - first > 1 {
        return 0;
    }
    let mut streak = 1;
    for pair in days.windows(2) {
        if pair[0] - pair[1] != 1 {
            break;
        }
        streak += 1;
    }
    streak
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_level_cost_grows() {
        assert_eq!(xp_to_next_level(0), 100);
        assert_eq!(xp_to_next_level(1), 100);
        assert_eq!(xp_to_next_level(4), 800);
    }

    #[test]
    fn progress_is_clamped() {
        assert_eq!(level_progress(1, 50), 0.5);
        assert_eq!(level_progress(1, 500), 1.0);
    }

    #[test]
    fn total_xp_splits_into_levels() {
        assert_eq!(level_from_total(0), (1, 0));
        assert_eq!(level_from_total(99), (1, 99));
        assert_eq!(level_from_total(100), (2, 0));
        // 100 (lvl 1) + 282 (lvl 2) = 382
        assert_eq!(level_from_total(400), (3, 18));
    }

    #[test]
    fn streak_counts_consecutive_days() {
        assert_eq!(streak_days(&[], 10), 0);
        assert_eq!(streak_days(&[10, 9, 8, 6], 10), 3);
        assert_eq!(streak_days(&[9, 8], 10), 2); // nothing yet today: streak still alive
        assert_eq!(streak_days(&[8, 7], 10), 0);
    }
}
