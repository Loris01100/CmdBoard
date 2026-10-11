use super::*;

/// Elden Ring: 800h; Hades: owned, never played.
fn steam() -> HashMap<u32, u64> {
    HashMap::from([(1_245_620, 48_000), (1_145_360, 0)])
}

fn xp_of(app: &App, name: &str) -> u32 {
    app.find_app(name).unwrap().total_xp
}

#[test]
fn an_added_steam_game_gets_its_past_playtime() {
    let mut app = App::with_defaults();
    app.steam_playtime = || Some(steam());
    run(
        &mut app,
        "add \"Elden Ring\" steam://rungameid/1245620 Jeux",
    );
    assert_eq!(
        app.message.as_ref().unwrap().0,
        "Ajouté : Elden Ring · Steam : 800h de jeu avant CmdBoard, +48000 XP"
    );
    assert_eq!(xp_of(&app, "Elden Ring"), 48_000);
    assert!(matches!(app.mode, Mode::Popup(Popup::LevelUp(_))));
    press(&mut app, KeyCode::Esc);

    run(&mut app, "add Hades steam://rungameid/1145360 Jeux");
    assert_eq!(app.message.as_ref().unwrap().0, "Ajouté : Hades");
    run(&mut app, r"add Notes C:\Windows\notepad.exe Outils");
    assert_eq!(app.message.as_ref().unwrap().0, "Ajouté : Notes");
}

#[test]
fn startup_imports_each_steam_game_once() {
    let mut app = App::with_defaults(); // Steam not read yet
    run(
        &mut app,
        "add \"Elden Ring\" steam://rungameid/1245620 Jeux",
    );
    run(&mut app, "add Hades steam://run/1145360 Jeux");
    assert_eq!(xp_of(&app, "Elden Ring"), 0);
    assert_eq!(app.import_steam_playtime(), None); // still unreadable: tried again later

    app.steam_playtime = || Some(steam());
    assert_eq!(
        app.import_steam_playtime(),
        Some((
            "Steam : 800h de jeu avant CmdBoard · +48000 XP".into(),
            MsgKind::Success
        ))
    );
    assert_eq!(app.profile.total_xp, 48_000);
    assert_eq!(app.import_steam_playtime(), None); // done once
    assert_eq!(xp_of(&app, "Elden Ring"), 48_000);
}
