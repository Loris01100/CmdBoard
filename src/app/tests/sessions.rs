use super::*;

#[test]
fn session_is_recorded_from_start_to_end() {
    let mut app = App::with_defaults();
    let steam = steam_id(&app);
    app.on_session_start(steam);
    app.on_session_start(steam); // duplicate start: ignored
    assert_eq!(app.active_sessions.len(), 1);
    assert_eq!(message_kind(&app), Some(MsgKind::Info));

    app.on_session_end(steam, 42 * 60);
    assert!(app.active_sessions.is_empty());
    // 42 XP for the minutes + 5 for a one-day streak (today).
    assert_eq!(
        app.message.as_ref().unwrap().0,
        "Session terminée : Steam (42 min, +47 XP)"
    );
    let entry = app.find_app("Steam").unwrap();
    assert_eq!(entry.total_secs, 42 * 60);
    assert_eq!(entry.total_xp, 47);
    assert_eq!(app.profile.total_xp, 47);
    // No level reached, but the first session ever unlocks a reward.
    assert_eq!(popup_title(&app), Some("Premiers pas".into()));
    assert!(entry.last_played.is_some());
}

#[test]
fn short_session_is_not_recorded() {
    let mut app = App::with_defaults();
    let steam = steam_id(&app);
    app.on_session_start(steam);
    app.on_session_end(steam, 5);
    assert_eq!(message_kind(&app), Some(MsgKind::Info));
    assert_eq!(app.find_app("Steam").unwrap().total_secs, 0);
}

#[test]
fn unknown_or_inactive_sessions_are_ignored() {
    let mut app = App::with_defaults();
    app.on_session_start(9_999);
    app.on_session_end(steam_id(&app), 600);
    assert!(app.active_sessions.is_empty());
    assert_eq!(app.message, None);
}

#[test]
fn removing_an_app_mid_session_is_harmless() {
    let mut app = App::with_defaults();
    let steam = steam_id(&app);
    app.on_session_start(steam);
    run(&mut app, "rm steam");
    press(&mut app, KeyCode::Enter);
    app.on_session_end(steam, 600); // the tracker notices afterwards
    assert!(app.active_sessions.is_empty());
    assert!(app.find_app("Steam").is_none());
}

#[test]
fn quitting_closes_running_sessions() {
    let mut app = App::with_defaults();
    app.on_session_start(steam_id(&app));
    app.end_all_sessions().unwrap();
    assert!(app.active_sessions.is_empty());
    assert!(app.db.recover_orphan_sessions().unwrap().is_empty()); // nothing left open
}

#[test]
fn orphan_sessions_earn_their_xp() {
    let mut app = App::with_defaults();
    let steam = steam_id(&app);
    let id = app.db.start_session(steam, unix_now() - 600).unwrap();
    app.db.checkpoint_session(id, 600).unwrap(); // then CmdBoard "crashed"

    app.close_orphan_sessions().unwrap();
    assert_eq!(app.find_app("Steam").unwrap().total_xp, 15); // 10 min + 5 streak
    assert_eq!(message_kind(&app), Some(MsgKind::Info));
    assert_eq!(popup_title(&app), Some("Premiers pas".into()));
}

#[test]
fn long_session_levels_up_with_popup_and_animation() {
    let mut app = App::with_defaults();
    let steam = steam_id(&app);
    app.on_session_start(steam);
    app.on_session_end(steam, 3 * 3600); // 180 + 5 XP: Steam and profile reach level 2

    let Mode::Popup(Popup::LevelUp(level_up)) = &app.mode else {
        panic!("expected a level-up, got {:?}", app.mode);
    };
    assert_eq!(level_up.app, "Steam");
    assert_eq!(level_up.app_level, Some(2));
    assert_eq!(level_up.global_level, Some(2));
    assert_eq!(level_up.gained, 185);

    // The bars start from the old value and fill up over a few ticks.
    let entry = app.find_app("Steam").unwrap().clone();
    assert_eq!(app.shown_app_xp(&entry), (1, 0));
    for _ in 0..XP_ANIM_FRAMES {
        app.on_tick();
    }
    assert_eq!(app.shown_app_xp(&entry), (2, 85));
    assert_eq!(app.shown_profile_xp(), (2, 85));
    assert!(app.xp_anims.is_empty() && app.profile_anim.is_none());

    // Then the rewards: first session ever, and 3 h in a row on Steam (plus
    // "Noctambule" when the test runs at night).
    press(&mut app, KeyCode::Enter);
    let mut rewards = Vec::new();
    while let Mode::Popup(Popup::RewardUnlocked(reward)) = &app.mode {
        rewards.push((reward.name.clone(), reward.app.clone()));
        press(&mut app, KeyCode::Esc);
    }
    assert_eq!(rewards[0], ("Premiers pas".to_string(), None));
    assert!(rewards.contains(&("Marathon".to_string(), Some("Steam".to_string()))));
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn rewards_unlock_once_globally_and_once_per_app() {
    let mut app = App::with_defaults();
    let steam = steam_id(&app);
    let notepad = app.find_app("Bloc-notes").unwrap().id;
    play(&mut app, steam, 3 * 3600);
    play(&mut app, steam, 3 * 3600);
    play(&mut app, notepad, 3 * 3600);

    assert_eq!(unlocked(&app, "Premiers pas"), [None]);
    assert_eq!(
        unlocked(&app, "Marathon"),
        [Some("Steam".to_string()), Some("Bloc-notes".to_string())]
    );
    assert!(unlocked(&app, "Centurion").is_empty());
    assert_eq!(app.find_app("Steam").unwrap().rewards, 1);
    assert!(matches!(
        &app.activity[0],
        Activity::Reward { name, .. } if name == "Marathon (Bloc-notes)"
    ));
}

#[test]
fn broken_rule_is_reported_and_others_still_unlock() {
    let mut app = App::with_defaults();
    app.db
        .execute_for_tests("UPDATE rewards SET rule = 'hours >= 1' WHERE code = 'marathon'");
    let steam = steam_id(&app);
    app.on_session_start(steam);
    app.on_session_end(steam, 3 * 3600);
    let (text, kind) = app.message.clone().unwrap();
    assert_eq!(kind, MsgKind::Error);
    assert!(
        text.contains("règle « marathon » : variable inconnue : hours"),
        "{text}"
    );
    play(&mut app, steam, 0); // closes the popups
    assert_eq!(unlocked(&app, "Premiers pas"), [None]);
}

#[test]
fn level_up_waits_while_typing() {
    let mut app = App::with_defaults();
    press(&mut app, KeyCode::Char(':'));
    let id = steam_id(&app);
    app.db.set_app_xp(id, 100).unwrap();
    app.reload().unwrap();
    app.on_xp_changed(id, 0, 0, 100);
    assert_eq!(app.mode, Mode::Command); // not interrupted
    press(&mut app, KeyCode::Esc);
    app.on_tick();
    assert!(matches!(app.mode, Mode::Popup(Popup::LevelUp(_))));
}

#[test]
fn tracker_receives_watch_list_on_reload() {
    let mut app = App::with_defaults();
    let (tx, rx) = std::sync::mpsc::channel();
    app.attach_tracker(tx);
    let list = rx.try_recv().unwrap();
    assert_eq!(list.len(), 4); // Explorateur has no watch_exe
    assert!(list.contains(&Watched {
        app_id: steam_id(&app),
        exe: "steam.exe".into()
    }));

    run(&mut app, "add Paint mspaint.exe");
    let list = rx.try_iter().last().unwrap();
    assert!(list.iter().any(|w| w.exe == "mspaint.exe"));
}

#[test]
fn running_sessions_checkpoint_every_minute() {
    let mut app = App::with_defaults();
    let steam = steam_id(&app);
    app.on_session_start(steam);
    let long_ago = Instant::now()
        .checked_sub(Duration::from_secs(120))
        .unwrap();
    app.on_session_progress(steam, Duration::from_secs(120), false);
    let session = app.active_sessions.get_mut(&steam).unwrap();
    session.last_checkpoint = long_ago;
    app.on_tick();
    assert!(app.active_sessions[&steam].last_checkpoint > long_ago);

    // A crash now: the checkpointed time is recovered at the next start.
    app.active_sessions.clear();
    app.close_orphan_sessions().unwrap();
    assert_eq!(app.find_app("Steam").unwrap().total_secs, 120);
}

#[test]
fn orphan_recovery_reports_nothing_or_broken_rules() {
    let mut app = App::with_defaults();
    app.close_orphan_sessions().unwrap();
    assert_eq!(app.message, None); // nothing was left open

    app.db
        .execute_for_tests("UPDATE rewards SET rule = 'hours >= 1' WHERE code = 'marathon'");
    let id = app
        .db
        .start_session(steam_id(&app), unix_now() - 600)
        .unwrap();
    app.db.checkpoint_session(id, 600).unwrap();
    app.close_orphan_sessions().unwrap();
    let (text, kind) = app.message.clone().unwrap();
    assert_eq!(kind, MsgKind::Error);
    assert!(text.contains("marathon"), "{text}");
}

#[test]
fn xp_display_without_animation_and_unknown_app() {
    let mut app = App::with_defaults();
    assert_eq!(app.shown_profile_xp(), (app.profile.level, app.profile.xp));
    app.on_xp_changed(9_999, 0, 0, 500);
    assert!(app.xp_anims.is_empty() && app.profile_anim.is_none());
    assert_eq!(app.mode, Mode::Normal);
}
