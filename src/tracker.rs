//! Session tracker thread: polls running processes with `sysinfo` and reports when a
//! watched app starts or stops, and how long it has really been played (plan section 12).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
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

/// Starts the tracker. Send it the full watch list whenever apps change; it stops
/// once either channel is closed. Play time stops counting once the user has not touched
/// the keyboard, mouse or a controller for `idle_limit` (`None`: always counts).
pub fn spawn(events: Sender<AppEvent>, idle_limit: Option<Duration>) -> Sender<Vec<Watched>> {
    let (watch_tx, watch_rx) = mpsc::channel::<Vec<Watched>>();
    thread::spawn(move || {
        let mut system = System::new();
        let mut input = Input::default();
        let mut watched = Vec::new();
        let mut played: HashMap<i64, Duration> = HashMap::new();
        let mut last_poll = Instant::now();
        loop {
            match watch_rx.recv_timeout(POLL) {
                Ok(list) => watched = watch_rx.try_iter().last().unwrap_or(list),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            system.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing(),
            );
            let names = system
                .processes()
                .values()
                .map(|p| p.name().to_string_lossy());
            let now_running = running_apps(&watched, names);

            // Input is only read while something is played.
            let now = Instant::now();
            let step = now - last_poll;
            last_poll = now;
            let idle =
                idle_limit.is_some_and(|limit| !played.is_empty() && input.idle_for() >= limit);
            for time in played.values_mut() {
                *time += credit(step, idle);
            }

            let previous: HashSet<i64> = played.keys().copied().collect();
            let (begun, ended) = diff(&previous, &now_running);
            for app_id in ended {
                let secs = played.remove(&app_id).map_or(0, |t| t.as_secs());
                if events
                    .send(AppEvent::SessionEnded { app_id, secs })
                    .is_err()
                {
                    return;
                }
            }
            for (&app_id, &time) in &played {
                let progress = AppEvent::SessionProgress {
                    app_id,
                    played: time,
                    idle,
                };
                if events.send(progress).is_err() {
                    return;
                }
            }
            for app_id in begun {
                played.insert(app_id, Duration::ZERO);
                if events.send(AppEvent::SessionStarted { app_id }).is_err() {
                    return;
                }
            }
        }
    });
    watch_tx
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

/// When the user last used the keyboard, the mouse or an XInput controller (Xbox pads,
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
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: `info` is a valid LASTINPUTINFO with its size set.
    if unsafe { GetLastInputInfo(&mut info) } == 0 {
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
    let status = unsafe { XInputGetState(slot, &mut state) };
    (status == 0).then_some(state.dwPacketNumber)
}

/// File name of `exe`, lowercased: `watch_exe` may hold a full path.
fn exe_key(exe: &str) -> String {
    let exe = exe.trim();
    Path::new(exe)
        .file_name()
        .map_or(exe.into(), |n| n.to_string_lossy().into_owned())
        .to_lowercase()
}

/// Apps with at least one process running, matched on exe name ignoring case.
fn running_apps<S: AsRef<str>>(
    watched: &[Watched],
    process_names: impl IntoIterator<Item = S>,
) -> HashSet<i64> {
    let running: HashSet<String> = process_names
        .into_iter()
        .map(|n| n.as_ref().to_lowercase())
        .collect();
    watched
        .iter()
        .filter(|w| running.contains(&exe_key(&w.exe)))
        .map(|w| w.app_id)
        .collect()
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
        let tracker = spawn(events, None);
        tracker.send(vec![watched(7, &exe)]).unwrap();
        let wait = Duration::from_secs(30);
        assert!(matches!(
            rx.recv_timeout(wait).unwrap(),
            AppEvent::SessionStarted { app_id: 7 }
        ));
        tracker.send(Vec::new()).unwrap();
        // Progress reports may come first.
        let ended = std::iter::from_fn(|| rx.recv_timeout(wait).ok())
            .find(|e| !matches!(e, AppEvent::SessionProgress { .. }));
        assert!(matches!(
            ended,
            Some(AppEvent::SessionEnded { app_id: 7, .. })
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
        ];
        let running = running_apps(&list, ["hades.exe", "code.exe", "explorer.exe"]);
        assert_eq!(running, HashSet::from([1, 2]));
    }

    #[test]
    fn several_instances_count_once_and_apps_can_share_an_exe() {
        let list = [watched(1, "steam.exe"), watched(2, "steam.exe")];
        let running = running_apps(&list, ["steam.exe", "steam.exe"]);
        assert_eq!(running, HashSet::from([1, 2]));
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
