//! Read-only access to the registry through the Unicode API of `windows-sys`, without
//! spawning `reg.exe`: installed programs, URI schemes, launcher libraries.

use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    HKEY, KEY_READ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW,
    RegOpenKeyExW,
};

use super::programs::wide;

/// An open registry key, closed on drop.
pub(super) struct Key(HKEY);

impl Key {
    pub(super) fn open(root: HKEY, path: &str, view: u32) -> Option<Self> {
        let path = wide(path);
        let mut key = null_mut();
        // SAFETY: `path` is nul-terminated, `key` receives the handle.
        let status =
            unsafe { RegOpenKeyExW(root, path.as_ptr(), 0, KEY_READ | view, &raw mut key) };
        (status == ERROR_SUCCESS).then_some(Self(key))
    }

    pub(super) fn subkeys(&self) -> Vec<String> {
        // Key names are at most 255 characters.
        let mut buf = [0u16; 256];
        let mut names = Vec::new();
        for index in 0.. {
            let mut len = u32::try_from(buf.len()).unwrap_or(0);
            // SAFETY: `buf` holds `len` characters; the optional outputs are null.
            let status = unsafe {
                RegEnumKeyExW(
                    self.0,
                    index,
                    buf.as_mut_ptr(),
                    &raw mut len,
                    null(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                )
            };
            if status != ERROR_SUCCESS {
                break; // ERROR_NO_MORE_ITEMS
            }
            names.push(String::from_utf16_lossy(&buf[..len as usize]));
        }
        names
    }

    /// A string value of subkey `sub` (environment variables expanded), if not blank.
    pub(super) fn string(&self, sub: &[u16], name: &str) -> Option<String> {
        let name = wide(name);
        let mut size = 0u32;
        // SAFETY: a null buffer asks for the size, in bytes.
        let status = unsafe {
            RegGetValueW(
                self.0,
                sub.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                null_mut(),
                &raw mut size,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        // SAFETY: `buf` holds `size` bytes.
        let status = unsafe {
            RegGetValueW(
                self.0,
                sub.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                buf.as_mut_ptr().cast(),
                &raw mut size,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let text = String::from_utf16_lossy(&buf[..size as usize / 2]);
        let text = text.trim_end_matches('\0').trim();
        (!text.is_empty()).then(|| text.to_string())
    }

    pub(super) fn dword(&self, sub: &[u16], name: &str) -> Option<u32> {
        let name = wide(name);
        let mut value = 0u32;
        let mut size = 4u32;
        // SAFETY: `value` holds the 4 bytes of a DWORD.
        let status = unsafe {
            RegGetValueW(
                self.0,
                sub.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_DWORD,
                null_mut(),
                (&raw mut value).cast(),
                &raw mut size,
            )
        };
        (status == ERROR_SUCCESS).then_some(value)
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: the handle came from RegOpenKeyExW and is closed once.
        unsafe { RegCloseKey(self.0) };
    }
}
