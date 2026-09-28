//! Starting with Windows: a value under the user's `Run` key that launches this exe.

use std::io;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, WIN32_ERROR};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};

use crate::win::wide;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "quickbar";

/// Whether Windows will start this very exe at sign-in; an entry left behind
/// by a copy somewhere else does not count.
pub fn is_on() -> bool {
    let (Some(ours), Some(theirs)) = (command_line(), registered_command()) else {
        return false;
    };
    ours.eq_ignore_ascii_case(&theirs)
}

pub fn set(on: bool) -> io::Result<()> {
    let (key, value) = (wide(RUN_KEY), wide(VALUE_NAME));
    let err = if on {
        let command = command_line().ok_or_else(|| io::Error::other("exe path unknown"))?;
        let data = wide(&command);
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                REG_SZ,
                data.as_ptr() as *const _,
                (data.len() * 2) as u32,
            )
        }
    } else {
        match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) } {
            ERROR_FILE_NOT_FOUND => 0,
            err => err,
        }
    };
    check(err)
}

/// This exe's path, quoted in case it has spaces.
fn command_line() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", exe.to_str()?))
}

/// What the `Run` key starts under our name, if anything.
fn registered_command() -> Option<String> {
    let (key, value) = (wide(RUN_KEY), wide(VALUE_NAME));
    let mut buf = [0u16; 1024];
    let mut bytes = (buf.len() * 2) as u32;
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut _,
            &mut bytes,
        )
    };
    check(err).ok()?;
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

fn check(err: WIN32_ERROR) -> io::Result<()> {
    match err {
        0 => Ok(()),
        err => Err(io::Error::from_raw_os_error(err as i32)),
    }
}
