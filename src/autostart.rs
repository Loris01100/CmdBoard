//! Starting the background tracker at Windows sign-in (plan section 12): a value of the
//! user's `Run` key, kept in step with `autostart` in `config.toml`. It shows in Task
//! Manager's startup apps, where Windows lets the user turn it off without removing it.

use std::path::Path;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW,
};

use crate::instance::BACKGROUND_FLAG;
use crate::launcher::programs::wide;
use crate::launcher::registry::Key;

/// Programs started at sign-in, for this user only: no admin rights needed.
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

const VALUE: &str = "CmdBoard";

/// What Windows runs at sign-in: `exe` quoted, as its path may hold spaces.
pub fn command_line(exe: &Path) -> String {
    format!("\"{}\" {BACKGROUND_FLAG}", exe.display())
}

/// Whether Windows starts the background tracker at sign-in.
pub fn is_enabled() -> bool {
    read(RUN_KEY, VALUE).is_some()
}

/// Adds the value for `exe`, or removes it. Writes only when it changes, so the path
/// follows a reinstall elsewhere.
pub fn sync(enabled: bool, exe: &Path) -> anyhow::Result<()> {
    sync_at(RUN_KEY, VALUE, enabled, exe)
}

fn sync_at(path: &str, name: &str, enabled: bool, exe: &Path) -> anyhow::Result<()> {
    let current = read(path, name);
    if !enabled {
        return if current.is_some() {
            remove(path, name)
        } else {
            Ok(())
        };
    }
    let command = command_line(exe);
    if current.as_deref() == Some(command.as_str()) {
        return Ok(());
    }
    write(path, name, &command)
}

fn read(path: &str, name: &str) -> Option<String> {
    Key::open(HKEY_CURRENT_USER, path, 0)?.string(&wide(""), name)
}

/// Writes a string under HKCU, creating the key if needed.
fn write(path: &str, name: &str, text: &str) -> anyhow::Result<()> {
    let (path, name, text) = (wide(path), wide(name), wide(text));
    let bytes = u32::try_from(text.len() * 2)?;
    // SAFETY: the strings are nul-terminated; `text` holds `bytes` bytes, nul included.
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            name.as_ptr(),
            REG_SZ,
            text.as_ptr().cast(),
            bytes,
        )
    };
    anyhow::ensure!(status == ERROR_SUCCESS, "registry error {status}");
    Ok(())
}

fn remove(path: &str, name: &str) -> anyhow::Result<()> {
    let (path, name) = (wide(path), wide(name));
    // SAFETY: both strings are nul-terminated.
    let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, path.as_ptr(), name.as_ptr()) };
    anyhow::ensure!(
        status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND,
        "registry error {status}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_quotes_the_exe() {
        let exe = Path::new(r"C:\Program Files\CmdBoard\cmdboard.exe");
        assert_eq!(
            command_line(exe),
            r#""C:\Program Files\CmdBoard\cmdboard.exe" --background"#
        );
    }

    #[test]
    fn sync_adds_follows_and_removes_the_value() {
        // A key of its own, never the real `Run` key.
        let path = format!(r"Software\CmdBoard.Test.Autostart.{}", std::process::id());
        let (old, new) = (Path::new(r"C:\old\cmdboard.exe"), Path::new(r"D:\new.exe"));

        sync_at(&path, VALUE, false, old).unwrap(); // nothing to remove
        assert_eq!(read(&path, VALUE), None);
        sync_at(&path, VALUE, true, old).unwrap();
        assert_eq!(read(&path, VALUE), Some(command_line(old)));
        sync_at(&path, VALUE, true, new).unwrap(); // moved
        assert_eq!(read(&path, VALUE), Some(command_line(new)));
        sync_at(&path, VALUE, false, new).unwrap();
        assert_eq!(read(&path, VALUE), None);

        let key = wide(&path);
        // SAFETY: `key` is nul-terminated; removes the empty test key.
        unsafe {
            windows_sys::Win32::System::Registry::RegDeleteKeyW(HKEY_CURRENT_USER, key.as_ptr())
        };
    }
}
