/// XP needed to go from `level` to `level + 1`.
pub fn xp_to_next_level(level: u32) -> u32 {
    (100.0 * (level.max(1) as f32).powf(1.5)) as u32
}

/// Progress through the current level, between 0.0 and 1.0.
pub fn level_progress(level: u32, xp: u32) -> f64 {
    (xp as f64 / xp_to_next_level(level) as f64).clamp(0.0, 1.0)
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
}
