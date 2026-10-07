pub mod app_table;
pub mod category_list;
pub mod command_line;
pub mod popup;
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

/// "640 KB", "1.5 GB", "953 GB": one decimal below 10, binary units, localized.
pub fn format_size(bytes: u64) -> String {
    let units = ["size.kb", "size.mb", "size.gb", "size.tb"];
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let n = if value < 10.0 {
        format!("{value:.1}").replace('.', &t!("size.decimal"))
    } else {
        format!("{value:.0}")
    };
    crate::i18n::tr(units[unit], &[("n", &n)])
}

/// Running timer: "4:05" below one hour, "1:02:03" above.
pub fn format_clock(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// How long ago something happened, given the elapsed seconds.
pub fn format_ago(secs: i64) -> String {
    match secs.max(0) {
        s if s < 60 => t!("ago.now"),
        s if s < 3600 => t!("ago.minutes", count = s / 60),
        s if s < 86_400 => t!("ago.hours", count = s / 3600),
        s if s < 2 * 86_400 => t!("ago.yesterday"),
        s => t!("ago.days", count = s / 86_400),
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
    fn sizes() {
        assert_eq!(format_size(0), "0,0 Ko");
        assert_eq!(format_size(640 << 10), "640 Ko");
        assert_eq!(format_size(1023 << 10), "1023 Ko");
        assert_eq!(format_size(1 << 20), "1,0 Mo");
        assert_eq!(format_size(1536 << 20), "1,5 Go");
        assert_eq!(format_size(953 << 30), "953 Go");
        assert_eq!(format_size(2048 << 30), "2,0 To");
    }

    #[test]
    fn clock() {
        assert_eq!(format_clock(0), "0:00");
        assert_eq!(format_clock(245), "4:05");
        assert_eq!(format_clock(3723), "1:02:03");
    }

    #[test]
    fn durations() {
        assert_eq!(format_duration(0), "0m");
        assert_eq!(format_duration(35 * 60), "35m");
        assert_eq!(format_duration(42 * 3600 + 59 * 60), "42h");
    }
}
