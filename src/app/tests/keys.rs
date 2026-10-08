use super::*;

#[test]
fn enter_on_categories_focuses_apps() {
    let mut app = App::with_defaults();
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.focus, Focus::Apps);
}

#[test]
fn colon_opens_and_esc_cancels() {
    let mut app = App::with_defaults();
    press(&mut app, KeyCode::Char(':'));
    assert_eq!(app.mode, Mode::Command);
    press(&mut app, KeyCode::Char('q')); // typed, not quit
    assert!(!app.should_quit);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.command_line.input.is_empty());
}

#[test]
fn errors_are_reported_not_fatal() {
    let mut app = App::with_defaults();
    for line in ["launch Inexistant", "move Inexistant Dev", "fly", "add x"] {
        run(&mut app, line);
        assert_eq!(message_kind(&app), Some(MsgKind::Error), "{line}");
    }
}

#[test]
fn help_shows_screen_or_usage() {
    let mut app = App::with_defaults();
    run(&mut app, "help add");
    assert_eq!(message_kind(&app), Some(MsgKind::Info));
    run(&mut app, "help");
    assert_eq!(app.screen, Screen::Help);
}

#[test]
fn history_recalls_previous_command() {
    let mut app = App::with_defaults();
    run(&mut app, "help add");
    press(&mut app, KeyCode::Char(':'));
    press(&mut app, KeyCode::Up);
    assert_eq!(app.command_line.input.text(), "help add");
}

#[test]
fn search_selects_an_app_from_any_category() {
    let mut app = App::with_defaults();
    press(&mut app, KeyCode::Char('/'));
    assert_eq!(app.mode, Mode::Search);
    assert_eq!(app.visible_apps().len(), app.apps.len()); // empty query: everything
    type_text(&mut app, "wterm");
    assert_eq!(app.selected_app().unwrap().name, "Windows Terminal");
    press(&mut app, KeyCode::Enter);

    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.selected_category().unwrap().name, "Dev");
    assert_eq!(app.selected_app().unwrap().name, "Windows Terminal");
    assert_eq!(app.focus, Focus::Apps);
}

#[test]
fn cancelled_search_restores_selection() {
    let mut app = App::with_defaults();
    press(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "bloc");
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.focus, Focus::Categories);
    assert_eq!(app.selected_app().unwrap().name, "Steam");

    press(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "zzz");
    assert!(app.selected_app().is_none());
    press(&mut app, KeyCode::Enter); // no match: like Esc
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.selected_app().unwrap().name, "Steam");
}

#[test]
fn tab_completes_in_the_command_line() {
    let mut app = App::with_defaults();
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "lau");
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.command_line.input.text(), "launch ");
    type_text(&mut app, "calc");
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.command_line.input.text(), "launch Calculatrice");
}

#[test]
fn search_keys_move_and_backspace_leaves() {
    let mut app = App::with_defaults();
    press(&mut app, KeyCode::Char('/'));
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.app_state.selected(), Some(1));
    press(&mut app, KeyCode::BackTab);
    press(&mut app, KeyCode::Up); // wraps
    assert_eq!(app.app_state.selected(), Some(app.apps.len() - 1));
    press(&mut app, KeyCode::Backspace); // empty query: leaves the search
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.selected_app().unwrap().name, "Steam");
}

#[test]
fn command_line_keys() {
    let mut app = App::with_defaults();
    run(&mut app, "sort");
    press(&mut app, KeyCode::Char(':'));
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Down); // past the newest: empty again
    assert!(app.command_line.input.is_empty());
    press(&mut app, KeyCode::Backspace); // empty line: leaves
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn keys_map_to_commands_per_screen() {
    let mut app = App::with_defaults();
    let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
    let cases = [
        (KeyCode::Char('1'), Some(Command::Show(Screen::Dashboard))),
        (KeyCode::Char('2'), Some(Command::Show(Screen::Stats))),
        (KeyCode::Char('z'), None),
    ];
    for (code, expected) in cases {
        assert_eq!(app.key_to_command(key(code)), expected, "{code:?}");
    }
    app.focus = Focus::Apps;
    assert_eq!(
        app.key_to_command(key(KeyCode::Enter)),
        Some(Command::Launch {
            app: "Steam".into()
        })
    );

    app.screen = Screen::Stats;
    assert_eq!(
        app.key_to_command(key(KeyCode::Char('s'))),
        Some(Command::ToggleStatsPie)
    );
    assert_eq!(app.key_to_command(key(KeyCode::Char('a'))), None);
    app.screen = Screen::Storage;
    assert_eq!(app.key_to_command(key(KeyCode::Char('x'))), None);

    app.screen = Screen::Optimize;
    app.optimize.gaming = vec![(Gaming::GameMode, true)];
    assert_eq!(
        app.key_to_command(key(KeyCode::Enter)),
        Some(Command::Bench(Bench::ALL[0]))
    );
    assert_eq!(app.key_to_command(key(KeyCode::Char('x'))), None);
    app.optimize.gaming_focus = true;
    assert_eq!(
        app.key_to_command(key(KeyCode::Enter)),
        Some(Command::ToggleGaming(Gaming::GameMode))
    );
    assert_eq!(
        app.key_to_command(key(KeyCode::Char('o'))),
        Some(Command::OpenGamingPage(Gaming::GameMode))
    );
}

#[test]
fn popup_keys_cancel_or_wait() {
    let mut app = App::with_defaults();
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Esc); // closes the picker
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(message_kind(&app), Some(MsgKind::Info));

    run(&mut app, "rm steam");
    press(&mut app, KeyCode::Char('x')); // neither yes nor no
    assert!(matches!(app.mode, Mode::Popup(Popup::Confirm { .. })));
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, Focus::Categories);
}
