mod icons;
pub mod layout;
mod screens;
pub mod theme;
mod widgets;

use ratatui::Frame;

use crate::app::{App, Mode, Screen};

/// Pure rendering: reads `app`, never mutates it.
pub fn draw(frame: &mut Frame, app: &App) {
    match app.screen {
        Screen::Dashboard => screens::dashboard::draw(frame, app),
        Screen::Help => screens::help::draw(frame, app),
        Screen::Rewards => screens::rewards::draw(frame, app),
        Screen::Stats => screens::stats::draw(frame, app),
        Screen::Storage => screens::storage::draw(frame, app),
        Screen::Optimize => screens::optimize::draw(frame, app),
    }
    if let Mode::Popup(popup) = &app.mode {
        widgets::popup::render(frame, popup, app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn every_screen_renders_at_any_size() {
        let mut app = App::with_defaults();
        for screen in [
            Screen::Dashboard,
            Screen::Stats,
            Screen::Rewards,
            Screen::Storage,
            Screen::Optimize,
            Screen::Help,
        ] {
            app.screen = screen;
            for (w, h) in [(120, 30), (70, 20), (10, 3)] {
                let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                terminal.draw(|f| draw(f, &app)).unwrap();
            }
        }
    }

    #[test]
    fn command_line_shows_prompt_then_message() {
        use crate::app::Mode;
        let mut app = App::with_defaults();
        let render = |app: &App| {
            let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
            terminal.draw(|f| draw(f, app)).unwrap();
            let buffer = terminal.backend().buffer();
            buffer
                .content()
                .iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
        };
        app.mode = Mode::Command;
        "help add"
            .chars()
            .for_each(|c| app.command_line.input.insert(c));
        assert!(render(&app).contains(":help add"));

        let text = app.command_line.submit();
        app.mode = Mode::Normal;
        app.execute(crate::command::parser::parse(&text).unwrap());
        assert!(render(&app).contains("add [<nom> <cible>"));
    }

    #[test]
    fn header_shows_live_session() {
        let mut app = App::with_defaults();
        let render = |app: &App| {
            let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
            terminal.draw(|f| draw(f, app)).unwrap();
            let buffer = terminal.backend().buffer();
            buffer
                .content()
                .iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
        };
        assert!(render(&app).contains("aucune session en cours"));
        app.on_session_start(app.find_app("Steam").unwrap().id);
        assert!(render(&app).contains("▶ Steam 0:00"));
    }

    #[test]
    fn details_panel_hidden_when_narrow() {
        let app = App::with_defaults();
        let render = |w| {
            let mut terminal = Terminal::new(TestBackend::new(w, 30)).unwrap();
            terminal.draw(|f| draw(f, &app)).unwrap();
            let buffer = terminal.backend().buffer();
            buffer
                .content()
                .iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
        };
        assert!(render(120).contains("Détails"));
        assert!(!render(80).contains("Détails"));
    }

    fn screen_text(app: &App, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..h)
            .map(|y| (0..w).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n")
            .collect()
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "table-driven: one rendering scenario per rare state"
    )]
    fn less_common_states_render() {
        use crate::app::{Mode, MsgKind};
        use crate::command::{Command, complete::Completion};
        use crate::launcher::programs::{Disk, Program};
        use crate::optimize::{Bench, Gaming, Score};
        use crate::popup::{LevelUp, Picker, Popup};
        use crate::storage::models::Unlock;
        let mut app = App::with_defaults();

        // Search, then an empty category.
        app.mode = Mode::Search;
        assert!(screen_text(&app, 120, 30).contains("Esc"));
        app.mode = Mode::Normal;
        app.cat_state.select(None);
        screen_text(&app, 120, 30);
        app.cat_state.select(Some(0));

        // Completion with many candidates, on a box too narrow to type in.
        app.mode = Mode::Command;
        app.command_line.completion = Some((
            Completion {
                base: String::new(),
                candidates: (0..10).map(|i| format!("cmd{i}")).collect(),
            },
            1,
        ));
        assert!(screen_text(&app, 120, 30).contains('…'));
        screen_text(&app, 3, 5);
        if let Some((completion, _)) = &mut app.command_line.completion {
            completion.candidates.truncate(1); // a single candidate: no hint
        }
        assert!(!screen_text(&app, 120, 30).contains("cmd0"));
        app.mode = Mode::Normal;
        for kind in [MsgKind::Info, MsgKind::Success, MsgKind::Error] {
            app.message = Some(("bonjour".into(), kind));
            assert!(screen_text(&app, 120, 30).contains("bonjour"));
        }
        app.update_available = Some("9.9.9".into());
        assert!(screen_text(&app, 120, 30).contains(":update"));

        // Blinking level-up, then the picker while scanning, without match, with matches.
        app.frame_count = 1;
        app.mode = Mode::Popup(Popup::LevelUp(LevelUp {
            app: "Steam".into(),
            app_level: Some(2),
            global_level: None,
            gained: 10,
        }));
        screen_text(&app, 120, 30);
        let mut picker = Picker::new("Jeux");
        app.scan_running = true;
        app.mode = Mode::Popup(Popup::Picker(picker.clone()));
        screen_text(&app, 120, 30);
        app.scan_running = false;
        picker.query.set("zzz");
        app.mode = Mode::Popup(Popup::Picker(picker.clone()));
        screen_text(&app, 120, 30);
        app.shortcuts = (0..20)
            .map(|i| crate::launcher::scan::Shortcut {
                name: format!("Jeu {i}"),
                target: format!("jeu{i}.exe"),
                watch_exe: None,
            })
            .collect();
        picker.query.set("");
        picker.selected = 15;
        app.mode = Mode::Popup(Popup::Picker(picker));
        assert!(screen_text(&app, 120, 40).contains("Jeu 15"));
        app.mode = Mode::Normal;

        // Stats of one app.
        app.execute(Command::Stats {
            app: Some("Steam".into()),
        });
        assert!(screen_text(&app, 120, 30).contains("Steam"));

        // Rewards: scoped to an app, per app unlocked twice, none selected.
        app.screen = Screen::Rewards;
        let unlock = |app: &str| Unlock {
            app: Some(app.into()),
            date: "01/01/2026".into(),
        };
        app.rewards[0].app = Some("Steam".into());
        app.rewards[1].per_app = true;
        app.rewards[1].unlocks = vec![unlock("Steam"), unlock("Hades")];
        for selected in [Some(0), Some(1), None] {
            app.reward_state.select(selected);
            screen_text(&app, 120, 40);
        }
        app.aliases = crate::command::alias::Aliases::parse("[alias]\nboost = \"sort xp\"")
            .unwrap()
            .0;
        app.screen = Screen::Help;
        assert!(screen_text(&app, 120, 80).contains(":boost"));

        // Storage: an empty drive, one drive's programs smallest first, then folders.
        app.screen = Screen::Storage;
        app.storage.scanning = true;
        screen_text(&app, 120, 30);
        app.storage.on_scanned(
            vec![Disk {
                letter: 'C',
                total: 0,
                free: 0,
            }],
            vec![Program {
                name: "Hades".into(),
                publisher: None,
                size: Some(1),
                drive: Some('C'),
                location: None,
                uninstall: "x.exe".into(),
            }],
        );
        app.storage.disk = Some('C');
        app.storage.ascending = true;
        assert!(screen_text(&app, 120, 30).contains("Hades"));
        app.execute(Command::ToggleFolders);
        let dir = std::path::PathBuf::from(r"C:\");
        screen_text(&app, 120, 30); // listing
        app.on_folder_listed(&dir, Vec::new());
        screen_text(&app, 120, 30); // empty

        // Optimization: a setting off, the gaming focus, every score kind.
        app.screen = Screen::Optimize;
        app.optimize.gaming = vec![(Gaming::GameMode, false), (Gaming::Recording, true)];
        app.optimize.gaming_focus = true;
        app.optimize
            .on_finished(Bench::Memory, false, Ok(Score::Bytes(1e9)));
        app.optimize.on_finished(
            Bench::Disk,
            false,
            Ok(Score::Disk {
                write: 1e8,
                read: 2e8,
            }),
        );
        screen_text(&app, 120, 30);
    }

    #[test]
    fn every_theme_and_size_renders() {
        let mut app = App::with_defaults();
        for name in crate::ui::theme::available(None) {
            app.theme = crate::ui::theme::load(&name, None).unwrap();
            for screen in [
                Screen::Dashboard,
                Screen::Stats,
                Screen::Rewards,
                Screen::Storage,
                Screen::Optimize,
                Screen::Help,
            ] {
                app.screen = screen;
                for (w, h) in [(160, 50), (100, 30), (55, 22), (40, 12), (20, 6)] {
                    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                    terminal.draw(|f| draw(f, &app)).unwrap();
                }
            }
        }
    }
}
