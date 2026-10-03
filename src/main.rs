mod app;
mod core;
mod storage;
mod ui;

use app::App;

fn main() -> anyhow::Result<()> {
    let mut terminal = ratatui::init(); // also installs the panic hook
    let result = App::new().run(&mut terminal);
    ratatui::restore();
    result
}
