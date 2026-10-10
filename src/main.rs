#[macro_use]
mod i18n;

mod app;
mod command;
mod config;
mod core;
mod event;
mod fuzzy;
mod instance;
mod launcher;
mod optimize;
mod popup;
mod storage;
mod text_input;
mod tracker;
mod ui;
mod update;

use std::sync::mpsc;

use app::{App, MsgKind};
use command::alias::Aliases;
use config::Config;
use instance::Instance;
use storage::{Database, db::data_dir};

/// `cmdboard --stop`: stops the background tracker.
const STOP_FLAG: &str = "--stop";

fn main() -> anyhow::Result<()> {
    // Broken user files are reported, never fatal.
    let dir = data_dir()?;
    let mut warnings = Vec::new();
    let (config, warning) = Config::load(&dir.join("config.toml"));
    warnings.extend(warning);
    // Before the database, so a new one gets its starter content in this language.
    warnings.extend(i18n::init(config.lang.as_deref()));

    match std::env::args().nth(1).as_deref() {
        Some(instance::BACKGROUND_FLAG) => return run_background(&config),
        Some(STOP_FLAG) => return stop_background(),
        _ => {}
    }

    // Held until exit: a second CmdBoard would track every session twice.
    let Some(_instance) = take_over_instance()? else {
        anyhow::bail!(t!("error.already_running"));
    };
    // Where to start the background tracker from on quit, even if `:update` replaces
    // this exe meanwhile.
    let exe = std::env::current_exe()?;

    // Open the database before taking over the terminal, so errors print normally.
    let db = Database::open_default()?;
    let mut app = App::new(db)?;
    // Handed over by the background tracker, or left open by a crash.
    let resumed = app.recover_sessions()?;

    warnings.extend(app.init_theme(&dir, config.theme.clone()));
    warnings.extend(app.init_sort(config.sort.as_deref()));
    let (aliases, warning) = Aliases::load(&dir.join("commands.toml"));
    app.aliases = aliases;
    warnings.extend(warning);
    if !warnings.is_empty() {
        app.message = Some((warnings.join(" · "), MsgKind::Error));
    }

    let (tx, rx) = mpsc::channel();
    app.attach_tracker(tracker::spawn(tx.clone(), config.idle_limit(), resumed));
    app.attach_events(tx.clone(), &config);
    event::spawn(tx);

    let mut terminal = ratatui::init(); // also installs the panic hook
    let result = app.run(&mut terminal, &rx);
    ratatui::restore();
    result?;
    if app.quit_sessions(config.background, &exe)? {
        println!("{}", t!("background.started"));
    }
    Ok(())
}

/// The instance, taken over from the background tracker if it holds it. `None` when
/// another UI holds it.
fn take_over_instance() -> anyhow::Result<Option<Instance>> {
    if let Some(instance) = instance::acquire()? {
        return Ok(Some(instance));
    }
    if !instance::request_stop() {
        return Ok(None);
    }
    instance::acquire_within(|| false)
}

/// `cmdboard --background`, started by the UI as it quits: tracks sessions without a
/// terminal until the UI starts again or `--stop`.
fn run_background(config: &Config) -> anyhow::Result<()> {
    let stop = instance::StopSignal::create()?;
    // The UI lets go of the instance as it quits.
    let Some(_instance) = instance::acquire_within(|| stop.is_requested())? else {
        return Ok(());
    };
    let mut app = App::new(Database::open_default()?)?;
    let resumed = app.recover_sessions()?;
    let (tx, rx) = mpsc::channel();
    app.attach_tracker(tracker::spawn(tx, config.idle_limit(), resumed));
    app.run_background(&rx, || stop.is_requested())
}

/// `cmdboard --stop`: the background tracker saves its sessions and quits.
fn stop_background() -> anyhow::Result<()> {
    if !instance::request_stop() {
        println!("{}", t!("background.not_running"));
        return Ok(());
    }
    // Stopped once it lets go of the instance.
    if instance::acquire_within(|| false)?.is_none() {
        anyhow::bail!(t!("background.stop_failed"));
    }
    println!("{}", t!("background.stopped"));
    Ok(())
}
