use super::*;

fn game(name: &str, target: &str, exe: Option<&str>) -> Shortcut {
    Shortcut {
        name: name.into(),
        target: target.into(),
        watch_exe: exe.map(String::from),
    }
}

/// Elden Ring, not added; Steam, added under that name; a game without an exe to watch.
fn with_library() -> App {
    let mut app = App::with_defaults();
    app.handle(AppEvent::LibraryScanned(vec![
        game(
            "ELDEN RING",
            "steam://rungameid/1245620",
            Some(r"D:\Games\ELDEN RING\eldenring.exe"),
        ),
        game("Steam", "steam://open/main", Some("steam.exe")),
        game("Hades", "steam://rungameid/1145360", None),
    ]));
    app
}

#[test]
fn only_games_not_added_are_watched_for_suggestions() {
    let app = with_library();
    assert_eq!(
        app.full_watch_list().unadded,
        [r"D:\Games\ELDEN RING\eldenring.exe"]
    );
    assert_eq!(app.suggestion(), None); // not running
}

#[test]
fn a_running_game_is_suggested_then_added_with_n() {
    let mut app = with_library();
    press(&mut app, KeyCode::Char('n'));
    assert_eq!(app.mode, Mode::Normal); // nothing to add yet

    app.handle(AppEvent::UnaddedRunning(vec!["eldenring.exe".into()]));
    assert_eq!(
        app.suggestion().map(|g| g.name.as_str()),
        Some("ELDEN RING")
    );
    press(&mut app, KeyCode::Char('n'));
    assert_eq!(form(&app).fields[0].input.text(), "ELDEN RING");
    for _ in 0..5 {
        if app.mode == Mode::Normal {
            break;
        }
        press(&mut app, KeyCode::Enter); // through the pre-filled fields, then submit
    }
    let added = app.find_app("ELDEN RING").expect("added");
    assert_eq!(added.launch_target, "steam://rungameid/1245620");

    // Watched as an app from now on: no longer suggested.
    let list = app.full_watch_list();
    assert!(list.unadded.is_empty());
    assert!(list.apps.iter().any(|w| w.exe.ends_with("eldenring.exe")));
    assert_eq!(app.suggestion(), None);
}

#[test]
fn the_suggestion_goes_once_the_game_closes() {
    let mut app = with_library();
    app.handle(AppEvent::UnaddedRunning(vec!["eldenring.exe".into()]));
    assert!(app.suggestion().is_some());
    app.handle(AppEvent::UnaddedRunning(Vec::new()));
    assert_eq!(app.suggestion(), None);
}

#[test]
fn x_ignores_the_running_game_until_shown_again() {
    let mut app = with_library();
    app.handle(AppEvent::UnaddedRunning(vec!["eldenring.exe".into()]));
    press(&mut app, KeyCode::Char('x'));
    assert_eq!(
        app.message.as_ref().unwrap().0,
        "ELDEN RING ne sera plus proposé (:ignore off ELDEN RING pour annuler)"
    );
    assert_eq!(app.suggestion(), None);
    assert!(app.full_watch_list().unadded.is_empty()); // the tracker stops looking
    press(&mut app, KeyCode::Char('x'));
    assert_eq!(app.mode, Mode::Normal); // nothing left to ignore: the key does nothing

    run(&mut app, "ignore");
    assert_eq!(
        app.message.as_ref().unwrap().0,
        "Jeux ignorés : ELDEN RING (:ignore off <jeu> pour annuler)"
    );
    run(&mut app, "ignore off Hades");
    assert_eq!(
        app.message.as_ref().unwrap().0,
        "jeu non ignoré : Hades (:ignore pour la liste)"
    );
    run(&mut app, "ignore off elden ring");
    assert_eq!(message_kind(&app), Some(MsgKind::Success));
    assert_eq!(
        app.suggestion().map(|g| g.name.as_str()),
        Some("ELDEN RING")
    );
    run(&mut app, "ignore");
    assert_eq!(
        app.message.as_ref().unwrap().0,
        "Aucun jeu ignoré (x au dashboard ignore le jeu lancé)"
    );
}

#[test]
fn ignored_games_from_config_are_never_watched() {
    let mut app = App::with_defaults();
    app.init_ignored_games(vec!["elden ring".into()]);
    app.handle(AppEvent::LibraryScanned(vec![game(
        "ELDEN RING",
        "steam://rungameid/1245620",
        Some("eldenring.exe"),
    )]));
    assert!(app.full_watch_list().unadded.is_empty());
}
