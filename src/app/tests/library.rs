use super::*;

#[test]
fn changing_category_resets_app_selection() {
    let mut app = App::with_defaults();
    press(&mut app, KeyCode::Char('k')); // "Recent", above the categories
    press(&mut app, KeyCode::Char('k')); // wraps to "Outils"
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.selected_app().unwrap().name, "Calculatrice");

    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::Char('k'));
    assert_eq!(app.selected_category().unwrap().name, "Dev");
    assert_eq!(app.selected_app().unwrap().name, "Windows Terminal");
}

#[test]
fn add_creates_category_and_selects_app() {
    let mut app = App::with_defaults();
    run(
        &mut app,
        r#"add "Hollow Knight" steam://rungameid/367520 Metroidvania"#,
    );
    assert_eq!(
        message_kind(&app),
        Some(MsgKind::Success),
        "{:?}",
        app.message
    );
    assert_eq!(app.selected_category().unwrap().name, "Metroidvania");
    assert_eq!(app.selected_app().unwrap().name, "Hollow Knight");
    assert_eq!(app.focus, Focus::Apps);

    run(&mut app, r#"add "hollow knight" x.exe"#); // same name, other case
    assert_eq!(message_kind(&app), Some(MsgKind::Error));
}

#[test]
fn add_without_category_uses_selected_one() {
    let mut app = App::with_defaults();
    run(&mut app, "add Paint mspaint.exe");
    let paint = app.selected_app().unwrap();
    assert_eq!(app.selected_category().unwrap().name, "Jeux");
    assert_eq!(paint.watch_exe.as_deref(), Some("mspaint.exe"));
}

#[test]
fn move_follows_the_app() {
    let mut app = App::with_defaults();
    run(&mut app, "mv bloc-notes dev");
    assert_eq!(
        app.message.as_ref().unwrap().0,
        "Déplacé : Bloc-notes → Dev"
    );
    assert_eq!(app.selected_category().unwrap().name, "Dev");
    assert_eq!(app.selected_app().unwrap().name, "Bloc-notes");
}

#[test]
fn edit_refuses_a_taken_name_but_allows_a_new_case() {
    let mut app = App::with_defaults();
    let edit = |name: &str| Command::Edit {
        app: "steam".into(),
        name: name.into(),
        target: "steam://open/main".into(),
        category: "Jeux".into(),
        watch_exe: Some("steam.exe".into()),
        args: None,
    };
    app.execute(edit("Windows Terminal"));
    assert_eq!(message_kind(&app), Some(MsgKind::Error));
    assert_ne!(app.find_app("Windows Terminal").unwrap().id, steam_id(&app));

    app.execute(edit("STEAM"));
    assert_eq!(message_kind(&app), Some(MsgKind::Success));
    assert_eq!(app.find_app("steam").unwrap().name, "STEAM");
}

#[test]
fn delete_app_asks_first() {
    let mut app = App::with_defaults();
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('d'));
    assert!(matches!(app.mode, Mode::Popup(Popup::Confirm { .. })));
    press(&mut app, KeyCode::Char('n'));
    assert_eq!(app.apps.len(), 5);

    press(&mut app, KeyCode::Char('d'));
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.apps.len(), 4);
    assert!(app.find_app("Steam").is_none());
    assert_eq!(app.selected_app().map(|a| a.name.as_str()), None); // Jeux is empty now
}

#[test]
fn rm_from_command_line_also_asks() {
    let mut app = App::with_defaults();
    run(&mut app, "rm bloc-notes");
    assert!(matches!(app.mode, Mode::Popup(Popup::Confirm { .. })));
    press(&mut app, KeyCode::Char('o'));
    assert!(app.find_app("Bloc-notes").is_none());
}

#[test]
fn only_empty_categories_can_be_removed() {
    let mut app = App::with_defaults();
    press(&mut app, KeyCode::Char('d')); // "Jeux" holds Steam
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(message_kind(&app), Some(MsgKind::Error));

    run(&mut app, "mv steam Dev");
    run(&mut app, "rmcat jeux");
    press(&mut app, KeyCode::Enter);
    assert_eq!(message_kind(&app), Some(MsgKind::Success));
    assert!(app.categories.iter().all(|c| c.name != "Jeux"));
}

#[test]
fn rewards_screen_has_its_own_selection() {
    let mut app = App::with_defaults();
    press(&mut app, KeyCode::Char('3'));
    assert_eq!(app.reward_state.selected(), Some(0));
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.reward_state.selected(), Some(1));
    press(&mut app, KeyCode::Char('k'));
    press(&mut app, KeyCode::Char('k')); // wraps
    assert_eq!(app.reward_state.selected(), Some(app.rewards.len() - 1));
    assert_eq!(app.selected_app().unwrap().name, "Steam"); // dashboard untouched
}

#[test]
fn stats_command_filters_and_resets() {
    let mut app = App::with_defaults();
    let steam = steam_id(&app);
    play(&mut app, steam, 600);
    let notepad = app.find_app("Bloc-notes").unwrap().id;
    play(&mut app, notepad, 1200);

    run(&mut app, "stats steam");
    assert_eq!(app.screen, Screen::Stats);
    assert_eq!(app.stats.session_count, 1);
    assert_eq!(app.stats_state.selected(), Some(0));

    run(&mut app, "stats");
    assert_eq!(app.stats_app, None);
    assert_eq!(app.stats.session_count, 2);
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.stats_state.selected(), Some(1));

    run(&mut app, "stats inconnue");
    assert_eq!(message_kind(&app), Some(MsgKind::Error));
}

#[test]
fn stats_of_a_removed_app_fall_back_to_all() {
    let mut app = App::with_defaults();
    run(&mut app, "stats steam");
    assert!(app.stats_app.is_some());
    app.db.delete_app(steam_id(&app)).unwrap();
    app.reload().unwrap();
    assert_eq!(app.stats_app, None);
}

#[test]
fn no_category_selected() {
    let mut app = App::with_defaults();
    app.cat_state.select(None);
    assert!(app.visible_apps().is_empty());
    run(&mut app, "add Paint mspaint.exe");
    assert_eq!(message_kind(&app), Some(MsgKind::Error));
    app.select_app(9_999); // unknown: nothing changes
    assert_eq!(app.cat_state.selected(), None);
}
