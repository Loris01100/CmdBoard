//! Session tracker thread: polls running processes with `sysinfo` and reports when a
//! watched app starts or stops, and how long it has really been played (plan section 12).
//! It also reports which library games not added to `CmdBoard` are running, so the
//! dashboard can suggest adding them.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::OsStr;
use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError, SendError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows_sys::Win32::UI::Input::XboxController::{XINPUT_STATE, XInputGetState};

use crate::event::AppEvent;

pub const POLL: Duration = Duration::from_secs(3);

/// Longest gap between two polls that still counts as play. A longer one means the PC
/// slept or the thread stalled, and the game was not being played meanwhile.
const MAX_STEP: Duration = Duration::from_secs(10);

/// An app to track and the exe name of its process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watched {
    pub app_id: i64,
    pub exe: String,
}

/// What the tracker looks for: the apps to track, and the exes of library games not
/// added yet, only reported while running (`AppEvent::UnaddedRunning`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchList {
    pub apps: Vec<Watched>,
    pub unadded: Vec<String>,
}

impl From<Vec<Watched>> for WatchList {
    fn from(apps: Vec<Watched>) -> Self {
        Self {
            apps,
            unadded: Vec::new(),
        }
    }
}

/// Starts the tracker. Send it the full watch list whenever apps change; it stops
/// once either channel is closed. Play time stops counting once the user has not touched
/// the keyboard, mouse or a controller for `idle_limit` (`None`: always counts).
/// `resumed` holds the sessions already running, by app id, with the time they were
/// played before: they go on without a new `SessionStarted`.
pub fn spawn(
    events: Sender<AppEvent>,
    idle_limit: Option<Duration>,
    resumed: HashMap<i64, Duration>,
) -> Sender<WatchList> {
    let (watch_tx, watch_rx) = mpsc::channel::<WatchList>();
    thread::spawn(move || {
        let mut system = System::new();
        let mut watched = Index::default();
        let mut unadded_running = BTreeSet::new();
        let mut sessions = Sessions {
            events,
            idle_limit,
            input: Input::default(),
            played: resumed,
            last_poll: Instant::now(),
        };
        loop {
            match watch_rx.recv_timeout(POLL) {
                Ok(list) => watched = Index::new(&watch_rx.try_iter().last().unwrap_or(list)),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            // Nothing to look for: the processes are not listed, and running sessions end.
            let now_running = if watched.is_empty() {
                Running::default()
            } else {
                poll_running(&mut system, &watched)
            };
            if sessions.update(&now_running.apps).is_err() {
                return;
            }
            if now_running.unadded != unadded_running {
                let exes = now_running.unadded.iter().cloned().collect();
                if sessions
                    .events
                    .send(AppEvent::UnaddedRunning(exes))
                    .is_err()
                {
                    return;
                }
                unadded_running = now_running.unadded;
            }
        }
    });
    watch_tx
}

/// Running sessions of the tracker thread and the time each has been played.
struct Sessions {
    events: Sender<AppEvent>,
    idle_limit: Option<Duration>,
    input: Input,
    played: HashMap<i64, Duration>,
    last_poll: Instant,
}

impl Sessions {
    /// Credits the time since the last poll, then reports ended, ongoing and new
    /// sessions. Fails once the UI has closed the event channel.
    fn update(&mut self, now_running: &HashSet<i64>) -> Result<(), SendError<AppEvent>> {
        let now = Instant::now();
        let step = now - self.last_poll;
        self.last_poll = now;
        // Input is only read while something is played.
        let idle = self
            .idle_limit
            .is_some_and(|limit| !self.played.is_empty() && self.input.idle_for() >= limit);
        for time in self.played.values_mut() {
            *time += credit(step, idle);
        }

        let previous: HashSet<i64> = self.played.keys().copied().collect();
        let (begun, ended) = diff(&previous, now_running);
        for app_id in ended {
            let secs = self.played.remove(&app_id).map_or(0, |t| t.as_secs());
            self.events.send(AppEvent::SessionEnded { app_id, secs })?;
        }
        for (&app_id, &played) in &self.played {
            let progress = AppEvent::SessionProgress {
                app_id,
                played,
                idle,
            };
            self.events.send(progress)?;
        }
        for app_id in begun {
            self.played.insert(app_id, Duration::ZERO);
            self.events.send(AppEvent::SessionStarted { app_id })?;
        }
        Ok(())
    }
}

/// Watched apps running right now, read once: at startup, to resume the sessions the
/// other `CmdBoard` process handed over.
pub fn running_now(watched: &[Watched]) -> HashSet<i64> {
    let list = WatchList::from(watched.to_vec());
    poll_running(&mut System::new(), &Index::new(&list)).apps
}

fn poll_running(system: &mut System, watched: &Index) -> Running {
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let names = system
        .processes()
        .values()
        .map(|p| p.name().to_string_lossy());
    running(watched, names)
}

/// Play time a poll adds to running sessions: the time since the previous poll, capped
/// by `MAX_STEP`, or nothing while the user is idle.
fn credit(step: Duration, idle: bool) -> Duration {
    if idle {
        Duration::ZERO
    } else {
        step.min(MAX_STEP)
    }
}

/// When the user last used the keyboard, the mouse or an `XInput` controller (Xbox pads,
/// and most others through Steam Input). Controllers are not seen by `GetLastInputInfo`.
#[derive(Debug)]
struct Input {
    /// Last state number of each controller slot, `None` when unplugged.
    pads: [Option<u32>; 4],
    last_pad_input: Instant,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            pads: [None; 4],
            last_pad_input: Instant::now(),
        }
    }
}

impl Input {
    fn idle_for(&mut self) -> Duration {
        for (slot, last) in (0..).zip(self.pads.iter_mut()) {
            let packet = pad_packet(slot);
            // The state number changes with every button or stick move.
            if packet.is_some() && packet != *last {
                self.last_pad_input = Instant::now();
            }
            *last = packet;
        }
        keyboard_mouse_idle().min(self.last_pad_input.elapsed())
    }
}

/// Time since the last keyboard or mouse input in this Windows session.
fn keyboard_mouse_idle() -> Duration {
    let mut info = LASTINPUTINFO {
        cbSize: u32::try_from(size_of::<LASTINPUTINFO>()).unwrap_or(0),
        dwTime: 0,
    };
    // SAFETY: `info` is a valid LASTINPUTINFO with its size set.
    if unsafe { GetLastInputInfo(&raw mut info) } == 0 {
        return Duration::ZERO; // unknown: count the time rather than lose it
    }
    // SAFETY: no arguments. Both are in the same wrapping millisecond counter.
    let now = unsafe { GetTickCount() };
    Duration::from_millis(now.wrapping_sub(info.dwTime).into())
}

/// State number of the controller in `slot` (0 to 3), `None` when none is plugged in.
fn pad_packet(slot: u32) -> Option<u32> {
    // SAFETY: XINPUT_STATE is plain data, filled by the call.
    let mut state: XINPUT_STATE = unsafe { std::mem::zeroed() };
    // SAFETY: `state` is a valid output for the call.
    let status = unsafe { XInputGetState(slot, &raw mut state) };
    (status == 0).then_some(state.dwPacketNumber)
}

/// A watch list by `exe_key`, built once per list rather than at every poll.
#[derive(Debug, Default)]
struct Index {
    apps: HashMap<String, Vec<i64>>,
    unadded: HashSet<String>,
}

impl Index {
    fn new(list: &WatchList) -> Self {
        let mut apps: HashMap<String, Vec<i64>> = HashMap::new();
        for w in &list.apps {
            apps.entry(exe_key(&w.exe)).or_default().push(w.app_id);
        }
        let unadded = list.unadded.iter().map(|exe| exe_key(exe)).collect();
        Self { apps, unadded }
    }

    fn is_empty(&self) -> bool {
        self.apps.is_empty() && self.unadded.is_empty()
    }
}

/// What one poll found running.
#[derive(Debug, Default, PartialEq, Eq)]
struct Running {
    apps: HashSet<i64>,
    /// `exe_key`s of library games not added.
    unadded: BTreeSet<String>,
}

/// File name of `exe`, lowercased: `watch_exe` may hold a full path. Process names are
/// matched on it.
pub fn exe_key(exe: &str) -> String {
    let exe = exe.trim();
    let name = Path::new(exe)
        .file_name()
        .map_or(exe.into(), OsStr::to_string_lossy);
    // Lowercased like process names (`lowercase_into`), so both always match.
    name.chars().flat_map(char::to_lowercase).collect()
}

/// `text` lowercased into `buffer`, reused so process names allocate nothing.
fn lowercase_into<'a>(buffer: &'a mut String, text: &str) -> &'a String {
    buffer.clear();
    buffer.extend(text.chars().flat_map(char::to_lowercase));
    buffer
}

/// Apps with at least one process running, and library games not added that run,
/// matched on exe name ignoring case.
fn running<S: AsRef<str>>(watched: &Index, process_names: impl IntoIterator<Item = S>) -> Running {
    let mut running = Running::default();
    let mut buffer = String::new();
    for name in process_names {
        let key = lowercase_into(&mut buffer, name.as_ref());
        if let Some(ids) = watched.apps.get(key) {
            running.apps.extend(ids);
        }
        if watched.unadded.contains(key) {
            running.unadded.insert(key.clone());
        }
    }
    running
}

/// Apps that started and apps that stopped between two polls, sorted.
fn diff(previous: &HashSet<i64>, current: &HashSet<i64>) -> (Vec<i64>, Vec<i64>) {
    let mut begun: Vec<i64> = current.difference(previous).copied().collect();
    let mut ended: Vec<i64> = previous.difference(current).copied().collect();
    begun.sort_unstable();
    ended.sort_unstable();
    (begun, ended)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_reports_start_and_end_of_a_running_exe() {
        // This test binary is a running process the tracker can watch.
        let exe = std::env::current_exe().unwrap();
        let exe = exe.file_name().unwrap().to_string_lossy().into_owned();
        let (events, rx) = mpsc::channel();
        let tracker = spawn(events, None, HashMap::new());
        tracker.send(vec![watched(7, &exe)].into()).unwrap();
        let wait = Duration::from_secs(30);
        assert!(matches!(
            rx.recv_timeout(wait).unwrap(),
            AppEvent::SessionStarted { app_id: 7 }
        ));
        tracker.send(WatchList::default()).unwrap();
        // Progress reports may come first.
        let ended = std::iter::from_fn(|| rx.recv_timeout(wait).ok())
            .find(|e| !matches!(e, AppEvent::SessionProgress { .. }));
        assert!(matches!(
            ended,
            Some(AppEvent::SessionEnded { app_id: 7, .. })
        ));
    }

    #[test]
    fn resumed_session_goes_on_from_its_time_played() {
        let exe = std::env::current_exe().unwrap();
        let exe = exe.file_name().unwrap().to_string_lossy().into_owned();
        let list = vec![watched(7, &exe)];
        assert!(running_now(&list).contains(&7));

        let (events, rx) = mpsc::channel();
        let before = Duration::from_secs(600);
        let tracker = spawn(events, None, HashMap::from([(7, before)]));
        tracker.send(list.into()).unwrap();
        let wait = Duration::from_secs(30);
        // No new start: the first report is the progress of the resumed session.
        match rx.recv_timeout(wait).unwrap() {
            AppEvent::SessionProgress { app_id, played, .. } => {
                assert_eq!(app_id, 7);
                assert!(played >= before);
            }
            other => panic!("{other:?}"),
        }
        tracker.send(WatchList::default()).unwrap();
        let ended = std::iter::from_fn(|| rx.recv_timeout(wait).ok())
            .find(|e| !matches!(e, AppEvent::SessionProgress { .. }));
        assert!(matches!(
            ended,
            Some(AppEvent::SessionEnded { app_id: 7, secs }) if secs >= 600
        ));
    }

    fn watched(app_id: i64, exe: &str) -> Watched {
        Watched {
            app_id,
            exe: exe.into(),
        }
    }

    #[test]
    fn matches_exe_name_ignoring_case_and_path() {
        let list = [
            watched(1, "Hades.exe"),
            watched(2, r"C:\Program Files\VS Code\Code.EXE"),
            watched(3, "steam.exe"),
            watched(4, "ÉPOPÉE.exe"),
        ];
        let names = ["hades.exe", "code.exe", "explorer.exe", "Épopée.EXE"];
        let index = Index::new(&list.to_vec().into());
        assert_eq!(running(&index, names).apps, HashSet::from([1, 2, 4]));
    }

    #[test]
    fn several_instances_count_once_and_apps_can_share_an_exe() {
        let list = [watched(1, "steam.exe"), watched(2, "steam.exe")];
        let index = Index::new(&list.to_vec().into());
        let found = running(&index, ["steam.exe", "steam.exe"]);
        assert_eq!(found.apps, HashSet::from([1, 2]));
    }

    #[test]
    fn unadded_games_are_reported_apart_from_apps() {
        let list = WatchList {
            apps: vec![watched(1, "steam.exe")],
            unadded: vec![
                r"D:\Games\ELDEN RING\eldenring.exe".into(),
                "hades.exe".into(),
            ],
        };
        let found = running(&Index::new(&list), ["EldenRing.exe", "steam.exe"]);
        assert_eq!(
            found,
            Running {
                apps: HashSet::from([1]),
                unadded: BTreeSet::from(["eldenring.exe".into()]),
            }
        );
        // Unadded games alone are enough to list the processes.
        assert!(Index::new(&WatchList::default()).is_empty());
        let only_unadded = WatchList {
            apps: Vec::new(),
            unadded: list.unadded,
        };
        assert!(!Index::new(&only_unadded).is_empty());
    }

    #[test]
    fn play_time_skips_idle_and_sleep() {
        let secs = Duration::from_secs;
        assert_eq!(credit(secs(3), false), secs(3));
        assert_eq!(credit(secs(3), true), Duration::ZERO);
        assert_eq!(credit(secs(8 * 3600), false), MAX_STEP); // woke from sleep
    }

    #[test]
    fn input_idle_time_is_readable() {
        // Reads the keyboard, mouse and controllers: must not panic, whatever is plugged in.
        Input::default().idle_for();
    }

    #[test]
    fn diff_reports_starts_and_ends() {
        let previous = HashSet::from([1, 2]);
        let current = HashSet::from([2, 4, 3]);
        assert_eq!(diff(&previous, &current), (vec![3, 4], vec![1]));
        assert_eq!(diff(&current, &current), (vec![], vec![]));
    }
}
