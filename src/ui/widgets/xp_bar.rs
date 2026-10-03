use ratatui::{style::Style, widgets::Gauge};

use crate::ui::theme::Theme;

/// Text bar for table cells, e.g. "█████░░░".
pub fn text(ratio: f64, width: usize) -> String {
    let filled = (ratio.clamp(0.0, 1.0) * width as f64).round() as usize;
    "█".repeat(filled) + &"░".repeat(width - filled)
}

/// Full-size gauge with a percentage label.
pub fn gauge(ratio: f64, theme: &Theme) -> Gauge<'static> {
    let ratio = ratio.clamp(0.0, 1.0);
    Gauge::default()
        .gauge_style(Style::new().fg(theme.xp_fill))
        .use_unicode(true)
        .ratio(ratio)
        .label(format!("{:.0}%", ratio * 100.0))
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
