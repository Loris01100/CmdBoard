use std::collections::HashSet;

use super::*;

#[test]
fn handed_over_session_resumes_where_it_was() {
    let mut app = App::with_defaults();
    let steam = steam_id(&app);
    let id = app.db.start_session(steam, unix_now() - 700).unwrap();
    app.db.checkpoint_session(id, 600).unwrap(); // handed over just now

    let resumed = app
        .resume_sessions(&HashSet::from([steam]), unix_now())
        .unwrap();
    assert_eq!(resumed, HashMap::from([(steam, Duration::from_secs(600))]));
    let session = app.active_sessions[&steam];
    assert_eq!((session.session_id, session.shown_secs()), (id, 600));
    assert_eq!(app.message, None); // nothing recovered as a crash
    assert_eq!(app.db.open_sessions().unwrap().len(), 1); // still running

    app.on_session_end(steam, 900);
    assert_eq!(app.find_app("Steam").unwrap().total_secs, 900); // one whole session
}

#[test]
fn stale_or_stopped_sessions_are_closed_like_after_a_crash() {
    for (running, later) in [(false, 0), (true, 10 * 60)] {
        let mut app = App::with_defaults();
        let steam = steam_id(&app);
        let id = app.db.start_session(steam, unix_now() - 700).unwrap();
        app.db.checkpoint_session(id, 600).unwrap();

        let running: HashSet<i64> = running.then_some(steam).into_iter().collect();
        let resumed = app.resume_sessions(&running, unix_now() + later).unwrap();
        assert!(resumed.is_empty());
        assert!(app.active_sessions.is_empty());
        assert!(app.db.open_sessions().unwrap().is_empty());
        assert_eq!(app.find_app("Steam").unwrap().total_xp, 15); // 10 min + 5 streak
    }
}

#[test]
fn quitting_hands_sessions_over_or_closes_them() {
    let mut app = App::with_defaults();
    let steam = steam_id(&app);
    app.on_session_start(steam);
    app.on_session_progress(steam, Duration::from_secs(120), false);
    app.hand_over_sessions().unwrap();
    assert!(app.active_sessions.is_empty());
    let open = app.db.open_sessions().unwrap();
    assert_eq!((open[0].app_id, open[0].secs), (steam, 120));

    // Turned off in config.toml: closed and awarded as before.
    let mut app = App::with_defaults();
    app.on_session_start(steam);
    assert!(!app.quit_sessions(false, Path::new("cmdboard.exe")).unwrap());
    assert!(app.db.open_sessions().unwrap().is_empty());
}

#[test]
fn background_loop_tracks_then_hands_over() {
    let mut app = App::with_defaults();
    let steam = steam_id(&app);
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(AppEvent::SessionStarted { app_id: steam }).unwrap();
    tx.send(AppEvent::SessionProgress {
        app_id: steam,
        played: Duration::from_secs(300),
        idle: false,
    })
    .unwrap();
    drop(tx); // the tracker stops
    app.run_background(&rx, || false).unwrap();
    let open = app.db.open_sessions().unwrap();
    assert_eq!((open.len(), open[0].secs), (1, 300));

    // A stop request ends it before anything else is read.
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(AppEvent::SessionEnded {
        app_id: steam,
        secs: 600,
    })
    .unwrap();
    app.run_background(&rx, || true).unwrap();
    assert_eq!(rx.try_iter().count(), 1);
}
