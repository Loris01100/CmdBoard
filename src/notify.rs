//! Windows notifications (plan section 11): a limit reached or a break reminder, seen
//! while in a game rather than in `CmdBoard`, background tracker included. A notification
//! area icon's balloon, which Windows 10 and 11 show as a toast named after the exe:
//! no app registration, no `WinRT`. Focus assist and "do not disturb while gaming" may
//! hold it in the notification center.

use std::ptr::{null, null_mut};
use std::thread;
use std::time::Duration;

use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_TIP, NIIF_INFO, NIIF_WARNING, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, HWND_MESSAGE, IDI_APPLICATION, LoadIconW,
};

use crate::launcher::programs::wide;

/// How long the icon stays: removing it takes the toast away with it.
const SHOWN_FOR: Duration = Duration::from_secs(15);

/// Resource id of the application icon (`assets/cmdboard.rc`).
const ICON_ID: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub level: Level,
    pub title: String,
    pub body: String,
}

/// Shows `notice` from a short-lived thread, which removes its icon afterwards. A
/// notification that cannot be shown is dropped: there is nowhere to report it in the
/// background.
pub fn show(notice: Notice) {
    thread::spawn(move || {
        if let Some(icon) = Icon::add(&notice) {
            thread::sleep(SHOWN_FOR);
            drop(icon);
        }
    });
}

/// A notification area icon on a message-only window: it receives no broadcasts, so
/// the thread need not pump messages. Removed on drop.
struct Icon(NOTIFYICONDATAW);

impl Icon {
    fn add(notice: &Notice) -> Option<Self> {
        let class = wide("STATIC"); // built-in class: nothing to register
        // SAFETY: `class` is nul-terminated; a message-only window needs no other argument.
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                null_mut(),
                null_mut(),
                null(),
            )
        };
        if window.is_null() {
            return None;
        }
        let mut data = NOTIFYICONDATAW {
            cbSize: u32::try_from(size_of::<NOTIFYICONDATAW>()).ok()?,
            hWnd: window,
            uFlags: NIF_ICON | NIF_TIP | NIF_INFO,
            hIcon: app_icon(),
            dwInfoFlags: match notice.level {
                Level::Info => NIIF_INFO,
                Level::Warning => NIIF_WARNING,
            },
            ..Default::default()
        };
        copy(&mut data.szTip, "CmdBoard");
        copy(&mut data.szInfoTitle, &notice.title);
        copy(&mut data.szInfo, &notice.body);
        let icon = Self(data);
        // SAFETY: `data` is fully initialized and sized by `cbSize`.
        let added = unsafe { Shell_NotifyIconW(NIM_ADD, &raw const icon.0) } != 0;
        added.then_some(icon)
    }
}

impl Drop for Icon {
    fn drop(&mut self) {
        // SAFETY: the icon and window were created by `Icon::add`, and are removed once.
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &raw const self.0);
            DestroyWindow(self.0.hWnd);
        }
    }
}

/// The exe's own icon, else the generic application one.
fn app_icon() -> windows_sys::Win32::UI::WindowsAndMessaging::HICON {
    // SAFETY: a null name gives this exe's module; a resource id is passed as a pointer
    // value (`MAKEINTRESOURCE`), and shared icons are never freed.
    unsafe {
        let icon = LoadIconW(GetModuleHandleW(null()), usize::from(ICON_ID) as *const u16);
        if icon.is_null() {
            LoadIconW(null_mut(), IDI_APPLICATION)
        } else {
            icon
        }
    }
}

/// Copies `text` into a fixed-size field, cut to fit with its nul terminator.
fn copy(field: &mut [u16], text: &str) {
    let units: Vec<u16> = text.encode_utf16().take(field.len() - 1).collect();
    field[..units.len()].copy_from_slice(&units);
    field[units.len()] = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_cuts_to_fit_with_a_nul() {
        let mut field = [7u16; 4];
        copy(&mut field, "abcdef");
        assert_eq!(String::from_utf16_lossy(&field[..3]), "abc");
        assert_eq!(field[3], 0);
        copy(&mut field, "é");
        assert_eq!(field[..2], [u16::from(b'\xe9'), 0]);
    }

    /// Shows a real notification: `cargo test shows_a_notification -- --ignored`.
    #[test]
    #[ignore = "shows a Windows notification"]
    fn shows_a_notification() {
        let notice = Notice {
            level: Level::Warning,
            title: "⚠ Limite atteinte".into(),
            body: "Jeux : 2h / 2h aujourd'hui".into(),
        };
        let icon = Icon::add(&notice).expect("notification icon not added");
        thread::sleep(Duration::from_secs(6));
        drop(icon);
    }
}
