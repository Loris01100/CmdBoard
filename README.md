# CmdBoard

A terminal dashboard for Windows: launch your apps and games from one place, track your play time, and earn XP, levels and rewards as you go.

<!-- Screenshot -->


## Features

- **Launcher**: apps grouped into categories, launched with `Enter`. Accepts an `.exe`, a `.lnk` shortcut, a name on the `PATH` or a URI (`steam://…`), with optional launch arguments. Up to nine favorites launch with `Alt+1`…`Alt+9` from any screen (`p` to pin), and a "Recent" row on top lists the apps played last. Adding an app suggests your installed Steam, Epic, GOG and Ubisoft Connect games, Microsoft Store and Xbox apps, and Start menu or desktop shortcuts.
- **Session tracking**: CmdBoard watches the app's process and records how long each session lasts, even if you launched the app from somewhere else. Time stops counting after 10 minutes without keyboard, mouse or controller input, and while the PC sleeps.
- **XP and rewards**: 1 XP per minute of play (sessions of 5 minutes or more), a bonus for daily streaks, a level for each app plus an overall profile level, and unlockable rewards.
- **Stats**: play time per app and per category.
- **Storage**: disk usage, installed programs sorted by size with their uninstallers, and a folder explorer that measures folder sizes.
- **Optimize**: quick CPU, memory and disk tests, plus toggles for Game Mode, Game Bar recording and GPU scheduling.
- **Command line** in a vim style (`:`), with history, Tab completion and your own aliases.
- **Themes**: Catppuccin (Latte, Frappé, Macchiato, Mocha) and an ANSI terminal theme. You can add your own.
- **Languages**: English, French and Portuguese, picked from Windows by default.

## Install

Requires Windows 10 or 11 (x86_64) and a modern terminal such as Windows Terminal.

**PowerShell**

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/Loris01100/CmdBoard/releases/latest/download/cmdboard-installer.ps1 | iex"
```

**MSI**: download `cmdboard-x86_64-pc-windows-msvc.msi` from the [latest release](https://github.com/Loris01100/CmdBoard/releases/latest).

**From source**

```powershell
cargo install --git https://github.com/Loris01100/CmdBoard
```

Then run `cmdboard`.

### Updating

Run `:update` inside CmdBoard. If you installed it with the MSI, run the new MSI instead (or `winget upgrade CmdBoard` once it is on winget). CmdBoard checks for a new version once a day and tells you in the status bar. Updates keep your data.

## Keys

| Key | Action |
|---|---|
| `↑↓` / `j k` | Navigate |
| `←→` / `Tab` | Switch panel (categories ↔ apps) |
| `Enter` | Launch the selected app |
| `/` | Search apps in every category |
| `:` | Command line |
| `a` / `e` / `m` / `d` | Add / edit / move / remove (an app, or an empty category) |
| `s` | Change the sort order (name, XP, recent, time) |
| `1`–`5` | Dashboard, Stats, Rewards, Storage, Optimize |
| `0` / `?` | Help |

Each screen lists its own keys in the help (`?`).

## Commands

| Command | Description |
|---|---|
| `:launch <app>` (`:l`) | Launch an app |
| `:add [<name> <target> [category]]` | Add an app (no arguments: opens the form) |
| `:move <app> [category]` (`:mv`) | Move an app to another category |
| `:edit <app>` | Change an app's name, target, category or process, keeping its history |
| `:rm <app>` | Remove an app and its history |
| `:rmcat <category>` | Remove an empty category |
| `:stats [app]` | Stats for every app, or one |
| `:clear sessions\|stats` | Clear the session history, or all stats |
| `:theme [name]` | Switch theme (no name: list themes) |
| `:sort [name\|xp\|recent\|time]` | Sort order of the apps |
| `:lang [en\|fr\|pt]` | Switch language |
| `:group <name> <app>, <app>…` | Create an alias `:<name>` that launches these apps |
| `:export [file]` / `:import <file>` | Back up apps and sessions to JSON / merge a backup |
| `:uninstall <program>` | Start a program's uninstaller |
| `:update` | Install the latest version |
| `:help [command]` (`:h`, `:?`) | Help, or a command's usage |
| `:quit` (`:q`) | Quit |

Destructive commands always ask for confirmation, and so does `:export` when the file already exists. Paths with spaces go in double quotes: `:add Notes "C:\Program Files\Notes\notes.exe" Tools`.

## Configuration

Everything lives in `%APPDATA%\CmdBoard`:

| File | Content |
|---|---|
| `cmdboard.db` | Your apps, sessions, XP and rewards (SQLite) |
| `config.toml` | Theme, sort order, language, `update_check = false` to turn off the update check, `idle_minutes` (default 10, `0` to always count play time) |
| `commands.toml` | Your aliases |
| `themes\*.toml` | Your own themes. One with the same name as a built-in theme replaces it |

Example `commands.toml`, where `$1`…`$9` are arguments and `$*` is all of them:

```toml
[alias]
gaming = "launch steam; launch discord"
play   = "launch $1; stats $1"
```

To write a theme, copy one of the files in [themes/](themes/) into `%APPDATA%\CmdBoard\themes` and edit it.

## Building

```powershell
cargo run
cargo test
```

Design notes (in French) are in [docs/plan_conception_tui_ratatui.md](docs/plan_conception_tui_ratatui.md).

## License

[MIT](LICENSE)
