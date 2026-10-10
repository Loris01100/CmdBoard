//! Only one `CmdBoard` per Windows session: two would both track the same apps, record
//! every session twice and award its XP twice. The UI and the background tracker
//! (`cmdboard --background`, plan section 12) take turns holding the instance.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::ptr::null;
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CreateEventW, CreateMutexW,
    DETACHED_PROCESS, EVENT_MODIFY_STATE, OpenEventW, SetEvent, WaitForSingleObject,
};

use crate::launcher::programs::wide;

/// `Local\`: one per logged-in user, who each have their own `%APPDATA%`.
const NAME: &str = r"Local\CmdBoard.SingleInstance";

/// Set to ask the background tracker to hand its sessions over and quit. It only exists
/// while a background tracker runs.
const STOP: &str = r"Local\CmdBoard.StopBackground";

/// How long a process waits for the other one to hand the instance over.
pub const HANDOVER: Duration = Duration::from_secs(10);

/// Command-line flag that starts the background tracker.
pub const BACKGROUND_FLAG: &str = "--background";

/// Proof that this process is the only `CmdBoard`. Keep it for the life of the process;
/// Windows releases it on exit, crash included.
pub struct Instance(HANDLE);

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: `self.0` is the handle `CreateMutexW` returned, closed only here.
        unsafe { CloseHandle(self.0) };
    }
}

/// `None` when another `CmdBoard` is already running.
pub fn acquire() -> anyhow::Result<Option<Instance>> {
    acquire_named(NAME)
}

/// Waits up to `HANDOVER` for the instance while the other process quits. `None` if it
/// is still held, or as soon as `give_up` says so.
pub fn acquire_within(give_up: impl Fn() -> bool) -> anyhow::Result<Option<Instance>> {
    let deadline = Instant::now() + HANDOVER;
    loop {
        if let Some(instance) = acquire()? {
            return Ok(Some(instance));
        }
        if give_up() || Instant::now() >= deadline {
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn acquire_named(name: &str) -> anyhow::Result<Option<Instance>> {
    let name = wide(name);
    // SAFETY: `name` is nul-terminated; null attributes give the default security.
    let handle = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: no call in between, so this is the error code of `CreateMutexW`.
    let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let instance = Instance(handle);
    Ok((!already_running).then_some(instance))
}

/// The background tracker's side of the stop request. Created before it waits for the
/// instance, so a UI starting meanwhile can already reach it.
pub struct StopSignal(HANDLE);

impl Drop for StopSignal {
    fn drop(&mut self) {
        // SAFETY: `self.0` is the handle `CreateEventW` returned, closed only here.
        unsafe { CloseHandle(self.0) };
    }
}

impl StopSignal {
    pub fn create() -> anyhow::Result<Self> {
        Self::create_named(STOP)
    }

    fn create_named(name: &str) -> anyhow::Result<Self> {
        let name = wide(name);
        // SAFETY: `name` is nul-terminated. Manual reset: it stays set once requested.
        let handle = unsafe { CreateEventW(null(), 1, 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Self(handle))
    }

    pub fn is_requested(&self) -> bool {
        // SAFETY: `self.0` is a valid event handle; a zero timeout only reads its state.
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_OBJECT_0 }
    }
}

/// Asks the background tracker to quit. Returns false when none is running.
pub fn request_stop() -> bool {
    request_stop_named(STOP)
}

fn request_stop_named(name: &str) -> bool {
    let name = wide(name);
    // SAFETY: `name` is nul-terminated; fails when no process holds the event.
    let handle = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
    if handle.is_null() {
        return false;
    }
    // SAFETY: `handle` was just opened with the right to set it, and is closed once.
    unsafe {
        SetEvent(handle);
        CloseHandle(handle);
    }
    true
}

/// Starts `exe --background` with no console, so it outlives the terminal. It leaves
/// the terminal's job when allowed, as some close every process in it.
pub fn spawn_background(exe: &Path) -> std::io::Result<()> {
    let spawn = |flags: u32| {
        Command::new(exe)
            .arg(BACKGROUND_FLAG)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | flags)
            .spawn()
    };
    spawn(CREATE_BREAKAWAY_FROM_JOB)
        .or_else(|_| spawn(0))
        .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_instance_is_refused_until_the_first_quits() {
        let name = format!(r"Local\CmdBoard.Test.{}", std::process::id());
        let first = acquire_named(&name).unwrap();
        assert!(first.is_some());
        assert!(acquire_named(&name).unwrap().is_none());
        drop(first);
        assert!(acquire_named(&name).unwrap().is_some());
    }

    #[test]
    fn stop_reaches_a_running_background_tracker_only() {
        let name = format!(r"Local\CmdBoard.TestStop.{}", std::process::id());
        assert!(!request_stop_named(&name)); // none running

        let signal = StopSignal::create_named(&name).unwrap();
        assert!(!signal.is_requested());
        assert!(request_stop_named(&name));
        assert!(signal.is_requested());
        assert!(signal.is_requested()); // stays set
        drop(signal);
        assert!(!request_stop_named(&name)); // gone with it
    }
}
