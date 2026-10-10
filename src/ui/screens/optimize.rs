use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table},
};

use crate::app::App;
use crate::optimize::{self, Bench, Score, Tier};
use crate::ui::{
    layout,
    widgets::{command_line, format_size, status_bar},
};

/// The PC in two lines, the benchmarks and the Windows gaming settings.
pub fn draw(frame: &mut Frame, app: &App) {
    let (body, command, status) = layout::screen(frame.area(), command_line::height(app));
    // On short terminals the tables keep the room and the PC summary goes.
    let system_height = if body.height >= 14 && app.optimize.system.is_some() {
        4
    } else {
        0
    };
    let [system_area, tables] =
        Layout::vertical([Constraint::Length(system_height), Constraint::Min(3)]).areas(body);
    let [bench_area, gaming_area] = if tables.width >= 90 {
        Layout::horizontal([Constraint::Fill(3), Constraint::Fill(2)]).areas(tables)
    } else {
        Layout::vertical([Constraint::Fill(1), Constraint::Length(5)]).areas(tables)
    };
    if system_height > 0 {
        draw_system(frame, system_area, app);
    }
    draw_benches(frame, bench_area, app);
    draw_gaming(frame, gaming_area, app);
    command_line::render(frame, command, app);
    status_bar::render(frame, status, app);
}

fn draw_system(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let Some(system) = &app.optimize.system else {
        return;
    };
    let line = |label: String, value: String| {
        Line::from(vec![
            Span::styled(format!(" {label:<12}"), theme.title),
            Span::raw(value),
        ])
    };
    let lines = vec![
        line(
            t!("optimize.cpu"),
            t!(
                "optimize.cpu_value",
                name = system.cpu.as_str(),
                cores = system.cores,
                threads = system.threads
            ),
        ),
        line(t!("optimize.ram"), format_size(system.ram)),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(theme.panel(&t!("optimize.system"), false)),
        area,
    );
}

/// Cells of a result's gauge.
const GAUGE: usize = 10;
/// The result column: the gauge, a space and the verdict.
const RESULT_WIDTH: u16 = 24;
const LEVEL_WIDTH: u16 = 7;

fn draw_benches(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let level = |heavy: bool| {
        if heavy {
            t!("optimize.heavy")
        } else {
            t!("optimize.light")
        }
    };
    let title = t!("optimize.benches", level = level(app.optimize.heavy));
    // Test, verdict, raw measure and level of each benchmark.
    let cells: Vec<_> = Bench::ALL
        .iter()
        .map(|&bench| {
            let (result, measure, heavy) =
                match (app.optimize.running, app.optimize.results.get(&bench)) {
                    (Some(running), _) if running == bench => (
                        Line::styled(t!("optimize.running"), Style::new().fg(theme.warning)),
                        String::new(),
                        String::new(),
                    ),
                    (_, Some((heavy, Ok(score)))) => (
                        verdict(app, bench, *score),
                        format_score(*score),
                        level(*heavy),
                    ),
                    (_, Some((heavy, Err(e)))) => (
                        Line::styled(e.clone(), Style::new().fg(theme.error)),
                        String::new(),
                        level(*heavy),
                    ),
                    (_, None) => (
                        Line::styled("—", theme.muted()),
                        String::new(),
                        String::new(),
                    ),
                };
            (bench.label(), result, measure, heavy)
        })
        .collect();
    // The raw measure is the first to go when room runs out.
    let width = |text: &str| u16::try_from(Line::raw(text).width()).unwrap_or(u16::MAX);
    let label_width = cells.iter().map(|c| width(&c.0)).max().unwrap_or(0);
    let measure_width = cells.iter().map(|c| width(&c.2)).max().unwrap_or(0);
    // Borders, highlight symbol and the gaps between the four columns.
    let needed = label_width + RESULT_WIDTH + measure_width + LEVEL_WIDTH + 7;
    let show_measure = measure_width > 0 && needed <= area.width;

    let mut header = vec![t!("optimize.test"), t!("optimize.result")];
    let mut widths = vec![Constraint::Fill(1), Constraint::Length(RESULT_WIDTH)];
    if show_measure {
        header.push(t!("optimize.measure"));
        widths.push(Constraint::Length(measure_width));
    }
    header.push(t!("optimize.level"));
    widths.push(Constraint::Length(LEVEL_WIDTH));
    let rows = cells.into_iter().map(|(label, result, measure, heavy)| {
        let mut row = vec![Cell::from(label), Cell::from(result)];
        if show_measure {
            row.push(Cell::from(
                Line::styled(measure, theme.muted()).alignment(Alignment::Right),
            ));
        }
        row.push(Cell::from(Span::styled(heavy, theme.muted())));
        Row::new(row)
    });
    let focused = !app.optimize.gaming_focus;
    let table = Table::new(rows, widths)
        .header(Row::new(header).style(theme.title))
        .block(theme.panel(&title, focused))
        .row_highlight_style(theme.highlight(focused))
        .highlight_symbol("> ");
    let mut state = app.optimize.bench_state;
    frame.render_stateful_widget(table, area, &mut state);
}

/// A gauge and a word, colored from slow to very fast.
fn verdict(app: &App, bench: Bench, score: Score) -> Line<'static> {
    let theme = &app.theme;
    let rating = optimize::rate(bench, score);
    let color = match rating.tier {
        Tier::Slow => theme.error,
        Tier::Fair => theme.warning,
        Tier::Fast | Tier::VeryFast => theme.success,
    };
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "the gauge is between 0 and 1, GAUGE is small"
    )]
    let filled = ((rating.gauge * GAUGE as f64).round() as usize).clamp(1, GAUGE);
    Line::from(vec![
        Span::styled("█".repeat(filled), Style::new().fg(color)),
        Span::styled("░".repeat(GAUGE - filled), theme.muted()),
        Span::styled(
            format!(" {}", rating.tier.label(bench)),
            Style::new().fg(color),
        ),
    ])
}

fn draw_gaming(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let rows = app.optimize.gaming.iter().map(|&(setting, on)| {
        let state = if on {
            Span::styled(t!("optimize.on"), Style::new().fg(theme.success))
        } else {
            Span::styled(t!("optimize.off"), theme.muted())
        };
        Row::new([Cell::from(setting.label()), Cell::from(state)])
    });
    let focused = app.optimize.gaming_focus;
    let table = Table::new(rows, [Constraint::Fill(1), Constraint::Length(12)])
        .block(theme.panel(&t!("optimize.gaming"), focused))
        .row_highlight_style(theme.highlight(focused))
        .highlight_symbol("> ");
    let mut state = app.optimize.gaming_state;
    frame.render_stateful_widget(table, area, &mut state);
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "rates are positive and far below u64::MAX"
)]
pub fn format_score(score: Score) -> String {
    let rate = |bytes: f64| t!("optimize.per_sec", size = format_size(bytes as u64));
    match score {
        Score::Ops(millions) => t!("optimize.ops", n = format!("{millions:.0}")),
        Score::Bytes(bytes) => rate(bytes),
        Score::Disk { write, read } => {
            t!(
                "optimize.disk_score",
                write = rate(write),
                read = rate(read)
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::{App, Screen};
    use crate::optimize::{Bench, Score};
    use ratatui::{Terminal, backend::TestBackend};

    fn screen(app: &App, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| crate::ui::draw(f, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    #[test]
    fn shows_system_results_and_gaming() {
        let mut app = App::with_defaults();
        app.screen = Screen::Optimize;
        app.optimize.system = Some(crate::optimize::System {
            cpu: "Ryzen 7".into(),
            cores: 8,
            threads: 16,
            ram: 32 << 30,
        });
        app.optimize.gaming = vec![(crate::optimize::Gaming::GameMode, true)];
        app.optimize
            .on_finished(Bench::CpuSingle, false, Ok(Score::Ops(1234.4)));
        app.optimize
            .on_finished(Bench::Memory, true, Err("mémoire insuffisante".into()));
        app.optimize.running = Some(Bench::Disk);
        let text = screen(&app, 140, 30);
        assert!(text.contains("Ryzen 7 · 8 cœurs / 16 threads"), "{text}");
        assert!(text.contains("32 Go"), "{text}");
        assert!(text.contains("█████████░ Très rapide"), "{text}");
        assert!(text.contains("Mesure") && text.contains("1234 M op/s"));

        // Without room for the raw measure, the verdict stays.
        let text = screen(&app, 120, 30);
        assert!(text.contains("Très rapide") && !text.contains("op/s"));
        app.optimize.results.clear();
        assert!(!screen(&app, 140, 30).contains("Mesure"));
        app.optimize
            .on_finished(Bench::CpuSingle, false, Ok(Score::Ops(1234.4)));
        app.optimize
            .on_finished(Bench::Memory, true, Err("mémoire insuffisante".into()));
        app.optimize.running = Some(Bench::Disk);
        let text = screen(&app, 140, 30);
        assert!(text.contains("mémoire insuffisante") && text.contains("lourd"));
        assert!(text.contains("en cours…"), "{text}");
        assert!(
            text.contains("Mode Jeu") && text.contains("activé"),
            "{text}"
        );

        // Narrow and short: still renders, the tables first.
        let text = screen(&app, 50, 12);
        assert!(
            !text.contains("Ryzen") && text.contains("Mode Jeu"),
            "{text}"
        );
    }

    #[test]
    fn disk_score_shows_both_rates() {
        let score = Score::Disk {
            write: f64::from(500 << 20),
            read: (2u64 << 30) as f64,
        };
        assert_eq!(
            super::format_score(score),
            "écriture 500 Mo/s · lecture 2,0 Go/s"
        );
    }
}
