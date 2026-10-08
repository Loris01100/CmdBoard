use super::*;

#[test]
fn optimize_screen_keys() {
    let mut app = App::with_defaults();
    press(&mut app, KeyCode::Char('5'));
    assert_eq!(app.screen, Screen::Optimize);
    assert!(app.optimize.system.is_some() && app.optimize.gaming.len() == Gaming::ALL.len());
    press(&mut app, KeyCode::Char('n'));
    assert!(app.optimize.heavy);
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.optimize.bench_state.selected(), Some(1));
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('k'));
    assert!(app.optimize.gaming_focus);
    assert_eq!(
        app.optimize.gaming_state.selected(),
        Some(Gaming::ALL.len() - 1)
    );
    assert_eq!(app.optimize.bench_state.selected(), Some(1));
    // No event channel in tests: the benchmark thread does not start.
    app.execute(Command::Bench(Bench::CpuSingle));
    assert_eq!(app.optimize.running, None);
    app.optimize.running = Some(Bench::Disk);
    app.execute(Command::Bench(Bench::CpuSingle));
    assert_eq!(message_kind(&app), Some(MsgKind::Error));
}
