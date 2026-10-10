use super::*;
use crossterm::event::{KeyEvent, KeyModifiers};

fn alt(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
}

fn pins(app: &App) -> Vec<(u8, String)> {
    (1..=9)
        .filter_map(|slot| Some((slot, app.pinned(slot)?.name.clone())))
        .collect()
}

#[test]
fn p_pins_to_the_first_free_slot_and_unpins() {
    let mut app = App::with_defaults();
    run(&mut app, "pin");
    assert_eq!(
        app.message.as_ref().unwrap().0,
        "Aucun favori : p sur une app, ou :pin <1-9> <app>"
    );
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('p')); // Steam, selected
    assert_eq!(app.message.as_ref().unwrap().0, "Favori 1 : Steam (Alt+1)");
    run(&mut app, "pin 3 Windows Terminal");
    run(&mut app, "pin 2 bloc-notes");
    assert_eq!(
        pins(&app),
        [
            (1, "Steam".into()),
            (2, "Bloc-notes".into()),
            (3, "Windows Terminal".into())
        ]
    );
    run(&mut app, "pin");
    assert_eq!(
        app.message.as_ref().unwrap().0,
        "Favoris : 1 Steam · 2 Bloc-notes · 3 Windows Terminal"
    );

    // A taken slot changes hands; `p` on a favorite unpins it.
    run(&mut app, "pin 1 Calculatrice");
    assert_eq!(app.find_app("Steam").unwrap().pin, None);
    press(&mut app, KeyCode::Char('p')); // Steam again: back in slot 4, the first free
    assert_eq!(app.find_app("Steam").unwrap().pin, Some(4));
    press(&mut app, KeyCode::Char('p'));
    assert_eq!(app.find_app("Steam").unwrap().pin, None);
    run(&mut app, "pin off Steam");
    assert_eq!(app.message.as_ref().unwrap().0, "Steam n'est pas un favori");
}

#[test]
fn alt_digit_launches_a_favorite_from_any_screen() {
    let mut app = App::with_defaults();
    // An empty slot says how to fill it, from any screen.
    press(&mut app, KeyCode::Char('2')); // Stats
    app.on_key(alt('5'));
    assert_eq!(app.screen, Screen::Stats); // not the Optimization screen
    assert_eq!(
        app.message.as_ref().unwrap().0,
        "Aucun favori en 5 (p sur une app, ou :pin 5 <app>)"
    );
    // A missing exe fails before anything starts: the error names the pinned target.
    app.db.execute_for_tests(
        r"UPDATE apps SET launch_target = 'C:\cmdboard-missing.exe' WHERE name = 'Steam'",
    );
    app.reload().unwrap();
    run(&mut app, "pin 5 Steam");
    app.on_key(alt('5'));
    let (text, kind) = app.message.clone().unwrap();
    assert_eq!(kind, MsgKind::Error);
    assert!(text.contains(r"C:\cmdboard-missing.exe"), "{text}");
}

#[test]
fn favorite_keys_follow_the_keyboard_layout() {
    let app = App::with_defaults();
    let slot = |key: KeyEvent| match app.key_to_command(key) {
        Some(Command::LaunchPin { slot }) => Some(slot),
        _ => None,
    };
    assert_eq!(slot(alt('1')), Some(1));
    assert_eq!(slot(alt('9')), Some(9));
    // AZERTY: the digit row without Shift.
    assert_eq!(slot(alt('&')), Some(1));
    assert_eq!(slot(alt('é')), Some(2));
    assert_eq!(slot(alt('ç')), Some(9));
    // Alt+Shift+1 on AZERTY still reads as `1`.
    let shifted = KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT | KeyModifiers::SHIFT);
    assert_eq!(slot(shifted), Some(1));
    // AltGr (Ctrl+Alt) types characters; plain digits switch screens.
    let altgr = KeyEvent::new(
        KeyCode::Char('1'),
        KeyModifiers::ALT | KeyModifiers::CONTROL,
    );
    assert_eq!(slot(altgr), None);
    assert_eq!(slot(alt('0')), None);
    assert_eq!(
        app.key_to_command(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE)),
        Some(Command::Show(Screen::Dashboard))
    );
}

#[test]
fn recent_row_lists_the_last_played_apps() {
    let mut app = App::with_defaults();
    // Nothing played: the first category is selected, "Recent" is empty above it.
    assert_eq!(app.selected_category().unwrap().name, "Jeux");
    assert!(app.recent_apps().is_empty());

    let terminal = app.find_app("Windows Terminal").unwrap().id;
    let steam = steam_id(&app);
    play(&mut app, steam, 600);
    let id = app.db.start_session(terminal, unix_now()).unwrap();
    app.db.close_session(id, unix_now() + 60, 120).unwrap(); // ends last
    app.reload().unwrap();

    press(&mut app, KeyCode::Char('k'));
    assert!(app.recents_selected());
    assert!(app.selected_category().is_none());
    let names: Vec<_> = app.visible_apps().iter().map(|a| a.name.clone()).collect();
    assert_eq!(names, ["Windows Terminal", "Steam"]); // most recent first
    // No category to remove or to add into from there.
    press(&mut app, KeyCode::Char('d'));
    assert_eq!(app.mode, Mode::Normal);
    press(&mut app, KeyCode::Enter); // to the apps
    assert_eq!(app.selected_app().unwrap().name, "Windows Terminal");
    // Selecting an app by name goes to its own category.
    run(&mut app, "stats");
    press(&mut app, KeyCode::Char('1'));
    app.execute(Command::Select {
        app: "Steam".into(),
    });
    assert_eq!(app.selected_category().unwrap().name, "Jeux");
}

#[test]
fn launch_arguments_are_saved_by_the_edit_form() {
    let mut app = App::with_defaults();
    run(&mut app, "edit Steam");
    for _ in 0..4 {
        press(&mut app, KeyCode::Tab);
    }
    type_text(&mut app, "-silent");
    press(&mut app, KeyCode::Enter);
    let steam = app.find_app("Steam").unwrap();
    assert_eq!(steam.launch_args.as_deref(), Some("-silent"));
}
