//! Goals and limits on play time (plan section 11): play at least, or at most, so many
//! minutes per day or per week, on an app, a category or every app.

/// A goal is reached by playing enough; a limit, by playing too much.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GoalKind {
    Goal,
    Limit,
}

impl GoalKind {
    /// Stored in `goals.kind`, and the command name.
    pub fn code(self) -> &'static str {
        match self {
            GoalKind::Goal => "goal",
            GoalKind::Limit => "limit",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        [GoalKind::Goal, GoalKind::Limit]
            .into_iter()
            .find(|k| k.code() == code)
    }
}

/// Days follow the local time zone; weeks start on Monday.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Period {
    Day,
    Week,
}

impl Period {
    /// Stored in `goals.period`.
    pub fn code(self) -> &'static str {
        match self {
            Period::Day => "day",
            Period::Week => "week",
        }
    }

    /// `day`/`d` or `week`/`w`, ignoring case.
    pub fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "day" | "d" => Some(Period::Day),
            "week" | "w" => Some(Period::Week),
            _ => None,
        }
    }

    pub fn minutes(self) -> u32 {
        match self {
            Period::Day => 24 * 60,
            Period::Week => 7 * 24 * 60,
        }
    }
}

/// How far the time played is from a goal's or limit's target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Under,
    /// 80 % of the target or more: a limit is close.
    Near,
    /// The target or more.
    Reached,
}

pub fn status(secs: u64, target_secs: u64) -> Status {
    if secs >= target_secs {
        Status::Reached
    } else if secs * 5 >= target_secs * 4 {
        Status::Near
    } else {
        Status::Under
    }
}

/// `1h52`, `45m`, `2h`: the syntax `parse_amount` reads, to the minute below.
pub fn format_hm(secs: u64) -> String {
    let (hours, minutes) = (secs / 3600, secs / 60 % 60);
    match (hours, minutes) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h{m:02}"),
    }
}

/// `2h/day`, `1h30/week`, `90m/d`: minutes and period. `None` when malformed, zero, or
/// longer than the period itself.
pub fn parse_amount(text: &str) -> Option<(u32, Period)> {
    let (duration, period) = text.split_once('/')?;
    let period = Period::parse(period)?;
    let minutes = parse_minutes(duration)?;
    (minutes > 0 && minutes <= period.minutes()).then_some((minutes, period))
}

/// `2h`, `90m`, `1h30`, `1h30m`, or plain minutes (`45`).
fn parse_minutes(text: &str) -> Option<u32> {
    let text = text.to_ascii_lowercase();
    let number = |s: &str| -> Option<u32> {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    match text.split_once('h') {
        Some((hours, rest)) => {
            let minutes = rest.strip_suffix('m').unwrap_or(rest);
            let minutes = if minutes.is_empty() {
                0
            } else {
                number(minutes).filter(|m| *m < 60)?
            };
            number(hours)?.checked_mul(60)?.checked_add(minutes)
        }
        None => number(text.strip_suffix('m').unwrap_or(&text)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_amounts() {
        let cases = [
            ("2h/day", Some((120, Period::Day))),
            ("1h30/week", Some((90, Period::Week))),
            ("1H30M/W", Some((90, Period::Week))),
            ("90m/d", Some((90, Period::Day))),
            ("45/day", Some((45, Period::Day))),
            ("24h/day", Some((1440, Period::Day))),
            ("25h/day", None), // longer than a day
            ("25h/week", Some((1500, Period::Week))),
            ("0m/day", None),
            ("1h60/day", None),
            ("2h", None),
            ("2h/month", None),
            ("h/day", None),
            ("-5/day", None),
            ("2x/day", None),
            ("99999999999h/week", None),
        ];
        for (text, expected) in cases {
            assert_eq!(parse_amount(text), expected, "{text}");
        }
    }

    #[test]
    fn formats_hours_and_minutes() {
        assert_eq!(format_hm(0), "0m");
        assert_eq!(format_hm(59), "0m");
        assert_eq!(format_hm(45 * 60), "45m");
        assert_eq!(format_hm(2 * 3600), "2h");
        assert_eq!(format_hm(3600 + 5 * 60 + 30), "1h05");
        assert_eq!(parse_minutes(&format_hm(3600 + 5 * 60)), Some(65));
    }

    #[test]
    fn status_thresholds() {
        assert_eq!(status(0, 100), Status::Under);
        assert_eq!(status(79, 100), Status::Under);
        assert_eq!(status(80, 100), Status::Near);
        assert_eq!(status(99, 100), Status::Near);
        assert_eq!(status(100, 100), Status::Reached);
        assert_eq!(status(500, 100), Status::Reached);
    }

    #[test]
    fn codes_round_trip() {
        for kind in [GoalKind::Goal, GoalKind::Limit] {
            assert_eq!(GoalKind::parse(kind.code()), Some(kind));
        }
        for period in [Period::Day, Period::Week] {
            assert_eq!(Period::parse(period.code()), Some(period));
        }
    }

    proptest::proptest! {
        #[test]
        fn parsing_never_panics_and_stays_within_the_period(text in "\\PC{0,16}") {
            if let Some((minutes, period)) = parse_amount(&text) {
                proptest::prop_assert!(minutes > 0 && minutes <= period.minutes());
            }
        }
    }
}
