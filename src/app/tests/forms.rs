use super::*;

#[test]
fn add_form_adds_app() {
    let mut app = App::with_defaults();
    press(&mut app, KeyCode::Char('a')); // picker first, empty in tests
    type_text(&mut app, "Paint");
    press(&mut app, KeyCode::Enter); // no match: by hand, the search becomes the name
    assert_eq!(form(&app).fields[0].input.text(), "Paint");
    assert_eq!(form(&app).fields[2].input.text(), "Jeux"); // selected category
    assert_eq!(form(&app).focused, 1);

    type_text(&mut app, "mspaint.exe");
    for _ in 0..3 {
        press(&mut app, KeyCode::Tab); // category, process, arguments
    }
    press(&mut app, KeyCode::Enter); // last field: submit

    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(message_kind(&app), Some(MsgKind::Success));
    let paint = app.selected_app().unwrap();
    assert_eq!(paint.name, "Paint");
    assert_eq!(paint.watch_exe.as_deref(), Some("mspaint.exe"));
}

#[test]
fn form_keeps_errors_inside() {
    let mut app = App::with_defaults();
    run(&mut app, "add");
    press(&mut app, KeyCode::Tab); // skip the picker
    for _ in 0..5 {
        press(&mut app, KeyCode::Enter); // empty name: submit fails on the last field
    }
    assert_eq!(form(&app).error.as_deref(), Some("Nom : champ requis"));
    assert_eq!(form(&app).focused, 0);

    type_text(&mut app, "steam"); // already exists, ignoring case
    assert_eq!(form(&app).error, None); // typing clears the error
    press(&mut app, KeyCode::Tab);
    type_text(&mut app, "x.exe");
    press(&mut app, KeyCode::BackTab);
    press(&mut app, KeyCode::BackTab); // wraps to the last field
    press(&mut app, KeyCode::Enter);
    assert_eq!(form(&app).error.as_deref(), Some("« steam » existe déjà"));

    press(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.apps.len(), 5);
}

#[test]
fn picker_fills_the_form_from_an_installed_app() {
    let mut app = App::with_defaults();
    let shortcut = |name: &str, target: &str, exe: Option<&str>| Shortcut {
        name: name.into(),
        target: target.into(),
        watch_exe: exe.map(Into::into),
    };
    app.shortcuts = vec![
        shortcut("Hades", r"C:\Start\Hades.lnk", Some("Hades.exe")),
        shortcut("Hollow Knight", "steam://rungameid/367520", None),
        shortcut("Steam", "STEAM://open/main", None), // already added: hidden
    ];
    press(&mut app, KeyCode::Char('a'));
    let Mode::Popup(Popup::Picker(picker)) = &app.mode else {
        panic!("{:?}", app.mode)
    };
    let names: Vec<_> = app
        .picker_matches(picker)
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(names, ["Hades", "Hollow Knight"]);

    type_text(&mut app, "hk");
    press(&mut app, KeyCode::Enter);
    let fields: Vec<_> = form(&app).fields.iter().map(|f| f.input.text()).collect();
    assert_eq!(
        fields,
        ["Hollow Knight", "steam://rungameid/367520", "Jeux", "", ""]
    );
    assert_eq!(form(&app).focused, 2); // only the category is left to check

    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Enter); // Hades, the first one
    assert_eq!(form(&app).fields[3].input.text(), "Hades.exe");
}

#[test]
fn move_form_moves_selected_app() {
    let mut app = App::with_defaults();
    press(&mut app, KeyCode::Char('m'));
    assert_eq!(form(&app).fields[0].input.text(), "Jeux");
    for _ in 0..4 {
        press(&mut app, KeyCode::Backspace);
    }
    type_text(&mut app, "Outils");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.selected_category().unwrap().name, "Outils");
    assert_eq!(app.selected_app().unwrap().name, "Steam");
}

#[test]
fn edit_form_changes_app_and_keeps_history() {
    let mut app = App::with_defaults();
    let steam = steam_id(&app);
    play(&mut app, steam, 600);
    let before = app.find_app_by_id(steam).unwrap().clone();

    press(&mut app, KeyCode::Char('e'));
    assert_eq!(form(&app).fields[1].input.text(), "steam://open/main");
    assert_eq!(form(&app).fields[2].input.text(), "Jeux");
    assert_eq!(form(&app).fields[3].input.text(), "steam.exe");
    type_text(&mut app, " Deck");
    for _ in 0..3 {
        press(&mut app, KeyCode::Tab);
    }
    for _ in 0.."steam.exe".len() {
        press(&mut app, KeyCode::Backspace);
    }
    press(&mut app, KeyCode::Tab);
    type_text(&mut app, "-silent");
    press(&mut app, KeyCode::Enter); // last field: submit

    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(message_kind(&app), Some(MsgKind::Success));
    let after = app.selected_app().unwrap();
    assert_eq!((after.id, after.name.as_str()), (steam, "Steam Deck"));
    assert_eq!(after.watch_exe, None); // empty, and a URI gives no exe
    assert_eq!(after.launch_args.as_deref(), Some("-silent"));
    assert_eq!(
        (after.total_secs, after.total_xp),
        (before.total_secs, before.total_xp)
    );
}
