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
