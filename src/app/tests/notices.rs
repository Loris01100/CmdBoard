use super::*;
use crate::app::sessions::ActiveSession;
use crate::notify::Level;

/// Notifications on, as `attach_notifications` would set them, but kept in the queue.
fn notifying(break_minutes: u64) -> App {
    let mut app = App::with_defaults();
    app.notices.enabled = true;
    app.notices.break_after = (break_minutes > 0).then(|| Duration::from_mins(break_minutes));
    app
}

fn bodies(app: &App) -> Vec<&str> {
    app.notices.queue.iter().map(|n| n.body.as_str()).collect()
}

#[test]
fn a_limit_is_notified_once_per_period() {
    let mut app = notifying(0);
    let steam = steam_id(&app);
    run(&mut app, "limit 30m/day Steam");
    run(&mut app, "goal 30m/day Jeux"); // reached too, but only shown in the UI
    app.on_session_start(steam);
    app.on_session_progress(steam, Duration::from_mins(29), false);
    assert!(app.notices.queue.is_empty());
    app.on_session_progress(steam, Duration::from_mins(31), false);
    app.on_session_progress(steam, Duration::from_mins(40), false);

    let notice = &app.notices.queue[..];
    assert_eq!(notice.len(), 1);
    assert_eq!(
        (notice[0].level, notice[0].title.as_str()),
        (Level::Warning, "Limite atteinte")
    );
    assert_eq!(bodies(&app), ["Steam : 31m / 30m aujourd'hui"]);
}

#[test]
fn nothing_is_notified_when_turned_off() {
    let mut app = App::with_defaults(); // not attached: off
    app.notices.break_after = Some(Duration::from_mins(30));
    let steam = steam_id(&app);
    run(&mut app, "limit 30m/day Steam");
    app.on_session_start(steam);
    app.on_session_progress(steam, Duration::from_mins(31), false);
    assert!(app.notices.queue.is_empty());
}

#[test]
fn breaks_are_suggested_until_the_user_pauses() {
    let mut app = notifying(30);
    let steam = steam_id(&app);
    app.on_session_start(steam);
    for minutes in [29, 30, 45, 61] {
        app.on_session_progress(steam, Duration::from_mins(minutes), false);
    }
    assert_eq!(
        bodies(&app),
        [
            "Vous jouez à Steam depuis 30m",
            "Vous jouez à Steam depuis 1h01"
        ]
    );
    assert_eq!(app.notices.queue[0].level, Level::Info);

    // Idle: a break, so the count starts again from there.
    app.notices.queue.clear();
    app.on_session_progress(steam, Duration::from_mins(70), true);
    app.on_session_progress(steam, Duration::from_mins(95), false);
    assert!(app.notices.queue.is_empty());
    app.on_session_progress(steam, Duration::from_mins(100), false);
    assert_eq!(bodies(&app), ["Vous jouez à Steam depuis 30m"]);
}

#[test]
fn a_resumed_session_is_not_reminded_twice() {
    let mut app = notifying(30);
    let steam = steam_id(&app);
    // Handed over by the other process after 65 minutes: reminded at 30 and 60 there.
    app.active_sessions
        .insert(steam, ActiveSession::new(1, Duration::from_mins(65)));
    app.on_session_progress(steam, Duration::from_mins(66), false);
    assert!(app.notices.queue.is_empty());
    app.on_session_progress(steam, Duration::from_mins(90), false);
    assert_eq!(bodies(&app), ["Vous jouez à Steam depuis 1h30"]);
}
