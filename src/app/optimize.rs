//! Optimization screen: PC details, benchmarks run in a short-lived thread, and the
//! Windows gaming settings. Nothing is saved.

use std::collections::{HashMap, VecDeque};

use anyhow::{Context, bail};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::TableState;

use super::{App, MsgKind, Outcome, step};
use crate::command::Command;
use crate::event::AppEvent;
use crate::optimize::{self, Bench, Gaming, Score};

#[derive(Debug)]
pub struct OptimizeScreen {
    /// Read when the screen first opens.
    pub system: Option<optimize::System>,
    /// On or off, read each time the screen opens.
    pub gaming: Vec<(Gaming, bool)>,
    pub bench_state: TableState,
    pub gaming_state: TableState,
    /// The gaming settings have the focus instead of the benchmarks.
    pub gaming_focus: bool,
    /// Benchmarks run their heavy version.
    pub heavy: bool,
    pub running: Option<Bench>,
    /// Benchmarks still to run after `running`, for `a`.
    pub queue: VecDeque<Bench>,
    /// Last result of each benchmark, with whether it ran heavy.
    pub results: HashMap<Bench, (bool, Result<Score, String>)>,
}

impl Default for OptimizeScreen {
    fn default() -> Self {
        Self {
            system: None,
            gaming: Vec::new(),
            bench_state: TableState::default().with_selected(0),
            gaming_state: TableState::default().with_selected(0),
            gaming_focus: false,
            heavy: false,
            running: None,
            queue: VecDeque::new(),
            results: HashMap::new(),
        }
    }
}

impl OptimizeScreen {
    /// Reads what the screen shows: the PC once, the gaming settings each time.
    pub(super) fn open(&mut self) {
        if self.system.is_none() {
            self.system = Some(optimize::system());
        }
        self.gaming = Gaming::ALL.iter().map(|&g| (g, g.enabled())).collect();
    }

    pub(super) fn move_selection(&mut self, forward: bool) {
        let (state, len) = if self.gaming_focus {
            (&mut self.gaming_state, self.gaming.len())
        } else {
            (&mut self.bench_state, Bench::ALL.len())
        };
        state.select(step(state.selected(), len, forward));
    }

    pub fn on_finished(&mut self, bench: Bench, heavy: bool, result: Result<Score, String>) {
        self.running = None;
        self.results.insert(bench, (heavy, result));
    }

    fn selected_setting(&self) -> Option<Gaming> {
        Some(self.gaming.get(self.gaming_state.selected()?)?.0)
    }
}

impl App {
    /// Tab switches between benchmarks and gaming settings, Enter runs or switches, `a`
    /// runs every benchmark, `n` light/heavy, `o` the setting's Windows page.
    pub(super) fn optimize_key(&self, key: KeyEvent) -> Option<Command> {
        let screen = &self.optimize;
        Some(match key.code {
            KeyCode::Tab | KeyCode::BackTab => Command::ToggleFocus,
            KeyCode::Char('n') => Command::ToggleBenchLevel,
            KeyCode::Char('a') => Command::BenchAll,
            KeyCode::Enter if screen.gaming_focus => {
                Command::ToggleGaming(screen.selected_setting()?)
            }
            KeyCode::Char('o') if screen.gaming_focus => {
                Command::OpenGamingPage(screen.selected_setting()?)
            }
            KeyCode::Enter => Command::Bench(*Bench::ALL.get(screen.bench_state.selected()?)?),
            _ => return None,
        })
    }

    pub(super) fn bench(&mut self, bench: Bench) -> anyhow::Result<()> {
        if self.optimize.running.is_some() {
            bail!(t!("optimize.busy"));
        }
        self.start_bench(bench, self.optimize.heavy);
        Ok(())
    }

    /// Every benchmark in turn, the next one started when the last finishes.
    pub(super) fn bench_all(&mut self) -> anyhow::Result<()> {
        let [first, rest @ ..] = Bench::ALL;
        self.bench(first)?;
        if self.optimize.running.is_some() {
            self.optimize.queue = rest.into();
        }
        Ok(())
    }

    /// Records a result and starts the next queued benchmark, at the same level.
    pub(super) fn on_bench_finished(
        &mut self,
        bench: Bench,
        heavy: bool,
        result: Result<Score, String>,
    ) {
        self.optimize.on_finished(bench, heavy, result);
        if let Some(next) = self.optimize.queue.pop_front() {
            self.start_bench(next, heavy);
        }
    }

    fn start_bench(&mut self, bench: Bench, heavy: bool) {
        let started = self.spawn(move || AppEvent::BenchFinished {
            bench,
            heavy,
            result: optimize::run(bench, heavy),
        });
        if started {
            self.optimize.running = Some(bench);
        } else {
            self.optimize.queue.clear();
        }
    }

    /// Settings Windows does not let switch from here open their page instead.
    pub(super) fn toggle_gaming(&mut self, setting: Gaming) -> Outcome {
        if !setting.switchable() {
            return open_gaming_page(setting);
        }
        let on = !setting.enabled();
        setting.set(on).with_context(|| t!("optimize.set_failed"))?;
        self.optimize.open();
        let state = if on {
            t!("optimize.on")
        } else {
            t!("optimize.off")
        };
        let text = t!("optimize.switched", name = setting.label(), state);
        Ok(Some((text, MsgKind::Success)))
    }
}

pub(super) fn open_gaming_page(setting: Gaming) -> Outcome {
    opener::open(setting.page()).with_context(|| t!("optimize.open_failed"))?;
    Ok(None)
}
