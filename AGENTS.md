# Repository instructions

CmdBoard is a Windows-only Rust/ratatui dashboard for launching apps, tracking usage, and awarding XP and rewards.

## Source of truth

Read [the design plan](docs/plan_conception_tui_ratatui.md) before implementing a feature. Update it in the same change when behavior, architecture, keybindings, or data diverge. Sections 1–7 cover state and commands, 8–10 the UI, 11–12 sessions and rewards, 15 tests, and 18 distribution. The plan is written in French; write updates to it in French.

Steps 1–11 are complete; step 12 is in progress. v0.1.0 and v0.2.0 are released; the winget release job is in place and only the first winget-pkgs submission remains. Check the plan and code for current implementation details rather than duplicating a feature inventory here.

## Architecture

- State belongs in App, including selections, table/list state and animations. Handlers mutate it; draw(frame, &app) only reads it.
- Keys, command-line input and aliases run through Command and App::execute. A new command-line action also needs a COMMANDS entry, parser arm, completion where relevant, and translated help. Key-only navigation actions need a variant and key mapping but no invented command-line syntax.
- Destructive commands carry confirmed: bool and open Popup::Confirm before executing. Forms produce commands; errors stay in the form.
- Keep business logic out of ui/: XP and reward rules in core/, SQLite in storage/, scanning/launching in launcher/, updating in update.rs.
- Three permanent threads (UI, events, tracker) communicate through mpsc/AppEvent. Temporary scanning/network work reports through the same event channel. Do not block UI handlers on network or scanning.
- Rewards are data-driven rules. launch_target and watch_exe are deliberately separate.
- Layers depend only inwards: domain (core/) ← infrastructure (storage/, launcher/, tracker, update…) ← application (app/, command/, popup) ← presentation (ui/). tests/architecture.rs enforces it; classify any new module there. Plan section 2 has the table.
- App is split by feature under app/ (keys, commands, library, forms, sessions, settings, one file per screen with its own state struct). Add a feature to its module, not to app/mod.rs.
- Size limits: at most 30 functions and 600 lines per file (tests excluded, tests/architecture.rs) and 100 lines per function (clippy too_many_lines). Split by feature when a file grows; only long table-driven tests may use #[expect(clippy::too_many_lines, reason = "…")].
- Closing a session, its XP and its rewards is one storage transaction (Database::close_session). Keep multi-step writes that must stay consistent inside one transaction in storage/.

## Windows and persisted data

- Windows only; do not introduce Linux portability layers.
- Filter crossterm events on KeyEventKind::Press. Preserve terminal restoration on panic.
- Store user data through directories under %APPDATA%\CmdBoard. Close orphaned sessions at startup and preserve session checkpointing and XP attribution.
- Never edit shipped migrations: add a migration using PRAGMA user_version. Updates must preserve user data.
- Preserve Windows paths in the custom command parser: double quotes group arguments, backslashes remain literal. Do not replace it with shell-words.

## UI, themes and translations

- Every widget color comes from Theme. TOML palettes resolve into semantic slots; built-in and user themes share the loader. Keep terminal ANSI-only and keep the current theme when loading a replacement fails.
- Preserve compatibility of user themes; caution is optional and falls back to warning.
- All UI text uses t!; add keys and matching placeholders to every locale. en.toml is the reference.
- Tests use the default French language. Do not change the global language in parallel tests. Starter rewards use starter_rewards.<code> translations.
- For UI, theme or translation changes, read the local [cmdboard-ui skill](.agents/skills/cmdboard-ui/SKILL.md). Claude users should read the same file directly; no separate copy is needed.

## Validation

Run cargo fmt --check, cargo clippy --all-targets -- -D warnings and appropriate cargo test checks before handing off code changes. CI runs clippy with -D warnings, so any warning fails the build. Clippy enables the pedantic group in Cargo.toml: fix a lint rather than silencing it, or use a local #[expect(clippy::…, reason = "…")]. Prefer try_from over `as` for narrowing casts. The pre-commit hook only checks formatting and needs git config core.hooksPath .githooks once per clone; CI runs formatting, clippy and tests.

Use table-driven parser tests, in-memory SQLite tests (including migration upgrades), built-in theme parsing tests, and ratatui TestBackend for rendering. Verify behavior and boundary cases rather than duplicating implementation details in assertions.

Useful commands: cargo run, cargo build, cargo test <name_substring>, and cargo run --example keys to print raw crossterm key events when debugging keyboard layouts. Coverage uses cargo llvm-cov --lcov --output-path target/lcov.info followed by cargo sonar-scanner. Add dependencies with cargo add; plan versions are indicative.

Commit messages use a lowercase type, a space before the colon and a short summary: feat : …, fix : …, chore : …, docs : …, release : ….

## Releases

Read plan section 18 for release work. dist plan previews artifacts; distribution targets x86_64-pc-windows-msvc with PowerShell and MSI installers and install-updater = false.

Never change WiX upgrade-guid or path-guid. Under Program Files, :update must direct users to winget instead of replacing the executable. Do not tag, push or publish without an explicit request.
