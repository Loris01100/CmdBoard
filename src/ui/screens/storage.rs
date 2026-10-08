use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::Style,
    symbols,
    text::{Line, Span},
    widgets::{Cell, LineGauge, Paragraph, Row, Table},
};

use crate::app::App;
use crate::launcher::programs::{self, Disk};
use crate::ui::{
    layout,
    theme::Theme,
    widgets::{command_line, format_size, status_bar},
};

/// Space used on each drive, then the installed programs by size, on one drive or all,
/// or (`f`) the folder browser.
pub fn draw(frame: &mut Frame, app: &App) {
    let (body, command, status) = layout::screen(frame.area(), command_line::height(app));
    // On short terminals the programs keep the room and the drives go.
    let disks_height = if body.height >= 12 && !app.storage.disks.is_empty() {
        app.storage.disks.len() as u16 + 2
    } else {
        0
    };
    let [disks_area, programs_area] =
        Layout::vertical([Constraint::Length(disks_height), Constraint::Min(3)]).areas(body);
    if disks_height > 0 {
        draw_disks(frame, disks_area, app);
    }
    if app.storage.folders.is_some() {
        draw_folders(frame, programs_area, app);
    } else {
        draw_programs(frame, programs_area, app);
    }
    command_line::render(frame, command, app);
    status_bar::render(frame, status, app);
}

fn draw_disks(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let block = theme.panel(&t!("storage.disks"), false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical(vec![Constraint::Length(1); app.storage.disks.len()]).split(inner);
    for (disk, &row) in app.storage.disks.iter().zip(rows.iter()) {
        let [letter_area, gauge_area, text_area] = Layout::horizontal([
            Constraint::Length(6),
            Constraint::Min(10),
            Constraint::Length(36),
        ])
        .spacing(1)
        .areas(row);
        let chosen = app.storage.disk == Some(disk.letter);
        let letter = format!("{} {}:", if chosen { ">" } else { " " }, disk.letter);
        let letter_style = if chosen { theme.title } else { Style::new() };
        frame.render_widget(
            Paragraph::new(Span::styled(letter, letter_style)),
            letter_area,
        );

        let ratio = used_ratio(disk);
        let color = match ratio {
            r if r >= 0.9 => theme.error,
            r if r >= 0.75 => theme.warning,
            _ => theme.success,
        };
        let gauge = LineGauge::default()
            .ratio(ratio)
            .label(format!("{:>3.0}%", ratio * 100.0))
            .filled_symbol(symbols::line::THICK_HORIZONTAL)
            .unfilled_symbol(symbols::line::THICK_HORIZONTAL)
            .filled_style(Style::new().fg(color))
            .unfilled_style(theme.muted());
        frame.render_widget(gauge, gauge_area);

        let usage = t!(
            "storage.disk_usage",
            used = format_size(disk.total - disk.free.min(disk.total)),
            total = format_size(disk.total),
            free = format_size(disk.free),
        );
        frame.render_widget(
            Paragraph::new(Span::styled(usage, theme.muted())).alignment(Alignment::Right),
            text_area,
        );
    }
}

fn used_ratio(disk: &Disk) -> f64 {
    if disk.total == 0 {
        return 0.0;
    }
    (1.0 - disk.free as f64 / disk.total as f64).clamp(0.0, 1.0)
}

fn draw_programs(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let programs = app.storage.visible_programs();
    let drive = match app.storage.disk {
        Some(letter) => format!("{letter}:"),
        None => t!("storage.all_disks"),
    };
    let order = if app.storage.ascending {
        t!("storage.smallest_first")
    } else {
        t!("storage.biggest_first")
    };
    let title = t!("storage.programs", drive, order, count = programs.len());
    let block = theme.panel(&title, true);

    if programs.is_empty() {
        let text = if app.storage.scanning {
            t!("storage.scanning")
        } else {
            t!("storage.none")
        };
        frame.render_widget(
            Paragraph::new(Span::styled(text, theme.muted())).block(block),
            area,
        );
        return;
    }

    let header = Row::new([
        t!("storage.name"),
        t!("storage.size"),
        t!("storage.drive"),
        t!("storage.publisher"),
    ])
    .style(theme.title);
    let rows = programs.iter().map(|program| {
        let size = size_cell(program.size, "—", theme);
        let drive = program.drive.map_or("—".into(), |d| format!("{d}:"));
        Row::new([
            Cell::from(program.name.as_str()),
            size,
            Cell::from(Line::from(drive).alignment(Alignment::Center)),
            Cell::from(Span::styled(
                program.publisher.clone().unwrap_or_default(),
                theme.muted(),
            )),
        ])
    });
    let widths = [
        Constraint::Fill(2),
        Constraint::Length(10),
        Constraint::Length(7),
        Constraint::Fill(1),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(theme.highlight(true))
        .highlight_symbol("> ");
    // Rendering needs `&mut TableState`; work on a copy so `draw` stays pure.
    let mut state = app.storage.state;
    frame.render_stateful_widget(table, area, &mut state);
}

/// The browsed folder's files and subfolders by size. Subfolders show "…", then the
/// percentage of their children measured, until measured.
fn draw_folders(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let Some(folders) = &app.storage.folders else {
        return;
    };
    let entries = app.storage.visible_entries();
    let dir = match &folders.dir {
        Some(dir) => dir.display().to_string(),
        None => t!("storage.drives"),
    };
    let order = if app.storage.ascending {
        t!("storage.smallest_first")
    } else {
        t!("storage.biggest_first")
    };
    let measuring = entries.iter().filter(|e| e.size.is_none()).count();
    let measuring = match measuring {
        0 => String::new(),
        count => t!("storage.measuring", count),
    };
    let title = t!(
        "storage.folders",
        dir,
        order,
        count = entries.len(),
        measuring
    );
    let block = theme.panel(&title, true);

    if entries.is_empty() {
        let text = if folders.listing {
            t!("storage.listing")
        } else {
            t!("storage.empty_folder")
        };
        frame.render_widget(
            Paragraph::new(Span::styled(text, theme.muted())).block(block),
            area,
        );
        return;
    }

    let header = Row::new([
        t!("storage.name"),
        t!("storage.size"),
        t!("storage.program"),
    ])
    .style(theme.title);
    let rows = entries.iter().map(|entry| {
        let name = if entry.is_dir {
            format!("{}\\", entry.name.trim_end_matches('\\'))
        } else {
            entry.name.clone()
        };
        // A program's folder: `d` uninstalls it instead of deleting it.
        let program = match folders.dir {
            Some(_) => programs::installed_in(&app.storage.programs, &entry.path)
                .map_or(String::new(), |p| p.name.clone()),
            None => String::new(),
        };
        Row::new([
            Cell::from(name),
            size_cell(
                entry.size,
                &folders
                    .progress
                    .get(&entry.path)
                    .map_or("…".into(), |p| format!("{p} %")),
                theme,
            ),
            Cell::from(Span::styled(program, theme.muted())),
        ])
    });
    let widths = [
        Constraint::Fill(2),
        Constraint::Length(10),
        Constraint::Fill(1),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(theme.highlight(true))
        .highlight_symbol("> ");
    let mut state = folders.state;
    frame.render_stateful_widget(table, area, &mut state);
}

/// Right-aligned size, or `missing` greyed out.
fn size_cell(size: Option<u64>, missing: &str, theme: &Theme) -> Cell<'static> {
    let line = match size {
        Some(bytes) => Line::from(format_size(bytes)),
        None => Line::styled(missing.to_string(), theme.muted()),
    };
    Cell::from(line.alignment(Alignment::Right))
}

#[cfg(test)]
mod tests {
    use crate::app::{App, Screen};
    use crate::launcher::programs::{Disk, Program};
    use ratatui::{Terminal, backend::TestBackend};

    fn screen(app: &App, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| crate::ui::draw(f, app)).unwrap();
        let buffer = terminal.backend().buffer();
        buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    fn program(name: &str, size: Option<u64>, drive: char) -> Program {
        Program {
            name: name.into(),
            publisher: Some("Éditeur".into()),
            size: size.map(|mb| mb << 20),
            drive: Some(drive),
            location: None,
            uninstall: "x.exe".into(),
        }
    }

    #[test]
    fn shows_disks_and_programs() {
        let mut app = App::with_defaults();
        app.screen = Screen::Storage;
        assert!(screen(&app, 110, 30).contains("Aucun programme"));

        let gb = 1 << 30;
        app.storage.on_scanned(
            vec![Disk {
                letter: 'C',
                total: 100 * gb,
                free: 5 * gb,
            }],
            vec![
                program("Blender", Some(1500), 'C'),
                program("Inconnu", None, 'D'),
            ],
        );
        let text = screen(&app, 110, 30);
        assert!(text.contains("95%"), "{text}");
        assert!(text.contains("95 Go / 100 Go · 5,0 Go libres"), "{text}");
        assert!(text.contains("Programmes · tous les disques · plus gros d'abord (2)"));
        assert!(
            text.contains("Blender") && text.contains("1,5 Go"),
            "{text}"
        );
        assert!(text.contains("Inconnu"));

        // Short and narrow: still renders, programs first.
        let text = screen(&app, 40, 10);
        assert!(!text.contains("95%") && text.contains("Blender"), "{text}");
    }

    #[test]
    fn browses_folders() {
        use crate::launcher::folders::Entry;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let press = |app: &mut App, code| app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
        let mut app = App::with_defaults();
        app.screen = Screen::Storage;
        let gb = 1 << 30;
        app.storage.on_scanned(
            vec![Disk {
                letter: 'C',
                total: 100 * gb,
                free: 40 * gb,
            }],
            Vec::new(),
        );
        press(&mut app, KeyCode::Char('f'));
        let text = screen(&app, 110, 30);
        assert!(text.contains("Dossiers · disques"), "{text}");
        assert!(text.contains(r"C:\") && text.contains("60 Go"), "{text}");

        press(&mut app, KeyCode::Enter);
        assert!(screen(&app, 110, 30).contains("Lecture du dossier"));
        let dir = std::path::PathBuf::from(r"C:\");
        let entry = |name: &str, is_dir, size| Entry {
            name: name.into(),
            path: dir.join(name),
            is_dir,
            size,
        };
        app.on_folder_listed(
            &dir,
            vec![
                entry("Dev", true, None),
                entry("big.iso", false, Some(4 * gb)),
            ],
        );
        let text = screen(&app, 110, 30);
        assert!(text.contains("1 dossier(s) en cours de mesure"), "{text}");
        assert!(text.contains(r"Dev\") && text.contains("…"), "{text}");
        app.storage.on_folder_progress(dir.join("Dev"), 40);
        assert!(screen(&app, 110, 30).contains("40 %"));

        app.storage.on_folder_sized(dir.join("Dev"), 9 * gb);
        let names: Vec<_> = app
            .storage
            .visible_entries()
            .iter()
            .map(|e| e.name.clone())
            .collect();
        assert_eq!(names, ["Dev", "big.iso"]);
        assert!(!screen(&app, 110, 30).contains("en cours de mesure"));

        // The selection followed big.iso when Dev went first; `d` asks before trashing it.
        press(&mut app, KeyCode::Char('d'));
        let crate::app::Mode::Popup(crate::popup::Popup::Confirm { command, .. }) = &app.mode
        else {
            panic!("expected a confirmation, got {:?}", app.mode);
        };
        assert_eq!(
            *command,
            crate::command::Command::Trash {
                path: dir.join("big.iso"),
                confirmed: true
            }
        );
        press(&mut app, KeyCode::Esc);

        press(&mut app, KeyCode::Left); // back to the drives
        assert!(screen(&app, 110, 30).contains("Dossiers · disques"));
        press(&mut app, KeyCode::Char('f')); // back to the programs
        assert!(screen(&app, 110, 30).contains("Programmes"));
    }
}
