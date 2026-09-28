//! The real [`Host`]: Claude's window, the clipboard and synthesized keys, and UI
//! Automation to find the prompt box among the web page's elements.

use std::thread::sleep;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, GetLastError, GlobalFree, HANDLE, HWND,
    LPARAM, POINT, RECT,
};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
};
use windows_sys::Win32::Storage::Packaging::Appx::GetPackageFamilyName;
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VIRTUAL_KEY, VK_CONTROL, VK_RETURN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GetForegroundWindow, GetWindow, GetWindowRect, GetWindowTextLengthW,
    GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, SW_RESTORE, SetForegroundWindow,
    ShowWindow,
};
use windows_sys::core::BOOL;

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation, IUIAutomationElement, IUIAutomationTreeWalker,
    TreeScope_Descendants, UIA_ControlTypePropertyId, UIA_EditControlTypeId,
};

use crate::config::Rect;
use crate::send::Host;

const CLAUDE_PACKAGE_FAMILY: &str = "Claude_pzs8sxrjxfjjc";
const CF_UNICODETEXT: u32 = 13;

/// How long Claude's window gets to come to the front.
const ACTIVATE_TIMEOUT: Duration = Duration::from_millis(500);
/// How long another program may keep the clipboard open before we give up.
const CLIPBOARD_TIMEOUT: Duration = Duration::from_millis(500);
/// How long Claude gets to put the cut draft on the clipboard.
const CUT_TIMEOUT: Duration = Duration::from_millis(300);
/// How long Claude gets to read the clipboard after Ctrl+V.
const PASTE_SETTLE: Duration = Duration::from_millis(150);
/// How long Claude gets to take the submitted text out of the prompt box.
const SUBMIT_SETTLE: Duration = Duration::from_millis(250);
/// How long Chromium gets to build its accessibility tree, the first time it is asked for one.
const PROMPT_LOOKUP_TIMEOUT: Duration = Duration::from_millis(500);
/// How long the prompt box gets to take the focus.
const FOCUS_TIMEOUT: Duration = Duration::from_millis(300);

// HTML classes on Claude's Code page, which UI Automation reports as class names.
/// The prompt box, a ProseMirror editor; with the second class while it really has the focus.
const PROMPT_CLASS: &str = "ProseMirror";
const PROMPT_FOCUSED_CLASS: &str = "ProseMirror-focused";
/// Around a Code conversation; the prompt box of a chat is elsewhere.
const CODE_PANEL_CLASS: &str = "epitaxy-chat-panel";
/// Each pane of a split; the one last used carries `aria-current`.
const PANE_CLASS: &str = "dframe-pane";

/// Claude's main window: a visible, unowned, titled top-level window of the Claude MSIX package.
pub fn find_claude() -> Option<HWND> {
    unsafe extern "system" fn visit(hwnd: HWND, found: LPARAM) -> BOOL {
        unsafe {
            if is_main_window(hwnd) && is_claude_process(hwnd) {
                *(found as *mut HWND) = hwnd;
                return 0;
            }
        }
        1
    }
    let mut found: HWND = std::ptr::null_mut();
    unsafe { EnumWindows(Some(visit), &mut found as *mut HWND as LPARAM) };
    (!found.is_null()).then_some(found)
}

unsafe fn is_main_window(hwnd: HWND) -> bool {
    unsafe {
        if IsWindowVisible(hwnd) == 0
            || !GetWindow(hwnd, GW_OWNER).is_null()
            || GetWindowTextLengthW(hwnd) == 0
        {
            return false;
        }
        let mut cloaked = 0u32;
        let hr = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            &mut cloaked as *mut u32 as *mut _,
            size_of::<u32>() as u32,
        );
        hr != 0 || cloaked == 0
    }
}

/// Whether `hwnd` belongs to any process of the Claude MSIX package.
pub unsafe fn is_claude_process(hwnd: HWND) -> bool {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut name = [0u16; 128];
        let mut len = name.len() as u32;
        let err = GetPackageFamilyName(process, &mut len, name.as_mut_ptr());
        CloseHandle(process);
        err == 0
            && len > 0
            && String::from_utf16_lossy(&name[..len as usize - 1]) == CLAUDE_PACKAGE_FAMILY
    }
}

/// Claims quickbar's name for this sign-in session; false when another copy already holds it.
pub fn first_instance() -> bool {
    let name = wide("Local\\quickbar");
    unsafe {
        // Stays open for the life of the process, which is what holds the name.
        let mutex = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        let err = GetLastError();
        // Denied means another copy, running as someone else, holds it; any other
        // failure should not keep quickbar from running.
        if mutex.is_null() {
            err != ERROR_ACCESS_DENIED
        } else {
            err != ERROR_ALREADY_EXISTS
        }
    }
}

/// The window's visible bounds in screen pixels, without the invisible resize border.
pub fn frame(hwnd: HWND) -> Option<RECT> {
    let mut rect = NO_RECT;
    let hr = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            &mut rect as *mut RECT as *mut _,
            size_of::<RECT>() as u32,
        )
    };
    if hr == 0 {
        Some(rect)
    } else {
        window_rect(hwnd)
    }
}

/// The window's bounds in screen pixels.
pub fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = NO_RECT;
    (unsafe { GetWindowRect(hwnd, &mut rect) } != 0).then_some(rect)
}

/// Whether `hwnd` still names a window.
pub fn is_live(hwnd: HWND) -> bool {
    !hwnd.is_null() && unsafe { IsWindow(hwnd) } != 0
}

/// The part of the screen at `at` that windows may cover: all of it but the taskbar.
pub fn work_area(at: POINT) -> RECT {
    let mut monitor = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    unsafe { GetMonitorInfoW(MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST), &mut monitor) };
    monitor.rcWork
}

/// Whether `hwnd` is one of our own windows.
pub fn is_ours(hwnd: HWND) -> bool {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    pid == unsafe { GetCurrentProcessId() }
}

/// Whether there is text on the clipboard to paste.
pub fn clipboard_has_text() -> bool {
    unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) != 0 }
}

/// The point packed into a mouse message's `lparam`, which may be negative.
pub fn point_of(lparam: LPARAM) -> POINT {
    POINT {
        x: (lparam & 0xffff) as i16 as i32,
        y: ((lparam >> 16) & 0xffff) as i16 as i32,
    }
}

impl From<RECT> for Rect {
    fn from(rect: RECT) -> Rect {
        Rect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }
}

const NO_RECT: RECT = RECT {
    left: 0,
    top: 0,
    right: 0,
    bottom: 0,
};

/// Every clipboard format that lives in global memory, copied out;
/// `None` when the clipboard could not be read, so restoring leaves it alone.
pub struct SavedClipboard(Option<Vec<(u32, Vec<u8>)>>);

pub struct WinHost {
    /// Owns the clipboard while we write to it; `SetClipboardData` may fail without an owner.
    owner: HWND,
    /// Claude's main window, once brought to the front; null before that.
    claude: HWND,
    /// Marks our own temporary clipboard content so clipboard history skips it.
    exclude_from_history: u32,
}

impl WinHost {
    pub fn new(owner: HWND) -> Self {
        let name = wide("ExcludeClipboardContentFromMonitorProcessing");
        WinHost {
            owner,
            claude: std::ptr::null_mut(),
            exclude_from_history: unsafe { RegisterClipboardFormatW(name.as_ptr()) },
        }
    }

    fn set_text(&self, text: &str) {
        let wide = wide(text);
        let Some(_clipboard) = Clipboard::open(self.owner) else {
            return;
        };
        unsafe {
            EmptyClipboard();
            put(CF_UNICODETEXT, bytes_of(&wide));
            put(self.exclude_from_history, &[0]);
        }
    }
}

impl Host for WinHost {
    type Clipboard = SavedClipboard;

    fn save_clipboard(&mut self) -> SavedClipboard {
        let mut saved = Vec::new();
        let Some(_clipboard) = Clipboard::open(self.owner) else {
            return SavedClipboard(None);
        };
        let mut format = 0;
        loop {
            format = unsafe { EnumClipboardFormats(format) };
            if format == 0 {
                break;
            }
            if is_gdi_format(format) {
                continue;
            }
            unsafe {
                let handle = GetClipboardData(format);
                if handle.is_null() {
                    continue;
                }
                let size = GlobalSize(handle);
                let data = GlobalLock(handle) as *const u8;
                if data.is_null() {
                    continue;
                }
                saved.push((format, std::slice::from_raw_parts(data, size).to_vec()));
                GlobalUnlock(handle);
            }
        }
        SavedClipboard(Some(saved))
    }

    fn restore_clipboard(&mut self, saved: SavedClipboard) {
        let Some(saved) = saved.0 else { return };
        let Some(_clipboard) = Clipboard::open(self.owner) else {
            return;
        };
        unsafe {
            EmptyClipboard();
            for (format, data) in &saved {
                put(*format, data);
            }
        }
    }

    fn activate_claude(&mut self) -> bool {
        let Some(hwnd) = find_claude() else {
            return false;
        };
        self.claude = hwnd;
        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            if GetForegroundWindow() != hwnd {
                SetForegroundWindow(hwnd);
            }
        }
        wait_until(ACTIVATE_TIMEOUT, || unsafe {
            GetForegroundWindow() == hwnd
        })
    }

    fn focus_prompt(&mut self) -> bool {
        // The page's own focus: Win32 keeps the keyboard on Claude's top-level window
        // whichever element of the page has it.
        let Some(_com) = Com::start() else {
            return false;
        };
        let Some(prompt) = find_prompt(self.claude) else {
            return false;
        };
        let has_focus = || unsafe {
            prompt.CurrentHasKeyboardFocus().is_ok_and(|b| b.as_bool())
                && has_class(&prompt, PROMPT_FOCUSED_CLASS)
        };
        if has_focus() {
            return true;
        }
        // As the page's own `focus()`: the editor keeps its text and puts its caret back.
        unsafe { prompt.SetFocus() }.is_ok() && wait_until(FOCUS_TIMEOUT, has_focus)
    }

    fn cut_draft(&mut self) -> String {
        // Ctrl+X on an empty box leaves the clipboard alone, so start from an empty one
        // and treat "nothing arrived" as "no draft".
        if let Some(_clipboard) = Clipboard::open(self.owner) {
            unsafe { EmptyClipboard() };
        }
        let before = unsafe { GetClipboardSequenceNumber() };
        chord(VK_CONTROL, b'A' as VIRTUAL_KEY);
        chord(VK_CONTROL, b'X' as VIRTUAL_KEY);
        if !wait_until(
            CUT_TIMEOUT,
            || unsafe { GetClipboardSequenceNumber() } != before,
        ) {
            return String::new();
        }
        read_text(self.owner).unwrap_or_default()
    }

    fn paste(&mut self, text: &str) {
        self.set_text(text);
        chord(VK_CONTROL, b'V' as VIRTUAL_KEY);
        sleep(PASTE_SETTLE);
    }

    fn submit(&mut self) {
        send_keys(&[key(VK_RETURN, 0), key(VK_RETURN, KEYEVENTF_KEYUP)]);
        sleep(SUBMIT_SETTLE);
    }
}

/// Keeps COM running on this thread; stopped when dropped, so drop COM objects first.
struct Com;

impl Com {
    fn start() -> Option<Com> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .is_ok()
            .then_some(Com)
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

/// A Code prompt box on Claude's page.
struct Prompt {
    element: IUIAutomationElement,
    /// It has the page's focus, which it keeps while Claude is in the background.
    focused: bool,
    /// Its pane is the one of a split last used.
    current: bool,
}

/// The Code prompt box to type into in Claude's window `claude`, found as described at [`pick`].
fn find_prompt(claude: HWND) -> Option<IUIAutomationElement> {
    unsafe {
        let uia: IUIAutomation =
            CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).ok()?;
        let root = uia
            .ElementFromHandle(windows::Win32::Foundation::HWND(claude))
            .ok()?;
        let edit = VARIANT::from(UIA_EditControlTypeId.0);
        let edits = uia
            .CreatePropertyCondition(UIA_ControlTypePropertyId, &edit)
            .ok()?;
        let walker = uia.RawViewWalker().ok()?;
        let mut prompts = Vec::new();
        // The first time it is asked, Chromium answers with no elements until its tree is built.
        wait_until(PROMPT_LOOKUP_TIMEOUT, || {
            let Ok(found) = root.FindAll(TreeScope_Descendants, &edits) else {
                return false;
            };
            let count = found.Length().unwrap_or(0);
            prompts = (0..count)
                .filter_map(|i| found.GetElement(i).ok())
                .filter_map(|element| code_prompt(&walker, element))
                .collect();
            !prompts.is_empty()
        });
        let flags: Vec<_> = prompts.iter().map(|p| (p.focused, p.current)).collect();
        let i = pick(&flags)?;
        Some(prompts.swap_remove(i).element)
    }
}

/// `element` as a Code prompt box: a ProseMirror editor on screen inside a Code conversation.
unsafe fn code_prompt(
    walker: &IUIAutomationTreeWalker,
    element: IUIAutomationElement,
) -> Option<Prompt> {
    unsafe {
        let on_screen = !element.CurrentIsOffscreen().ok()?.as_bool();
        let rect = element.CurrentBoundingRectangle().ok()?;
        if !has_class(&element, PROMPT_CLASS)
            || !on_screen
            || rect.right <= rect.left
            || rect.bottom <= rect.top
        {
            return None;
        }
        // Up to its pane, which sits above the conversation.
        let (mut in_code, mut current) = (false, false);
        let mut above = walker.GetParentElement(&element).ok();
        while let Some(parent) = above {
            in_code |= has_class(&parent, CODE_PANEL_CLASS);
            if has_class(&parent, PANE_CLASS) {
                current = parent.CurrentAriaProperties().is_ok_and(|props| {
                    props
                        .to_string()
                        .split(';')
                        .any(|p| p.strip_prefix("current=").is_some_and(|v| v != "false"))
                });
                break;
            }
            above = walker.GetParentElement(&parent).ok();
        }
        let focused = element.CurrentHasKeyboardFocus().is_ok_and(|b| b.as_bool());
        in_code.then_some(Prompt {
            element,
            focused,
            current,
        })
    }
}

/// Of the Code prompt boxes, `(focused, current)` each, the one to type into: the one with the
/// page's focus, else the one in the pane Claude marks as last used, else the only one.
fn pick(prompts: &[(bool, bool)]) -> Option<usize> {
    let only = |matches: &dyn Fn(&(bool, bool)) -> bool| {
        let mut found = prompts.iter().enumerate().filter(|(_, p)| matches(p));
        match (found.next(), found.next()) {
            (Some((i, _)), None) => Some(i),
            _ => None,
        }
    };
    only(&|p| p.0)
        .or_else(|| only(&|p| p.1))
        .or_else(|| only(&|_| true))
}

/// Whether `element`'s HTML classes include `class`.
unsafe fn has_class(element: &IUIAutomationElement, class: &str) -> bool {
    unsafe { element.CurrentClassName() }.is_ok_and(|name| {
        name.to_string()
            .split_ascii_whitespace()
            .any(|c| c == class)
    })
}

/// Holds the clipboard open; other programs may have it briefly, so retry for a while.
struct Clipboard;

impl Clipboard {
    fn open(owner: HWND) -> Option<Clipboard> {
        wait_until(CLIPBOARD_TIMEOUT, || unsafe { OpenClipboard(owner) } != 0).then_some(Clipboard)
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}

fn read_text(owner: HWND) -> Option<String> {
    let _clipboard = Clipboard::open(owner)?;
    unsafe {
        let handle = GetClipboardData(CF_UNICODETEXT);
        if handle.is_null() {
            return None;
        }
        let data = GlobalLock(handle) as *const u16;
        if data.is_null() {
            return None;
        }
        let max = GlobalSize(handle) / 2;
        let wide = std::slice::from_raw_parts(data, max);
        let len = wide.iter().position(|&c| c == 0).unwrap_or(max);
        let text = String::from_utf16_lossy(&wide[..len]);
        GlobalUnlock(handle);
        Some(text)
    }
}

/// Copies `data` into global memory and hands it to the open clipboard.
unsafe fn put(format: u32, data: &[u8]) {
    unsafe {
        let handle = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1));
        if handle.is_null() {
            return;
        }
        let dest = GlobalLock(handle) as *mut u8;
        if dest.is_null() {
            GlobalFree(handle);
            return;
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), dest, data.len());
        GlobalUnlock(handle);
        if SetClipboardData(format, handle as HANDLE).is_null() {
            GlobalFree(handle);
        }
    }
}

/// Formats whose clipboard handle is a GDI object rather than global memory.
fn is_gdi_format(format: u32) -> bool {
    const CF_BITMAP: u32 = 2;
    const CF_METAFILEPICT: u32 = 3;
    const CF_PALETTE: u32 = 9;
    const CF_ENHMETAFILE: u32 = 14;
    matches!(format, CF_BITMAP | CF_METAFILEPICT | CF_PALETTE | CF_ENHMETAFILE)
        || (0x80..=0x8F).contains(&format) // CF_OWNERDISPLAY, CF_DSP*
        || (0x300..=0x3FF).contains(&format) // CF_GDIOBJFIRST..CF_GDIOBJLAST
}

fn bytes_of(wide: &[u16]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2) }
}

fn wait_until(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if done() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        sleep(Duration::from_millis(10));
    }
}

fn key(vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn chord(modifier: VIRTUAL_KEY, vk: VIRTUAL_KEY) {
    send_keys(&[
        key(modifier, 0),
        key(vk, 0),
        key(vk, KEYEVENTF_KEYUP),
        key(modifier, KEYEVENTF_KEYUP),
    ]);
}

fn send_keys(inputs: &[INPUT]) {
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
}

/// `s` as a nul-terminated UTF-16 string.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
