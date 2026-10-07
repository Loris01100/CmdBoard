//! Session tracker thread: polls running processes with `sysinfo` and reports when a
//! watched app starts or stops (plan section 12).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use crate::event::AppEvent;

const POLL: Duration = Duration::from_secs(3);

/// An app to track and the exe name of its process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watched {
    pub app_id: i64,
    pub exe: String,
}

/// Starts the tracker. Send it the full watch list whenever apps change; it stops
/// once either channel is closed.
pub fn spawn(events: Sender<AppEvent>) -> Sender<Vec<Watched>> {
    let (watch_tx, watch_rx) = mpsc::channel::<Vec<Watched>>();
    thread::spawn(move || {
        let mut system = System::new();
        let mut watched = Vec::new();
        let mut started: HashMap<i64, Instant> = HashMap::new();
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
            let previous: HashSet<i64> = started.keys().copied().collect();
            let (begun, ended) = diff(&previous, &now_running);

            for app_id in ended {
                let secs = started.remove(&app_id).map_or(0, |t| t.elapsed().as_secs());
                if events
                    .send(AppEvent::SessionEnded { app_id, secs })
                    .is_err()
                {
                    return;
                }
            }
            for app_id in begun {
                started.insert(app_id, Instant::now());
                if events.send(AppEvent::SessionStarted { app_id }).is_err() {
                    return;
                }
            }
        }
    });
    watch_tx
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
        let tracker = spawn(events);
        tracker.send(vec![watched(7, &exe)]).unwrap();
        let wait = Duration::from_secs(30);
        assert!(matches!(
            rx.recv_timeout(wait).unwrap(),
            AppEvent::SessionStarted { app_id: 7 }
        ));
        tracker.send(Vec::new()).unwrap();
        assert!(matches!(
            rx.recv_timeout(wait).unwrap(),
            AppEvent::SessionEnded { app_id: 7, .. }
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
    fn diff_reports_starts_and_ends() {
        let previous = HashSet::from([1, 2]);
        let current = HashSet::from([2, 4, 3]);
        assert_eq!(diff(&previous, &current), (vec![3, 4], vec![1]));
        assert_eq!(diff(&current, &current), (vec![], vec![]));
    }
}
