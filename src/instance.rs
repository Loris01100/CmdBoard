//! Only one CmdBoard per Windows session: two would both track the same apps, record
//! every session twice and award its XP twice.

use std::ptr::null;

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows_sys::Win32::System::Threading::CreateMutexW;

use crate::launcher::programs::wide;

/// `Local\`: one per logged-in user, who each have their own `%APPDATA%`.
const NAME: &str = r"Local\CmdBoard.SingleInstance";

/// Proof that this process is the only CmdBoard. Keep it for the life of the process;
/// Windows releases it on exit, crash included.
pub struct Instance(HANDLE);

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: `self.0` is the handle `CreateMutexW` returned, closed only here.
        unsafe { CloseHandle(self.0) };
    }
}

/// `None` when another CmdBoard is already running.
pub fn acquire() -> anyhow::Result<Option<Instance>> {
    acquire_named(NAME)
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
}
