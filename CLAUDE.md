# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

CmdBoard is a Windows-only terminal dashboard (Rust + ratatui) for launching app shortcuts grouped by category. It tracks play/usage sessions and awards XP, levels and rewards for them. The design document, [docs/plan_conception_tui_ratatui.md](docs/plan_conception_tui_ratatui.md) (in French), is the source of truth for architecture, data model, keybindings, theming, distribution and the build order. Read it before implementing a feature. If the code ends up diverging from the plan, update the plan in the same change.

Development follows the 12 numbered steps in section 16 of the plan. Each step has a verifiable deliverable. Implement them in order. Steps 1–11 are done: the dashboard reads from SQLite (`%APPDATA%\CmdBoard\cmdboard.db`, seeded with starter apps on first run), every key maps to a `Command`, the `:` command line (`launch`, `add`, `move`, `rm`, `rmcat`, `xp`, `stats`, `theme`, `help`, `quit`, with history and Tab completion from `command/complete.rs`) runs through the same `App::execute`, and `a`/`m`/`d` open add/move forms and delete confirmations (`Mode::Popup`, state in `src/popup.rs`). `App::run` consumes `AppEvent`s from the event thread (`src/event.rs`, keys + 250 ms `Tick`) and the session tracker (`src/tracker.rs`, `sysinfo` every 3 s). Sessions are inserted open, checkpointed every 60 s, closed on end (dropped under 60 s), and orphans are closed at startup. The header shows a live timer. Closed sessions earn XP (`core::xp::xp_for_session`, saved to `sessions.xp_gained` and `apps.total_xp`), and so do sessions closed on quit and orphans. XP bars animate from `App::xp_anims`/`profile_anim` over `frame_count`, a `Popup::LevelUp` opens on level-up (queued in `pending_popups` while the user is typing), and `:xp <app> <amount>` adjusts XP by hand. Rewards are rows in `rewards` (starter set inserted by migration v2) whose `rule` uses the small condition language in `core/rewards.rs`; after each recorded session, `App::unlock_rewards` evaluates the pending ones against `Database::session_facts` and queues a `Popup::RewardUnlocked` per unlock. `scope = 'app'` rewards unlock once per app (`unlocked_rewards.app_id`). The Rewards screen (`ui/screens/rewards.rs`) lists them with its own `reward_state`. The Stats screen (`ui/screens/stats.rs`) shows `App::stats` (`Database::stats`, optionally filtered by `:stats <app>`). `/` enters `Mode::Search`: `visible_apps()` then returns fuzzy matches (`src/fuzzy.rs`) among all apps, and Enter runs `Command::Select`. Aliases from `%APPDATA%\CmdBoard\commands.toml` (`command/alias.rs`) expand in `App::run_line` and run one command at a time through `run_command`. Themes are TOML files (`themes/`, embedded; user ones in `%APPDATA%\CmdBoard\themes\`) loaded by `ui::theme::load`; `App::init_theme` applies the one from `config.toml` (`src/config.rs`) or the truecolor default, and `:theme <name>` switches and saves it. `ui/layout.rs` stacks the dashboard panels under 60 columns and drops the profile/recent-rewards panels on short terminals. Step 12 is in progress. `dist` is set up (`dist-workspace.toml`, `.github/workflows/release.yml`, `wix/main.wxs`). `:update` (`src/update.rs`) runs `self_update` in a short-lived thread that answers with `AppEvent::UpdateFinished`. Under Program Files it doesn't replace the exe and points to winget instead. A passive check runs at most once a day (`update_check`/`last_update_check` in `config.toml`), and when a newer version exists the status bar shows `vX.Y disponible`. What's left is publishing `v0.1.0` and the winget manifest.

## Commands

- `cargo run`: launch the TUI
- `cargo build`, `cargo clippy --all-targets`, `cargo fmt`. A pre-commit hook (`.githooks/pre-commit`, enable with `git config core.hooksPath .githooks`) rejects unformatted commits, and CI (`.github/workflows/ci.yml`) runs fmt, clippy and tests.
- `cargo test`: all tests. `cargo test <name_substring>` runs a single test, `cargo test --lib core::xp` runs a single module.
- Add dependencies with `cargo add <crate>` to get current versions. The versions listed in the plan (section 17) are only indicative.
- Releases (plan section 18): `dist plan` previews the release. Bump `version` in `Cargo.toml`, then push a `vX.Y.Z` tag; GitHub Actions builds the zip, MSI and PowerShell installer and publishes the GitHub Release. Don't tag or push without being asked.

## Architecture rules (from the plan)

- **Single state, Elm-style**: all state lives in `App` (`src/app.rs`) and only `update`/`on_*` handlers mutate it. `ui::draw(frame, &app)` is pure: it reads state and must not mutate it. Widgets keep no state of their own. Everything, including `ListState` and `TableState`, lives in `App`.
- **Everything goes through `Command`**: keypresses, the `:` command line and `commands.toml` aliases are all translated into the `Command` enum and run through one execution path. Don't add a code path that bypasses it. This includes `:update` and `:theme`. A new command needs a `Command` variant, an entry in the `COMMANDS` table (`command/mod.rs`, which feeds aliases, usage errors, `:help` and the Help screen) and a parser arm. The parser splits arguments itself (double quotes group, backslashes kept); don't switch to `shell-words`, which mangles Windows paths. Destructive commands carry a `confirmed: bool`; unconfirmed, executing them opens a `Popup::Confirm` holding the confirmed copy, so keys and `:` commands share the same confirmation. Forms build a `Command` and run it through `run_command`; errors stay inside the open form.
- **No business logic in `ui/`**: XP formulas (`core/xp.rs`), the rewards rule engine (`core/rewards.rs`), SQLite (`storage/`), launching/scanning (`launcher/`) and updating (`update.rs`) must stay testable without a terminal.
- **Threads**: three permanent ones. The main UI thread, the event thread (keyboard plus a ~250 ms `Tick`) and the session tracker (polls `sysinfo` every 2–5 s). They communicate via `mpsc` using `AppEvent`. One-off network work (the update check/download) runs in a short-lived thread that reports back through the same channel (`AppEvent::UpdateFinished`). Never block the UI thread on I/O.
- **Rewards are data**: rewards are rule definitions stored in the DB/JSON (`rewards.rule`), not hardcoded. They are evaluated after each session ends.
- `apps.launch_target` (exe path or URI such as `steam://...`) and `apps.watch_exe` (the process to track) are deliberately separate fields, because launchers like Steam, Epic and Battle.net spawn a different process.

## Theme (plan section 10)

- Every color comes from `Theme` (`ui/theme.rs`). Never hardcode colors in widgets.
- Theme files have two layers: `[palette]` (raw colors, copied from the official Catppuccin palette) and `[slots]` (semantic roles such as `border_focused = "sapphire"`, mapping one-to-one to `Theme` fields). Slots may reference a palette name, a `#rrggbb` hex or an ANSI color name. Code only reads slots; the palette is resolved at load time and not kept.
- Built-in themes (`themes/catppuccin-{latte,frappe,macchiato,mocha}.toml` and `themes/terminal.toml`) are embedded with `include_str!` and parsed by the same loader as user themes in `%APPDATA%\CmdBoard\themes\`. Don't add the `catppuccin` crate.
- Default is `catppuccin-mocha` when truecolor is detected (`WT_SESSION` set, or `COLORTERM` is `truecolor`/`24bit`), otherwise `terminal` (16 ANSI colors only).
- A broken theme file must produce an error message and keep the current theme, never panic.

## Distribution and updates (plan section 18)

- Built with `dist`, Windows target only (`x86_64-pc-windows-msvc`), installers `powershell` and `msi`, `install-updater = false`.
- Never change the WiX `upgrade-guid` / `path-guid` in `Cargo.toml`: doing so breaks MSI upgrades.
- `:update` uses `self_update` against GitHub Releases. If the exe lives under `Program Files` (MSI or winget install), it must not self-replace; it tells the user to run `winget upgrade CmdBoard` instead.
- **DB migrations**: the schema is versioned with `PRAGMA user_version` and migrated at startup. Never edit a migration that has shipped; add a new one. User data in `%APPDATA%` must survive every update.

## Windows-specific pitfalls

- crossterm on Windows emits both `Press` and `Release`. Always filter on `KeyEventKind::Press`, or every key fires twice.
- Install a panic hook that restores the terminal (`ratatui::init()` provides one).
- On startup, close orphaned sessions left over from a previous crash.
- Store the DB, `config.toml` and user themes in `%APPDATA%\CmdBoard` via the `directories` crate. Scan `.lnk` shortcuts from the user and ProgramData Start Menu folders and the Desktop.
- Terminals can't show real icons. Use Nerd Font glyphs or emoji.
- Linux is out of scope. Don't add cross-platform abstractions for it.

## Testing conventions

- `core/` and `command/parser`: plain unit tests. The parser uses table-driven tests (input string → `Command`).
- `storage/`: `rusqlite::Connection::open_in_memory()`. Test that migrations bring an empty DB and every older `user_version` up to the latest schema.
- `ui/theme`: test that every built-in theme parses and resolves all slots.
- `ui/`: ratatui `TestBackend`, or `insta` snapshots.
