use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, Wrap},
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

fn draw_benches(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let mode = if app.optimize.heavy {
        t!("optimize.heavy")
    } else {
        t!("optimize.light")
    };
    let title = t!("optimize.benches", mode = mode);
    // Test, verdict and raw measure of each benchmark; the test notes a result
    // measured in the other mode.
    let cells: Vec<_> = Bench::ALL
        .iter()
        .map(|&bench| {
            let result = app.optimize.results.get(&bench);
            let mut label = vec![Span::raw(bench.label())];
            if let Some((heavy, _)) = result
                && *heavy != app.optimize.heavy
            {
                let other = if *heavy {
                    t!("optimize.heavy_marker")
                } else {
                    t!("optimize.light_marker")
                };
                label.push(Span::styled(format!(" · {other}"), theme.muted()));
            }
            let (result, measure) = match (app.optimize.running, result) {
                (Some(running), _) if running == bench => (
                    Line::styled(t!("optimize.running"), Style::new().fg(theme.warning)),
                    String::new(),
                ),
                _ if app.optimize.queue.contains(&bench) => (
                    Line::styled(t!("optimize.queued"), theme.muted()),
                    String::new(),
                ),
                (_, Some((_, Ok(score)))) => (verdict(app, bench, *score), format_score(*score)),
                (_, Some((_, Err(e)))) => (
                    Line::styled(e.clone(), Style::new().fg(theme.error)),
                    String::new(),
                ),
                (_, None) => (Line::styled("—", theme.muted()), String::new()),
            };
            (Line::from(label), result, measure)
        })
        .collect();
    // The raw measure is the first to go when room runs out.
    let width = |line: &Line| u16::try_from(line.width()).unwrap_or(u16::MAX);
    let label_width = cells.iter().map(|c| width(&c.0)).max().unwrap_or(0);
    let measure_width = cells
        .iter()
        .map(|c| width(&Line::raw(c.2.as_str())))
        .max()
        .unwrap_or(0);
    // Borders, highlight symbol and the gaps between the three columns.
    let needed = label_width + RESULT_WIDTH + measure_width + 6;
    let show_measure = measure_width > 0 && needed <= area.width;

    let mut header = vec![t!("optimize.test"), t!("optimize.result")];
    let mut widths = vec![Constraint::Fill(1), Constraint::Length(RESULT_WIDTH)];
    if show_measure {
        header.push(t!("optimize.measure"));
        widths.push(Constraint::Length(measure_width));
    }
    let rows = cells.into_iter().map(|(label, result, measure)| {
        let mut row = vec![Cell::from(label), Cell::from(result)];
        if show_measure {
            row.push(Cell::from(
                Line::styled(measure, theme.muted()).alignment(Alignment::Right),
            ));
        }
        Row::new(row)
    });
    let focused = !app.optimize.gaming_focus;
    let block = theme.panel(&title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // The summary under the tests, when the panel has room for both.
    let summary_height = if inner.height >= 9 { 3 } else { 0 };
    let [table_area, summary_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(summary_height)]).areas(inner);
    let table = Table::new(rows, widths)
        .header(Row::new(header).style(theme.title))
        .row_highlight_style(theme.highlight(focused))
        .highlight_symbol("> ");
    let mut state = app.optimize.bench_state;
    frame.render_stateful_widget(table, table_area, &mut state);
    if summary_height > 0 {
        let lines = vec![Line::default(), summary(app)];
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: true }),
            summary_area,
        );
    }
}

/// The PC in one sentence once every test has a result, else how to get one.
fn summary(app: &App) -> Line<'static> {
    let theme = &app.theme;
    let scores: Vec<_> = Bench::ALL
        .iter()
        .filter_map(|&b| match app.optimize.results.get(&b) {
            Some((_, Ok(score))) => Some((b, *score)),
            _ => None,
        })
        .collect();
    let busy = app.optimize.running.is_some();
    let Some(summary) = optimize::summarize(&scores).filter(|_| !busy) else {
        let hint = if busy {
            String::new()
        } else {
            t!("optimize.run_all_first")
        };
        return Line::styled(format!(" {hint}"), theme.muted());
    };
    let (verdict, color) = match summary.tier {
        Tier::Slow => (t!("optimize.summary_slow"), theme.error),
        Tier::Fair => (t!("optimize.summary_fair"), theme.warning),
        Tier::Fast => (t!("optimize.summary_fast"), theme.success),
        Tier::VeryFast => (t!("optimize.summary_very_fast"), theme.success),
    };
    let text = match summary.weakest {
        Some(bench) => {
            let part = match bench {
                Bench::CpuSingle => t!("optimize.weak_cpu_single"),
                Bench::CpuMulti => t!("optimize.weak_cpu_multi"),
                Bench::Memory => t!("optimize.weak_memory"),
                Bench::Disk => t!("optimize.weak_disk"),
            };
            t!("optimize.weakest", verdict = verdict, part = part)
        }
        None => t!("optimize.balanced", verdict = verdict),
    };
    Line::styled(format!(" {text}"), Style::new().fg(color))
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
        assert!(
            text.contains("Test rapide (3 s) · n pour changer"),
            "{text}"
        );
        assert!(!text.contains("Niveau"), "{text}");
        assert!(text.contains("Mesure") && text.contains("1234 M op/s"));

        // Without room for the raw measure, the verdict stays.
        let text = screen(&app, 100, 30);
        assert!(text.contains("Très rapide") && !text.contains("op/s"));
        app.optimize.results.clear();
        assert!(!screen(&app, 140, 30).contains("Mesure"));
        app.optimize
            .on_finished(Bench::CpuSingle, false, Ok(Score::Ops(1234.4)));
        app.optimize
            .on_finished(Bench::Memory, true, Err("mémoire insuffisante".into()));
        app.optimize.running = Some(Bench::Disk);
        let text = screen(&app, 140, 30);
        // Measured in the other mode: marked next to the test.
        assert!(text.contains("Mémoire (copie) · complet  "), "{text}");
        assert!(text.contains("mémoire insuffisante"));
        assert!(!text.contains("Processeur (1 cœur) ·"), "{text}");
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
    fn summary_follows_the_run() {
        let gb = |n: f64| n * f64::from(1u32 << 30);
        let mut app = App::with_defaults();
        app.screen = Screen::Optimize;
        let text = screen(&app, 140, 30);
        assert!(text.contains("a lance tous les tests"), "{text}");

        // Running all: the next ones wait, no verdict yet.
        app.optimize.running = Some(Bench::CpuMulti);
        app.optimize.queue = vec![Bench::Memory, Bench::Disk];
        app.optimize
            .on_finished(Bench::CpuSingle, false, Ok(Score::Ops(678.0)));
        app.optimize.running = Some(Bench::CpuMulti);
        let text = screen(&app, 140, 30);
        assert_eq!(text.matches("en attente").count(), 2, "{text}");
        assert!(!text.contains("a lance tous les tests"), "{text}");

        app.optimize.queue.clear();
        for (bench, score) in [
            (Bench::CpuMulti, Score::Ops(6_300.0)),
            (Bench::Memory, Score::Bytes(gb(4.0))),
            (
                Bench::Disk,
                Score::Disk {
                    write: gb(2.0),
                    read: gb(2.0),
                },
            ),
        ] {
            app.optimize.on_finished(bench, false, Ok(score));
        }
        let text = screen(&app, 140, 30);
        assert!(
            text.contains("Bon PC de jeu — le point faible est la mémoire."),
            "{text}"
        );
        // Too short a panel: the tests keep the room.
        assert!(!screen(&app, 140, 10).contains("Bon PC"));
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
