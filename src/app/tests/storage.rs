use super::*;

#[test]
fn storage_sorts_filters_and_confirms_uninstall() {
    let mut app = App::with_defaults();
    let program = |name: &str, size: Option<u64>, drive: char| Program {
        name: name.into(),
        publisher: None,
        size,
        drive: Some(drive),
        location: None,
        uninstall: "x.exe".into(),
    };
    let disk = |letter| Disk {
        letter,
        total: 100,
        free: 50,
    };
    app.storage.on_scanned(
        vec![disk('C'), disk('D')],
        vec![
            program("Alpha", Some(10), 'C'),
            program("Beta", None, 'C'),
            program("Gamma", Some(30), 'D'),
            program("Zeta", Some(20), 'C'),
        ],
    );
    let names = |app: &App| -> Vec<String> {
        app.storage
            .visible_programs()
            .iter()
            .map(|p| p.name.clone())
            .collect()
    };
    press(&mut app, KeyCode::Char('4'));
    assert_eq!(app.screen, Screen::Storage);
    assert_eq!(names(&app), ["Gamma", "Zeta", "Alpha", "Beta"]);
    press(&mut app, KeyCode::Char('s'));
    assert_eq!(names(&app), ["Alpha", "Zeta", "Gamma", "Beta"]); // unknown stays last
    press(&mut app, KeyCode::Tab);
    assert_eq!(names(&app), ["Alpha", "Zeta", "Beta"]); // C:
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.storage.disk, None); // D:, then every disk again
    press(&mut app, KeyCode::BackTab);
    assert_eq!(app.storage.disk, Some('D'));

    press(&mut app, KeyCode::Char('d'));
    let Mode::Popup(Popup::Confirm { command, .. }) = &app.mode else {
        panic!("expected a confirmation, got {:?}", app.mode);
    };
    assert_eq!(
        *command,
        Command::Uninstall {
            program: "Gamma".into(),
            confirmed: true
        }
    );
    press(&mut app, KeyCode::Esc); // never run a real uninstaller in tests
    run(&mut app, "uninstall nope");
    assert_eq!(message_kind(&app), Some(MsgKind::Error));
    press(&mut app, KeyCode::Char('0'));
    assert_eq!(app.screen, Screen::Help);
}

#[test]
fn folder_browser_without_threads() {
    let mut app = App::with_defaults();
    app.storage.disks = vec![Disk {
        letter: 'C',
        total: 100,
        free: 40,
    }];
    app.storage.programs = vec![Program {
        name: "Hades".into(),
        publisher: None,
        size: None,
        drive: Some('C'),
        location: Some(r"C:\Games\Hades".into()),
        uninstall: "x.exe".into(),
    }];
    let dir = PathBuf::from(r"C:\Games");
    let entry = |name: &str, is_dir: bool, size: Option<u64>| Entry {
        name: name.into(),
        path: dir.join(name),
        is_dir,
        size,
    };
    app.on_folder_listed(&dir, Vec::new()); // no browser open: ignored
    app.screen = Screen::Storage;
    app.open_folder(Some(dir.clone()), None);
    app.on_folder_listed(
        &dir,
        vec![
            entry("Hades", true, None),
            entry("notes.txt", false, Some(10)),
        ],
    );
    let names = |app: &App| -> Vec<String> {
        app.storage
            .visible_entries()
            .iter()
            .map(|e| e.name.clone())
            .collect()
    };
    assert_eq!(names(&app), ["notes.txt", "Hades"]); // unmeasured last

    let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(
        app.key_to_command(key(KeyCode::Char('d'))),
        Some(Command::Uninstall {
            program: "Hades".into(),
            confirmed: false
        })
    );
    press(&mut app, KeyCode::Char('k'));
    assert_eq!(
        app.key_to_command(key(KeyCode::Delete)),
        Some(Command::Trash {
            path: dir.join("notes.txt"),
            confirmed: false
        })
    );
    assert_eq!(
        app.key_to_command(key(KeyCode::Char('s'))),
        Some(Command::ToggleStorageOrder)
    );
    assert_eq!(app.key_to_command(key(KeyCode::Char('x'))), None);

    app.storage.on_folder_sized(dir.join("Hades"), 100);
    assert_eq!(names(&app), ["Hades", "notes.txt"]);
    assert_eq!(app.storage.selected_entry().unwrap().name, "notes.txt"); // followed it

    app.execute(Command::Trash {
        path: dir.join("notes.txt"),
        confirmed: true,
    }); // no event channel in tests
    assert_eq!(message_kind(&app), Some(MsgKind::Error));
    app.on_trashed(&dir.join("notes.txt"), Ok(()));
    assert_eq!(message_kind(&app), Some(MsgKind::Success));
    assert_eq!(names(&app), ["Hades"]);
    assert!(app.storage.folder_sizes.contains_key(&dir.join("Hades")));

    press(&mut app, KeyCode::Left); // C:\
    press(&mut app, KeyCode::Left); // the drives
    assert_eq!(app.storage.folders.as_ref().unwrap().dir, None);
    assert_eq!(app.key_to_command(key(KeyCode::Char('d'))), None);
    press(&mut app, KeyCode::Char('f'));
    assert!(app.storage.folders.is_none());
}

#[test]
fn storage_selection_and_unplugged_drive() {
    let mut app = App::with_defaults();
    let program = |name: &str| Program {
        name: name.into(),
        publisher: None,
        size: Some(1),
        drive: Some('C'),
        location: None,
        uninstall: "x.exe".into(),
    };
    app.storage.disk = Some('D');
    app.storage.on_scanned(
        vec![Disk {
            letter: 'C',
            total: 100,
            free: 40,
        }],
        vec![program("Alpha"), program("Beta")],
    );
    assert_eq!(app.storage.disk, None); // D: is gone
    press(&mut app, KeyCode::Char('4'));
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.storage.state.selected(), Some(1));
}
