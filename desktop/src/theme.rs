//! Light or dark, as Claude shows it: Claude's own choice from its `config.json`,
//! or Windows' app mode when Claude follows the system or cannot be read.
//! A thread of its own watches both and tells the bar when the answer changes.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE, HWND, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadDirectoryChangesW,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, REG_NOTIFY_CHANGE_LAST_SET, RRF_RT_REG_DWORD, RegCloseKey,
    RegGetValueW, RegNotifyChangeKeyValue, RegOpenKeyExW,
};
use windows_sys::Win32::System::Threading::{CreateEventW, INFINITE, WaitForMultipleObjects};
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::paint::Palette;
use crate::win::wide;

const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const CONFIG: &str = "config.json";
/// Claude saves `config.json` by writing a temporary file and renaming it over the old one;
/// in between there is no `config.json`, so a failed read gets one more try this much later.
const RETRY: Duration = Duration::from_millis(50);
/// Waited after a change before reading, so a burst of them makes one read.
const SETTLE: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    pub fn palette(self) -> &'static Palette {
        match self {
            Theme::Light => &Palette::LIGHT,
            Theme::Dark => &Palette::DARK,
        }
    }

    /// The theme in the `wparam` of a message `follow` posted.
    pub fn from_message(wparam: WPARAM) -> Theme {
        if wparam == Theme::Light as WPARAM {
            Theme::Light
        } else {
            Theme::Dark
        }
    }
}

/// The theme now. From then on, a thread of its own posts `message` to `hwnd`,
/// with the new theme in `wparam`, each time the theme changes, for the life of the process.
/// Where watching cannot start, changes there go unnoticed; the theme is still read at start.
pub fn follow(hwnd: HWND, message: u32) -> Theme {
    let hwnd = hwnd as usize;
    let (tell, told) = mpsc::channel();
    std::thread::spawn(move || watch(tell, hwnd as HWND, message));
    told.recv()
        .unwrap_or_else(|_| read(claude_folder().as_deref()))
}

/// Where Claude keeps `config.json`.
fn claude_folder() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("Claude"))
}

/// Claude's choice, or Windows' when Claude follows the system or its choice cannot be read.
fn read(folder: Option<&Path>) -> Theme {
    match folder.and_then(claude_choice) {
        Some(Choice::Light) => Theme::Light,
        Some(Choice::Dark) => Theme::Dark,
        Some(Choice::System) | None => system_theme(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Choice {
    Light,
    Dark,
    System,
}

fn claude_choice(folder: &Path) -> Option<Choice> {
    let path = folder.join(CONFIG);
    let text = std::fs::read_to_string(&path).or_else(|_| {
        std::thread::sleep(RETRY);
        std::fs::read_to_string(&path)
    });
    parse_choice(&text.ok()?)
}

/// The `userThemeMode` in Claude's `config.json`. Like Claude itself,
/// takes only `light`, `dark` and `system`.
fn parse_choice(config: &str) -> Option<Choice> {
    let key = "\"userThemeMode\"";
    let rest = &config[config.find(key)? + key.len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    match &rest[..rest.find('"')?] {
        "light" => Some(Choice::Light),
        "dark" => Some(Choice::Dark),
        "system" => Some(Choice::System),
        _ => None,
    }
}

/// Windows' app mode; light, Windows' default, when it cannot be read.
fn system_theme() -> Theme {
    let (key, value) = (wide(PERSONALIZE), wide("AppsUseLightTheme"));
    let mut light = 1u32;
    let mut bytes = size_of_val(&light) as u32;
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut light as *mut u32 as *mut _,
            &mut bytes,
        )
    };
    if err == 0 && light == 0 {
        Theme::Dark
    } else {
        Theme::Light
    }
}

/// Sends the theme through `first`, then waits on Claude's folder and Windows' theme key
/// and posts each change of theme. Returns only when there is nothing left to wait on.
fn watch(first: Sender<Theme>, hwnd: HWND, message: u32) {
    let folder = claude_folder();
    // Boxed: the system writes into it while a wait is pending, so it must not move.
    let mut claude = folder.as_deref().and_then(Folder::open).map(Box::new);
    let mut system = SystemKey::open();
    // Watching starts before the first read, so a change in between is not missed.
    arm(&mut claude, &mut system);
    let mut theme = read(folder.as_deref());
    // Gone only if `follow` is, and then nobody is waiting.
    let _ = first.send(theme);
    // Changes seen and not yet read, and when to read them.
    let mut due: Option<Instant> = None;
    loop {
        let events = arm(&mut claude, &mut system);
        if events.is_empty() && due.is_none() {
            return;
        }
        let timeout = due.map(|due| due.saturating_duration_since(Instant::now()));
        let woke = match timeout {
            // Windows takes no empty list of things to wait on.
            Some(timeout) if events.is_empty() => {
                std::thread::sleep(timeout);
                WAIT_TIMEOUT
            }
            _ => unsafe {
                let timeout = timeout.map_or(INFINITE, |t| t.as_millis() as u32);
                WaitForMultipleObjects(events.len() as u32, events.as_ptr(), 0, timeout)
            },
        };
        if woke == WAIT_TIMEOUT {
            due = None;
            let now = read(folder.as_deref());
            if now != theme {
                theme = now;
                unsafe { PostMessageW(hwnd, message, theme as WPARAM, 0) };
            }
            continue;
        }
        let Some(i) = woke
            .checked_sub(WAIT_OBJECT_0)
            .filter(|&i| (i as usize) < events.len())
        else {
            return;
        };
        let changed = match &mut claude {
            Some(folder) if folder.event.0 == events[i as usize] => folder.config_changed(),
            _ => system.as_mut().is_some_and(SystemKey::value_set),
        };
        if changed && due.is_none() {
            due = Some(Instant::now() + SETTLE);
        }
    }
}

/// Has both watchers wait for their next change, dropping the ones that fail.
/// Returns what to wait on.
fn arm(claude: &mut Option<Box<Folder>>, system: &mut Option<SystemKey>) -> Vec<HANDLE> {
    let mut events = Vec::with_capacity(2);
    if let Some(folder) = claude {
        if folder.arm() {
            events.push(folder.event.0);
        } else {
            *claude = None;
        }
    }
    if let Some(key) = system {
        if key.arm() {
            events.push(key.event.0);
        } else {
            *system = None;
        }
    }
    events
}

/// An event object, closed when dropped.
struct Event(HANDLE);

impl Event {
    fn new(manual_reset: bool) -> Option<Event> {
        let event =
            unsafe { CreateEventW(std::ptr::null(), manual_reset as i32, 0, std::ptr::null()) };
        (!event.is_null()).then_some(Event(event))
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// Claude's folder, watched for files written or renamed in it.
struct Folder {
    handle: HANDLE,
    event: Event,
    overlapped: OVERLAPPED,
    /// Where the system lists what changed; `FILE_NOTIFY_INFORMATION` wants 4-byte alignment.
    buffer: [u32; 1024],
    armed: bool,
}

impl Folder {
    fn open(path: &Path) -> Option<Folder> {
        let path = wide(path.to_str()?);
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                FILE_LIST_DIRECTORY,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let Some(event) = Event::new(true) else {
            unsafe { CloseHandle(handle) };
            return None;
        };
        Some(Folder {
            handle,
            event,
            overlapped: unsafe { std::mem::zeroed() },
            buffer: [0; 1024],
            armed: false,
        })
    }

    /// Starts waiting for the next change, unless a wait is already pending. False on failure.
    fn arm(&mut self) -> bool {
        if self.armed {
            return true;
        }
        self.overlapped = unsafe { std::mem::zeroed() };
        self.overlapped.hEvent = self.event.0;
        let ok = unsafe {
            ReadDirectoryChangesW(
                self.handle,
                self.buffer.as_mut_ptr() as *mut _,
                size_of_val(&self.buffer) as u32,
                0,
                FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE,
                std::ptr::null_mut(),
                &mut self.overlapped,
                None,
            )
        };
        self.armed = ok != 0;
        self.armed
    }

    /// After the event fired: takes the changes in and says whether `config.json` was among them.
    fn config_changed(&mut self) -> bool {
        self.armed = false;
        let mut bytes = 0;
        let ok = unsafe { GetOverlappedResult(self.handle, &self.overlapped, &mut bytes, 0) };
        if ok == 0 {
            return false;
        }
        // Zero bytes: more changed than the buffer holds, so assume it was.
        let listed = unsafe {
            std::slice::from_raw_parts(self.buffer.as_ptr() as *const u8, bytes as usize)
        };
        bytes == 0 || names(listed).any(|name| name.eq_ignore_ascii_case(CONFIG))
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        unsafe {
            // The system may still write into `overlapped` and `buffer`; wait until it is done.
            if self.armed {
                CancelIoEx(self.handle, &self.overlapped);
                let mut bytes = 0;
                GetOverlappedResult(self.handle, &self.overlapped, &mut bytes, 1);
            }
            CloseHandle(self.handle);
        }
    }
}

/// The file names in a list of `FILE_NOTIFY_INFORMATION` entries.
fn names(listed: &[u8]) -> impl Iterator<Item = String> + '_ {
    let field = |at: usize| -> Option<usize> {
        Some(u32::from_le_bytes(listed.get(at..at + 4)?.try_into().ok()?) as usize)
    };
    let mut at = Some(0);
    std::iter::from_fn(move || {
        let entry = at?;
        let (next, length) = (field(entry)?, field(entry + 8)?);
        let name = listed.get(entry + 12..entry + 12 + length)?;
        at = (next != 0).then_some(entry + next);
        let (pairs, _) = name.as_chunks::<2>();
        let units: Vec<u16> = pairs.iter().map(|&pair| u16::from_le_bytes(pair)).collect();
        Some(String::from_utf16_lossy(&units))
    })
}

/// Windows' theme key, watched for values set in it.
struct SystemKey {
    key: HKEY,
    event: Event,
    armed: bool,
}

impl SystemKey {
    fn open() -> Option<SystemKey> {
        let event = Event::new(false)?;
        let path = wide(PERSONALIZE);
        let mut key = std::ptr::null_mut();
        let err =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_NOTIFY, &mut key) };
        if err != 0 {
            return None;
        }
        Some(SystemKey {
            key,
            event,
            armed: false,
        })
    }

    /// Asks for the next change, unless already asked. Each notice comes once. False on failure.
    fn arm(&mut self) -> bool {
        if !self.armed {
            let err = unsafe {
                RegNotifyChangeKeyValue(self.key, 0, REG_NOTIFY_CHANGE_LAST_SET, self.event.0, 1)
            };
            self.armed = err == 0;
        }
        self.armed
    }

    /// After the event fired: takes the notice, so `arm` asks for the next one. Always true.
    fn value_set(&mut self) -> bool {
        self.armed = false;
        true
    }
}

impl Drop for SystemKey {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.key) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_claude_choice() {
        let config = r#"{"locale":"en-US","userThemeMode": "dark","x":1}"#;
        assert_eq!(parse_choice(config), Some(Choice::Dark));
        assert_eq!(
            parse_choice(r#"{"userThemeMode":"system"}"#),
            Some(Choice::System)
        );
        assert_eq!(
            parse_choice("{\n  \"userThemeMode\" :\n \"light\"\n}"),
            Some(Choice::Light)
        );
    }

    #[test]
    fn unknown_or_missing_choice_is_none() {
        assert_eq!(parse_choice(r#"{"userThemeMode":"auto"}"#), None);
        assert_eq!(parse_choice(r#"{"userThemeMode":1}"#), None);
        assert_eq!(parse_choice(r#"{"userThemeMode":"dar"#), None);
        assert_eq!(parse_choice(r#"{"locale":"en-US"}"#), None);
    }

    #[test]
    fn lists_changed_names() {
        let entry = |next: u32, name: &str| {
            let name: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let mut bytes = Vec::new();
            bytes.extend(next.to_le_bytes());
            bytes.extend(1u32.to_le_bytes());
            bytes.extend((name.len() as u32).to_le_bytes());
            bytes.extend(name);
            bytes
        };
        let first = entry(0, "config.json.tmp-1");
        let mut listed = entry(first.len() as u32, "config.json.tmp-1");
        listed.extend(entry(0, "Config.json"));
        let names: Vec<String> = names(&listed).collect();
        assert_eq!(names, ["config.json.tmp-1", "Config.json"]);
    }
}
