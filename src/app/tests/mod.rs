//! Behavior of `App` driven like the user would: keys, command lines and thread events.
//! Each file covers one `app` module.

mod forms;
mod keys;
mod library;
mod optimize;
mod sessions;
mod settings;
mod storage;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::sessions::XP_ANIM_FRAMES;
use super::*;
use crate::command::{Command, alias::Aliases};
use crate::config;
use crate::event::AppEvent;
use crate::launcher::{
    folders::Entry,
    programs::{Disk, Program},
    scan::Shortcut,
};
use crate::optimize::{Bench, Gaming};
use crate::popup::{Form, FormKind, Picker, Popup};
use crate::storage::{models::Activity, unix_now};
use crate::tracker::Watched;
use crate::ui::theme;
use crate::update;

fn press(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

/// Types `:` + `text` + Enter.
fn run(app: &mut App, text: &str) {
    press(app, KeyCode::Char(':'));
    text.chars().for_each(|c| press(app, KeyCode::Char(c)));
    press(app, KeyCode::Enter);
}

fn message_kind(app: &App) -> Option<MsgKind> {
    app.message.as_ref().map(|(_, kind)| *kind)
}

#[test]
fn step_wraps_both_ways() {
    assert_eq!(step(Some(2), 3, true), Some(0));
    assert_eq!(step(Some(0), 3, false), Some(2));
    assert_eq!(step(None, 3, true), Some(0));
    assert_eq!(step(Some(0), 0, true), None);
}

#[test]
fn clamp_keeps_selection_in_range() {
    assert_eq!(clamp(None, 3), Some(0));
    assert_eq!(clamp(Some(5), 3), Some(2));
    assert_eq!(clamp(Some(1), 0), None);
}

#[test]
fn loads_from_database() {
    let app = App::with_defaults();
    assert_eq!(app.selected_category().unwrap().name, "Jeux");
    assert_eq!(app.selected_app().unwrap().name, "Steam");
}

fn type_text(app: &mut App, text: &str) {
    text.chars().for_each(|c| press(app, KeyCode::Char(c)));
}

fn form(app: &App) -> &Form {
    match &app.mode {
        Mode::Popup(Popup::Form(form)) => form,
        other => panic!("expected a form, got {other:?}"),
    }
}

fn steam_id(app: &App) -> i64 {
    app.find_app("Steam").unwrap().id
}

/// Name of the reward shown in the current popup, if any.
fn popup_title(app: &App) -> Option<String> {
    match &app.mode {
        Mode::Popup(Popup::RewardUnlocked(reward)) => Some(reward.name.clone()),
        _ => None,
    }
}

/// Plays a whole session on `app_id` and closes every popup it opens.
fn play(app: &mut App, app_id: i64, secs: u64) {
    app.on_session_start(app_id);
    app.on_session_end(app_id, secs);
    while matches!(app.mode, Mode::Popup(_)) {
        press(app, KeyCode::Esc);
    }
}

fn unlocked(app: &App, name: &str) -> Vec<Option<String>> {
    let reward = app.rewards.iter().find(|r| r.name == name).unwrap();
    reward.unlocks.iter().map(|u| u.app.clone()).collect()
}

fn with_aliases(toml: &str) -> App {
    let mut app = App::with_defaults();
    app.aliases = Aliases::parse(toml).unwrap().0;
    app
}

#[test]
fn q_quits() {
    let mut app = App::with_defaults();
    press(&mut app, KeyCode::Char('q'));
    assert!(app.should_quit);
}

#[test]
fn ctrl_c_quits_from_any_mode() {
    let mut app = App::with_defaults();
    press(&mut app, KeyCode::Char(':'));
    app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.should_quit);
}

#[test]
fn screens_and_sorts_have_distinct_labels() {
    use std::collections::HashSet;
    let screens = [
        Screen::Dashboard,
        Screen::Stats,
        Screen::Rewards,
        Screen::Storage,
        Screen::Optimize,
        Screen::Help,
    ];
    let titles: HashSet<String> = screens.into_iter().map(Screen::title).collect();
    assert_eq!(titles.len(), screens.len());
    let labels: HashSet<String> = AppSort::ALL.into_iter().map(AppSort::label).collect();
    assert_eq!(labels.len(), AppSort::ALL.len());
}

#[test]
fn events_are_routed_to_their_handler() {
    let mut app = App::with_defaults();
    let steam = steam_id(&app);
    app.handle(AppEvent::Key(KeyEvent::new(
        KeyCode::Char('2'),
        KeyModifiers::NONE,
    )));
    assert_eq!(app.screen, Screen::Stats);
    app.handle(AppEvent::Tick);
    assert_eq!(app.frame_count, 1);

    app.handle(AppEvent::SessionStarted { app_id: steam });
    assert_eq!(app.active_sessions.len(), 1);
    app.handle(AppEvent::SessionProgress {
        app_id: steam,
        played: Duration::from_secs(90),
        idle: true,
    });
    let session = app.active_sessions[&steam];
    assert!(session.idle);
    assert_eq!(session.shown_secs(), 90); // idle: the timer stands still
    app.handle(AppEvent::SessionEnded {
        app_id: steam,
        secs: 5,
    });
    assert!(app.active_sessions.is_empty());

    app.handle(AppEvent::UpdateFinished {
        action: update::Action::Check,
        result: Ok(update::Outcome::Available {
            version: "9.9.9".into(),
            managed: false,
        }),
    });
    assert_eq!(app.update_available.as_deref(), Some("9.9.9"));

    let disk = Disk {
        letter: 'C',
        total: 100,
        free: 40,
    };
    app.handle(AppEvent::StorageScanned {
        disks: vec![disk],
        programs: Vec::new(),
    });
    assert_eq!(app.storage.disks.len(), 1);

    app.handle(AppEvent::BenchFinished {
        bench: Bench::Memory,
        heavy: true,
        result: Err("x".into()),
    });
    assert_eq!(
        app.optimize.results[&Bench::Memory],
        (true, Err("x".into()))
    );

    // The drives list, then events for it.
    app.toggle_folders();
    let drive = PathBuf::from(r"C:\");
    assert_eq!(app.storage.visible_entries()[0].size, Some(60));
    app.handle(AppEvent::FolderListed {
        dir: drive.clone(),
        entries: Vec::<Entry>::new(),
    }); // not the shown folder: ignored
    app.handle(AppEvent::FolderProgress {
        path: drive.clone(),
        percent: 40,
    });
    assert_eq!(app.storage.folders.as_ref().unwrap().progress[&drive], 40);
    app.handle(AppEvent::FolderSized {
        path: drive.clone(),
        size: 70,
    });
    assert_eq!(app.storage.visible_entries()[0].size, Some(70));
    app.handle(AppEvent::Trashed {
        path: drive,
        result: Err("refusé".into()),
    });
    assert_eq!(
        app.message.as_ref().unwrap(),
        &("refusé".into(), MsgKind::Error)
    );

    let mut picker = Picker::new("Jeux");
    picker.selected = 3;
    app.mode = Mode::Popup(Popup::Picker(picker));
    app.scan_running = true;
    app.handle(AppEvent::ShortcutsScanned(vec![Shortcut {
        name: "Hades".into(),
        target: "hades.exe".into(),
        watch_exe: None,
    }]));
    assert!(!app.scan_running);
    assert_eq!(app.shortcuts.len(), 1);
    assert!(matches!(&app.mode, Mode::Popup(Popup::Picker(p)) if p.selected == 0));
}

#[test]
fn threads_report_through_the_event_channel() {
    let dir = std::env::temp_dir().join(format!("cmdboard-app-threads-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub").join("a.bin"), [0u8; 300]).unwrap();

    let mut app = App::with_defaults();
    let (tx, rx) = std::sync::mpsc::channel();
    let config = config::Config {
        update_check: false,
        ..Default::default()
    };
    app.attach_events(tx, &config);
    assert!(app.storage.scanning && !app.update_running);
    app.show(Screen::Storage); // already scanning: no second thread

    app.open_folder(Some(dir.clone()), None);
    app.execute(Command::OpenForm(FormKind::Add)); // the picker scans the installed apps
    assert!(app.scan_running);
    app.execute(Command::OpenForm(FormKind::Add)); // already scanning: no second thread

    let wait = Duration::from_secs(60);
    while app.storage.scanning
        || app.scan_running
        || app.storage.folder_sizes.get(&dir.join("sub")) != Some(&300)
    {
        app.handle(rx.recv_timeout(wait).unwrap());
    }
    assert_eq!(app.storage.visible_entries()[0].name, "sub");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn queued_events_cost_one_draw() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = App::with_defaults();
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    for _ in 0..3 {
        tx.send(AppEvent::Resize).unwrap();
    }
    tx.send(AppEvent::Key(KeyCode::Char('q').into())).unwrap();
    app.run(&mut terminal, &rx).unwrap();
    // The first frame only: the queued events, quit included, were handled together.
    assert_eq!(terminal.get_frame().count(), 1);
}
