# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

CmdBoard is a Windows-only terminal dashboard (Rust + ratatui) for launching app shortcuts grouped by category. It tracks play/usage sessions and awards XP, levels and rewards for them. The repo currently contains only the design document, [docs/plan_conception_tui_ratatui.md](docs/plan_conception_tui_ratatui.md) (in French), which is the source of truth for architecture, data model, keybindings and the build order. Read it before implementing a feature. If the code ends up diverging from the plan, update the plan in the same change.

Development follows the 11 numbered steps in section 16 of the plan. Each step has a verifiable deliverable. Implement them in order.

## Commands

Standard Cargo (once the crate exists):

- `cargo run`: launch the TUI
- `cargo build`, `cargo clippy --all-targets`, `cargo fmt`
- `cargo test`: all tests. `cargo test <name_substring>` runs a single test, `cargo test --lib core::xp` runs a single module.
- Add dependencies with `cargo add <crate>` to get current versions. The versions listed in the plan (section 17) are only indicative.

## Architecture rules (from the plan)

- **Single state, Elm-style**: all state lives in `App` (`src/app.rs`) and only `update`/`on_*` handlers mutate it. `ui::draw(frame, &app)` is pure: it reads state and must not mutate it. Widgets keep no state of their own. Everything, including `ListState` and `TableState`, lives in `App`.
- **Everything goes through `Command`**: keypresses, the `:` command line and `commands.toml` aliases are all translated into the `Command` enum and run through one execution path. Don't add a code path that bypasses it.
- **No business logic in `ui/`**: XP formulas (`core/xp.rs`), the rewards rule engine (`core/rewards.rs`), SQLite (`storage/`) and launching/scanning (`launcher/`) must stay testable without a terminal.
- **Threads**: there are three. The main UI thread, the event thread (keyboard plus a ~250 ms `Tick`) and the session tracker (polls `sysinfo` every 2–5 s). They communicate via `mpsc` using `AppEvent`.
- **Rewards are data**: rewards are rule definitions stored in the DB/JSON (`rewards.rule`), not hardcoded. They are evaluated after each session ends.
- **Theme**: every color comes from `Theme` (`ui/theme.rs`, loaded from `themes/*.toml`). Never hardcode colors in widgets.
- `apps.launch_target` (exe path or URI such as `steam://...`) and `apps.watch_exe` (the process to track) are deliberately separate fields, because launchers like Steam, Epic and Battle.net spawn a different process.

## Windows-specific pitfalls

- crossterm on Windows emits both `Press` and `Release`. Always filter on `KeyEventKind::Press`, or every key fires twice.
- Install a panic hook that restores the terminal (`ratatui::init()` provides one).
- On startup, close orphaned sessions left over from a previous crash.
- Store the DB in `%APPDATA%` via the `directories` crate. Scan `.lnk` shortcuts from the user and ProgramData Start Menu folders and the Desktop.
- Terminals can't show real icons. Use Nerd Font glyphs or emoji.

## Testing conventions

- `core/` and `command/parser`: plain unit tests. The parser uses table-driven tests (input string → `Command`).
- `storage/`: `rusqlite::Connection::open_in_memory()`.
- `ui/`: ratatui `TestBackend`, or `insta` snapshots.
