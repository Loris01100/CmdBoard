pub mod app_table;
pub mod category_list;
pub mod profile_panel;
pub mod status_bar;
pub mod xp_bar;

/// "42h" above one hour, "35m" below.
pub fn format_duration(secs: u64) -> String {
    match secs / 3600 {
        0 => format!("{}m", secs / 60),
        hours => format!("{hours}h"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(format_duration(0), "0m");
        assert_eq!(format_duration(35 * 60), "35m");
        assert_eq!(format_duration(42 * 3600 + 59 * 60), "42h");
    }
}
