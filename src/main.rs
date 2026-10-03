mod app;
mod command;
mod core;
mod launcher;
mod storage;
mod ui;

use app::App;
use storage::Database;

fn main() -> anyhow::Result<()> {
    // Open the database before taking over the terminal, so errors print normally.
    let mut app = App::new(Database::open_default()?)?;
    let mut terminal = ratatui::init(); // also installs the panic hook
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
}
