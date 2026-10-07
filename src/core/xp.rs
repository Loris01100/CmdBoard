/// Sessions shorter than this earn no XP (anti-abuse).
pub const MIN_XP_MINUTES: u32 = 5;

/// XP earned by a session: 1 XP per minute, plus 5 per streak day (capped at 7 days).
/// `streak_days` counts consecutive active days, today included.
pub fn xp_for_session(duration_min: u32, streak_days: u32) -> u32 {
    if duration_min < MIN_XP_MINUTES {
        return 0;
    }
    duration_min + streak_days.min(7) * 5
}

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

/// Value of an animated counter going from `from` to `to` over `frames` ticks,
/// `elapsed` ticks after it started. Eases out: fast at first, slowing at the end.
pub fn animate(from: u32, to: u32, elapsed: u64, frames: u64) -> u32 {
    if elapsed >= frames || frames == 0 {
        return to;
    }
    let t = elapsed as f64 / frames as f64;
    let eased = 1.0 - (1.0 - t).powi(3);
    (from as f64 + (to as f64 - from as f64) * eased).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_xp_rewards_time_and_streak() {
        assert_eq!(xp_for_session(4, 10), 0); // too short
        assert_eq!(xp_for_session(5, 0), 5);
        assert_eq!(xp_for_session(42, 1), 47);
        assert_eq!(xp_for_session(60, 30), 95); // streak bonus capped at 7 days
    }

    #[test]
    fn animation_eases_to_target() {
        assert_eq!(animate(0, 100, 0, 8), 0);
        assert!(animate(0, 100, 4, 8) > 50); // ease-out: past halfway at mid-time
        assert_eq!(animate(0, 100, 8, 8), 100);
        assert_eq!(animate(0, 100, 99, 8), 100);
        assert_eq!(animate(100, 40, 8, 8), 40);
    }

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
