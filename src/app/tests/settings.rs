use super::*;

#[test]
fn sort_orders_apps_and_keeps_selection() {
    let mut app = App::with_defaults();
    press(&mut app, KeyCode::Char('k')); // "Outils"
    press(&mut app, KeyCode::Char('l'));
    let names =
        |app: &App| -> Vec<String> { app.visible_apps().iter().map(|a| a.name.clone()).collect() };
    assert_eq!(names(&app), ["Bloc-notes", "Calculatrice", "Explorateur"]);

    let set_xp = |app: &mut App, name: &str, xp: u32| {
        let id = app.find_app(name).unwrap().id;
        app.db.set_app_xp(id, xp).unwrap();
        app.reload().unwrap();
    };
    set_xp(&mut app, "Explorateur", 100);
    set_xp(&mut app, "Calculatrice", 50);
    run(&mut app, "sort xp");
    assert_eq!(names(&app), ["Explorateur", "Calculatrice", "Bloc-notes"]);
    assert_eq!(app.selected_app().unwrap().name, "Bloc-notes"); // followed it
    set_xp(&mut app, "Bloc-notes", 500); // reload keeps the order
    assert_eq!(names(&app)[0], "Bloc-notes");

    press(&mut app, KeyCode::Char('s'));
    assert_eq!(app.sort, AppSort::Recent);
    run(&mut app, "sort name");
    assert_eq!(names(&app), ["Bloc-notes", "Calculatrice", "Explorateur"]);
    assert_eq!(
        app.init_sort(Some("size")).unwrap(),
        "config.toml : tri inconnu : size"
    );
}

#[test]
fn alias_runs_its_commands_in_order() {
    let mut app = with_aliases("[alias]\nboost = \"sort xp; stats $1\"");
    run(&mut app, "boost bloc-notes");
    assert_eq!(app.sort, AppSort::Xp);
    assert_eq!(app.screen, Screen::Stats);
    assert_eq!(app.stats_app, Some(app.find_app("Bloc-notes").unwrap().id));

    run(&mut app, "boost");
    assert_eq!(
        app.message.as_ref().unwrap().0,
        "alias : argument $1 manquant"
    );
    run(&mut app, "help boost");
    assert_eq!(
        app.message.as_ref().unwrap().0,
        "alias boost : sort xp; stats $1"
    );
}

#[test]
fn group_command_saves_a_launch_alias() {
    let dir = std::env::temp_dir().join(format!("cmdboard-app-group-{}", std::process::id()));
    let mut app = App::with_defaults();
    app.init_theme(&dir, None);
    run(&mut app, "group Outils bloc-notes, calculatrice");
    assert_eq!(message_kind(&app), Some(MsgKind::Success));
    assert_eq!(
        app.aliases.get("outils"),
        Some("launch Bloc-notes; launch Calculatrice")
    );
    assert_eq!(Aliases::load(&dir.join("commands.toml")).0, app.aliases);

    run(&mut app, "group outils nope");
    assert_eq!(message_kind(&app), Some(MsgKind::Error));
    assert_eq!(
        app.aliases.get("outils"),
        Some("launch Bloc-notes; launch Calculatrice")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn alias_stops_at_first_error_or_confirmation() {
    let mut app =
        with_aliases("[alias]\nbad = \"stats nope; sort xp\"\nclean = \"rm steam; sort xp\"");
    run(&mut app, "bad");
    assert_eq!(message_kind(&app), Some(MsgKind::Error));
    assert!(
        app.message
            .as_ref()
            .unwrap()
            .0
            .starts_with("stats nope : app inconnue")
    );
    assert_eq!(app.sort, AppSort::Name);

    run(&mut app, "clean");
    assert!(matches!(app.mode, Mode::Popup(Popup::Confirm { .. })));
    assert_eq!(app.sort, AppSort::Name);
}

#[test]
fn theme_command_switches_and_keeps_current_on_error() {
    let mut app = App::with_defaults();
    run(&mut app, "theme");
    let (text, _) = app.message.clone().unwrap();
    assert!(
        text.starts_with("Thèmes : catppuccin-frappe, catppuccin-latte"),
        "{text}"
    );
    assert!(text.ends_with("(actuel : terminal)"));

    run(&mut app, "theme Catppuccin-Latte");
    assert_eq!(app.message.as_ref().unwrap().0, "Thème : Catppuccin Latte");
    assert_eq!(app.theme_name, "catppuccin-latte");

    run(&mut app, "theme nope");
    assert_eq!(message_kind(&app), Some(MsgKind::Error));
    assert_eq!(app.theme.name, "Catppuccin Latte");
}

/// Stays in French: the language is global and tests run in parallel.
#[test]
fn lang_command_lists_and_rejects_unknown() {
    let mut app = App::with_defaults();
    run(&mut app, "lang");
    assert_eq!(
        app.message.as_ref().unwrap().0,
        "Langues : en, fr, pt (actuelle : fr)"
    );
    run(&mut app, "lang FR");
    assert_eq!(app.message.as_ref().unwrap().0, "Langue : Français");
    run(&mut app, "lang xx");
    assert_eq!(
        app.message.as_ref().unwrap().0,
        "langue inconnue : xx (:lang pour la liste)"
    );
}

#[test]
fn configured_theme_is_loaded_and_saved() {
    let dir = std::env::temp_dir().join(format!("cmdboard-app-theme-{}", std::process::id()));
    let mut app = App::with_defaults();
    assert_eq!(app.init_theme(&dir, Some("catppuccin-frappe".into())), None);
    assert_eq!(app.theme.name, "Catppuccin Frappé");

    run(&mut app, "theme catppuccin-macchiato");
    let (config, _) = config::Config::load(&dir.join("config.toml"));
    assert_eq!(config.theme.as_deref(), Some("catppuccin-macchiato"));

    let mut app = App::with_defaults();
    let warning = app.init_theme(&dir, Some("gone".into()));
    assert!(warning.unwrap().contains("thème inconnu"));
    assert_eq!(app.theme_name, theme::default_name()); // fell back
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn update_outcomes_reach_status_bar_and_message() {
    use update::{Action, Outcome};
    let mut app = App::with_defaults();
    run(&mut app, "update"); // no event channel in tests
    assert_eq!(message_kind(&app), Some(MsgKind::Error));

    app.message = None;
    app.on_update_finished(Action::Check, Err("offline".into()));
    assert_eq!(app.message, None); // the passive check stays silent
    let available = Outcome::Available {
        version: "0.2.0".into(),
        managed: true,
    };
    app.on_update_finished(Action::Check, Ok(available.clone()));
    assert_eq!(
        (app.update_available.as_deref(), &app.message),
        (Some("0.2.0"), &None)
    );

    app.on_update_finished(Action::Install, Ok(available));
    assert!(
        app.message
            .as_ref()
            .unwrap()
            .0
            .contains("winget upgrade CmdBoard")
    );
    app.on_update_finished(
        Action::Install,
        Ok(Outcome::Installed {
            version: "0.2.0".into(),
        }),
    );
    assert_eq!(message_kind(&app), Some(MsgKind::Success));
    assert_eq!(app.update_available, None);
}

#[test]
fn sort_by_time_from_config_and_listed() {
    let mut app = App::with_defaults();
    let notepad = app.find_app("Bloc-notes").unwrap().id;
    play(&mut app, notepad, 600);
    assert_eq!(app.init_sort(None), None);
    assert_eq!(app.init_sort(Some("TIME")), None);
    assert_eq!(app.sort, AppSort::Time);
    assert_eq!(app.apps[0].name, "Bloc-notes");

    run(&mut app, "sort");
    let (text, kind) = app.message.clone().unwrap();
    assert_eq!(kind, MsgKind::Info);
    assert!(text.contains("name, xp, recent, time"), "{text}");
}

#[test]
fn sort_is_saved_to_config() {
    let dir = std::env::temp_dir().join(format!("cmdboard-app-sort-{}", std::process::id()));
    let mut app = App::with_defaults();
    app.init_theme(&dir, None);
    run(&mut app, "sort recent");
    let (config, _) = config::Config::load(&dir.join("config.toml"));
    assert_eq!(config.sort.as_deref(), Some("recent"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn install_outcomes_show_a_message() {
    use update::{Action, Outcome};
    let mut app = App::with_defaults();
    app.update_available = Some("9.9.9".into());
    app.on_update_finished(Action::Install, Ok(Outcome::UpToDate));
    assert_eq!(app.update_available, None);
    assert_eq!(message_kind(&app), Some(MsgKind::Info));
    app.on_update_finished(Action::Install, Err("offline".into()));
    assert_eq!(message_kind(&app), Some(MsgKind::Error));

    app.update_running = true;
    run(&mut app, "update");
    assert_eq!(message_kind(&app), Some(MsgKind::Error));
    assert!(app.update_running);
}

#[test]
fn clear_commands_ask_first() {
    let mut app = App::with_defaults();
    let steam = steam_id(&app);
    play(&mut app, steam, 600);
    run(&mut app, "clear sessions");
    press(&mut app, KeyCode::Enter);
    assert_eq!(message_kind(&app), Some(MsgKind::Success));
    assert!(
        app.activity
            .iter()
            .all(|a| !matches!(a, Activity::Session { .. }))
    );

    run(&mut app, "clear stats");
    press(&mut app, KeyCode::Char('y'));
    assert_eq!(message_kind(&app), Some(MsgKind::Success));
    assert_eq!(app.stats.session_count, 0);
}

#[test]
fn export_then_import() {
    let path =
        std::env::temp_dir().join(format!("cmdboard-app-export-{}.json", std::process::id()));
    let mut app = App::with_defaults();
    let export = |confirmed| Command::Export {
        path: Some(path.display().to_string()),
        confirmed,
    };
    app.execute(export(false));
    assert_eq!(
        message_kind(&app),
        Some(MsgKind::Success),
        "{:?}",
        app.message
    );

    // The file exists now: asks before replacing it, and leaves it alone on Esc.
    std::fs::write(&path, "keep me").unwrap();
    app.execute(export(false));
    assert!(matches!(
        &app.mode,
        Mode::Popup(Popup::Confirm { command, .. }) if *command == export(true)
    ));
    press(&mut app, KeyCode::Esc);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep me");
    app.execute(export(true));
    assert_eq!(message_kind(&app), Some(MsgKind::Success));
    assert_ne!(std::fs::read_to_string(&path).unwrap(), "keep me");
    app.execute(Command::Import {
        path: path.display().to_string(),
    });
    assert_eq!(
        message_kind(&app),
        Some(MsgKind::Success),
        "{:?}",
        app.message
    );
    std::fs::remove_file(&path).unwrap();
}
