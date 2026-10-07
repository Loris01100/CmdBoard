#[macro_use]
mod i18n;

mod app;
mod command;
mod config;
mod core;
mod event;
mod fuzzy;
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
use storage::{Database, db::data_dir};

fn main() -> anyhow::Result<()> {
    // Broken user files are reported, never fatal.
    let dir = data_dir()?;
    let mut warnings = Vec::new();
    let (config, warning) = Config::load(&dir.join("config.toml"));
    warnings.extend(warning);
    // Before the database, so a new one gets its starter content in this language.
    warnings.extend(i18n::init(config.lang.as_deref()));

    // Open the database before taking over the terminal, so errors print normally.
    let db = Database::open_default()?;
    let mut app = App::new(db)?;
    app.close_orphan_sessions()?; // left open by a previous crash

    warnings.extend(app.init_theme(&dir, config.theme.clone()));
    warnings.extend(app.init_sort(config.sort.as_deref()));
    let (aliases, warning) = Aliases::load(&dir.join("commands.toml"));
    app.aliases = aliases;
    warnings.extend(warning);
    if !warnings.is_empty() {
        app.message = Some((warnings.join(" · "), MsgKind::Error));
    }

    let (tx, rx) = mpsc::channel();
    app.attach_tracker(tracker::spawn(tx.clone()));
    app.attach_events(tx.clone(), &config);
    event::spawn(tx);

    let mut terminal = ratatui::init(); // also installs the panic hook
    let result = app.run(&mut terminal, &rx);
    ratatui::restore();
    result
}
