//! Prints every key event crossterm receives, to debug keyboard layouts and terminals.
//! Run with `cargo run --example keys`, press keys, Esc to quit.

use crossterm::{
    event::{self, Event, KeyCode},
    terminal,
};

fn main() -> std::io::Result<()> {
    terminal::enable_raw_mode()?;
    print!("Appuyez sur des touches (Esc pour quitter)\r\n");
    loop {
        let event = event::read()?;
        print!("{event:?}\r\n");
        if let Event::Key(key) = event
            && key.code == KeyCode::Esc
        {
            break;
        }
    }
    terminal::disable_raw_mode()
}
