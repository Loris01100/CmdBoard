use super::*;
use crate::optimize::Score;

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

#[test]
fn run_all_queues_every_benchmark() {
    let mut app = App::with_defaults();
    press(&mut app, KeyCode::Char('5'));
    assert_eq!(
        app.key_to_command(KeyEvent::from(KeyCode::Char('a'))),
        Some(Command::BenchAll)
    );
    // No event channel in tests: nothing starts, nothing stays queued.
    app.execute(Command::BenchAll);
    assert_eq!((app.optimize.running, app.optimize.queue.len()), (None, 0));
    app.optimize.running = Some(Bench::Disk);
    app.execute(Command::BenchAll);
    assert_eq!(message_kind(&app), Some(MsgKind::Error));

    // A finished benchmark is recorded, then the next one would start.
    app.optimize.queue = vec![Bench::Memory, Bench::Disk];
    app.on_bench_finished(Bench::CpuMulti, true, Ok(Score::Ops(6_300.0)));
    assert!(app.optimize.results.contains_key(&Bench::CpuMulti));
    assert_eq!((app.optimize.running, app.optimize.queue.len()), (None, 0));
}
