use ratatui::{style::Style, widgets::Gauge};

use crate::core::xp;
use crate::ui::theme::Theme;

/// Text bar for table cells, e.g. "█████░░░".
pub fn text(ratio: f64, width: usize) -> String {
    let filled = (ratio.clamp(0.0, 1.0) * width as f64).round() as usize;
    "█".repeat(filled) + &"░".repeat(width - filled)
}

/// Full-size gauge for `xp` within `level`, labelled "xp/needed".
pub fn gauge(level: u32, xp: u32, theme: &Theme) -> Gauge<'static> {
    Gauge::default()
        .gauge_style(Style::new().fg(theme.xp_fill))
        .use_unicode(true)
        .ratio(xp::level_progress(level, xp))
        .label(format!("{xp}/{} XP", xp::xp_to_next_level(level)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_bar_fills_proportionally() {
        assert_eq!(text(0.0, 4), "░░░░");
        assert_eq!(text(0.5, 4), "██░░");
        assert_eq!(text(2.0, 4), "████");
    }
}
