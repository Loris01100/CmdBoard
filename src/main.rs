mod app;
mod command;
mod config;
mod core;
mod event;
mod fuzzy;
mod launcher;
mod popup;
mod storage;
mod text_input;
mod tracker;
mod ui;

use std::sync::mpsc;

use app::{App, MsgKind};
use command::alias::Aliases;
use config::Config;
use storage::{Database, db::data_dir};

fn main() -> anyhow::Result<()> {
    // Open the database before taking over the terminal, so errors print normally.
    let db = Database::open_default()?;
    let mut app = App::new(db)?;
    app.close_orphan_sessions()?; // left open by a previous crash

    // Broken user files are reported, never fatal.
    let dir = data_dir()?;
    let mut warnings = Vec::new();
    let (config, warning) = Config::load(&dir.join("config.toml"));
    warnings.extend(warning);
    warnings.extend(app.init_theme(&dir, config.theme));
    let (aliases, warning) = Aliases::load(&dir.join("commands.toml"));
    app.aliases = aliases;
    warnings.extend(warning);
    if !warnings.is_empty() {
        app.message = Some((warnings.join(" · "), MsgKind::Error));
    }

    let (tx, rx) = mpsc::channel();
    app.attach_tracker(tracker::spawn(tx.clone()));
    event::spawn(tx);

    let mut terminal = ratatui::init(); // also installs the panic hook
    let result = app.run(&mut terminal, &rx);
    ratatui::restore();
    result
}
