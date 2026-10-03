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

/// How long ago something happened, given the elapsed seconds.
pub fn format_ago(secs: i64) -> String {
    match secs.max(0) {
        s if s < 60 => "à l'instant".into(),
        s if s < 3600 => format!("il y a {} min", s / 60),
        s if s < 86_400 => format!("il y a {} h", s / 3600),
        s if s < 2 * 86_400 => "hier".into(),
        s => format!("il y a {} j", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ago() {
        assert_eq!(format_ago(-5), "à l'instant");
        assert_eq!(format_ago(150), "il y a 2 min");
        assert_eq!(format_ago(3 * 3600 + 10), "il y a 3 h");
        assert_eq!(format_ago(30 * 3600), "hier");
        assert_eq!(format_ago(5 * 86_400), "il y a 5 j");
    }

    #[test]
    fn durations() {
        assert_eq!(format_duration(0), "0m");
        assert_eq!(format_duration(35 * 60), "35m");
        assert_eq!(format_duration(42 * 3600 + 59 * 60), "42h");
    }
}
