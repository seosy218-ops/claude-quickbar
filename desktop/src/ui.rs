//! The overlay: a small topmost window with a ⚡ that folds out one button per phrase.
//! Only the buttons show, as chips floating on Claude: the gaps between them are clear
//! and clicks there land on Claude. It never takes focus, so Claude keeps its caret
//! while the user clicks.
//! It rides on Claude's window and shows only while Claude is in front,
//! driven by system window events rather than polling.
//! Holding the ⚡ drags the whole bar; holding a phrase drags it to another place
//! among the phrases, the others making way as it goes.

use std::cell::RefCell;
use std::time::Duration;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CHILDID_SELF, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, DispatchMessageW, EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_FOREGROUND,
    EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART, GetCursorPos, GetForegroundWindow,
    GetMessageW, GetSystemMetrics, GetWindowThreadProcessId, IDC_ARROW, IsIconic, IsWindowVisible,
    KillTimer, LoadCursorW, MA_NOACTIVATE, MB_ICONWARNING, MF_CHECKED, MF_SEPARATOR, MF_STRING,
    MF_UNCHECKED, MSG, MessageBoxW, OBJID_WINDOW, PostMessageW, PostQuitMessage, RegisterClassW,
    SM_CXDRAG, SM_CYDRAG, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS, WM_APP, WM_CAPTURECHANGED, WM_DESTROY, WM_DPICHANGED, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NULL, WM_RBUTTONUP, WM_TIMER, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WindowFromPoint,
};

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};

use std::io::ErrorKind;

use crate::autostart;
use crate::config::{Anchor, Config};
use crate::dialog;
use crate::menu::{self, Item};
use crate::paint::{Color, FONT, Font, GdiPlus, ICON_FONT, Layer, Palette, scale};
use crate::send::{Phrase, SendError, send};
use crate::theme::{self, Theme};
use crate::tray::{self, Tray};
use crate::win::{
    WinHost, find_claude, frame, is_claude_process, is_live, is_ours, point_of, wide, window_rect,
};

/// Posted by the send thread when it is done; `wparam` is one of the `SENT` outcomes.
const WM_SENT: u32 = WM_APP + 1;
const SENT: WPARAM = 0;
const SENT_NO_CLAUDE: WPARAM = 1;
const SENT_NO_PROMPT: WPARAM = 2;
/// Posted to lay the bar out again once the message that asked for it has returned.
const WM_RELAYOUT: u32 = WM_APP + 2;
/// Posted to open the phrase box once the click that asked for it has returned;
/// `wparam` is the phrase's index plus one, or 0 for a new phrase.
const WM_ASK: u32 = WM_APP + 3;
/// Sent by the tray icon; `lparam` is the mouse message.
const WM_TRAY: u32 = WM_APP + 4;
/// Posted by the theme watcher; `wparam` is the new theme.
const WM_THEME: u32 = WM_APP + 5;
const STATUS_TIMER: usize = 1;
const STATUS_TIME: Duration = Duration::from_millis(2500);
const TRAY_TIMER: usize = 2;
const TRAY_RETRY: Duration = Duration::from_secs(1);
const CLAUDE_TIMER: usize = 3;
const CLAUDE_LOOKUP_EVERY: Duration = Duration::from_millis(500);
/// Looks for Claude's main window per foreground change, when Claude is in front without one.
const CLAUDE_LOOKUPS: u8 = 10;
const MENU_QUIT: usize = 1;
const MENU_EDIT: usize = 2;
const MENU_DELETE: usize = 3;
const MENU_AUTOSTART: usize = 4;

// Layout in 96-dpi pixels, after the pills above Claude's prompt (Local, the folder):
// 24 high, 6 in from each side, 6 apart, corners of 6, 13px text, 16px icons.
// The ⚡ and the + are an icon alone, 6 + 16 + 6 wide, like Claude's folder pill.
/// Clear room around the buttons, for the shadow under them in the light theme.
const PAD: i32 = 1;
const GAP: i32 = 6;
const CHIP_HEIGHT: i32 = 24;
const CHIP_PAD_X: i32 = 6;
const CHIP_RADIUS: i32 = 6;
const CHIP_SHADOW: i32 = 1;
/// Between the warning sign and the message in the status chip.
const STATUS_GAP: i32 = 6;
const FONT_SIZE: i32 = 13;
const ICON_SIZE: i32 = 16;

const BOLT: &str = "\u{e945}";
const ADD: &str = "\u{e710}";
const WARNING: &str = "\u{e7ba}";
/// How much of a phrase button shows while a phrase is being sent (40%).
const SENDING_ALPHA: u8 = 0x66;

const CONFIG_INVALID: &str = "Config file is invalid";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Button {
    /// The ⚡, which folds the phrase buttons out and back in.
    Toggle,
    /// An index into the config's phrases.
    Phrase(usize),
    /// The `+` after the phrases, which adds one.
    Add,
}

struct State {
    hwnd: HWND,
    palette: &'static Palette,
    font: Font,
    icon_font: Font,
    config: Config,
    /// Buttons and their rectangles in client pixels, left to right.
    buttons: Vec<(Button, RECT)>,
    expanded: bool,
    /// The config file was broken; say so once the bar is first on screen.
    config_warning: bool,
    /// The bar's size in screen pixels.
    size: SIZE,
    pressed: Option<Button>,
    /// The button under the mouse.
    hot: Option<Button>,
    drag: Option<Drag>,
    sending: bool,
    status: Option<&'static str>,
    /// Claude's main window, once seen; null before that.
    claude: HWND,
    /// Whether one of Claude's windows is in the foreground.
    active: bool,
    shown: bool,
    /// Follows Claude's window around; scoped to Claude's process.
    location_hook: HWINEVENTHOOK,
    watched_pid: u32,
    /// Counts foreground changes, so a stale one handled out of order can be dropped.
    foreground_events: u64,
    /// Further looks for Claude's main window left for the current foreground change.
    claude_lookups: u8,
    /// Taken on the way out, which takes the icon out of the tray.
    tray: Option<Tray>,
    /// What the window shows, at the window's size; none while it cannot be made.
    layer: Option<Layer>,
}

/// A press on the ⚡ or a phrase that may turn into a drag.
struct Drag {
    /// Cursor when the button went down, in screen pixels.
    cursor: POINT,
    grip: Grip,
    moved: bool,
}

/// What a drag takes along.
enum Grip {
    /// The ⚡ moves the bar; its position when the button went down, in screen pixels.
    Bar(POINT),
    /// A phrase changes places.
    Phrase(Reorder),
}

/// A phrase being dragged along the others.
struct Reorder {
    /// The phrase's index, and the place among the phrases it drops into.
    from: usize,
    to: usize,
    /// Where the cursor took hold, from the button's left edge, and where that edge is now,
    /// in client pixels.
    grab: i32,
    left: i32,
}

impl Reorder {
    /// Follows the cursor, at `x` in client pixels, along `buttons` as laid out now:
    /// the button stays among the phrases and drops before the first other one whose
    /// middle it has not passed, first or last when past the ends. Returns whether that
    /// place changed.
    fn follow(&mut self, x: i32, buttons: &[(Button, RECT)]) -> bool {
        let phrases = || {
            buttons
                .iter()
                .filter(|(b, _)| matches!(b, Button::Phrase(_)))
        };
        let dragged = Button::Phrase(self.from);
        let Some((_, rect)) = phrases().find(|&&(b, _)| b == dragged) else {
            return false;
        };
        let width = rect.right - rect.left;
        let (first, last) = phrases().fold((i32::MAX, i32::MIN), |(first, last), (_, r)| {
            (first.min(r.left), last.max(r.right))
        });
        let left = x - self.grab;
        let middle = left + width / 2;
        let to = phrases()
            .filter(|&&(b, r)| b != dragged && (r.left + r.right) / 2 < middle)
            .count();
        self.left = left.min(last - width).max(first);
        std::mem::replace(&mut self.to, to) != to
    }
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

pub fn run() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // Without it the buttons lose their round corners but the bar still works.
        let _gdiplus = GdiPlus::start();
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide("quickbar");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        });
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }
        let palette = theme::follow(hwnd, WM_THEME).palette();
        menu::set_theme(palette);
        dialog::set_theme(palette);
        let config = Config::load();
        STATE.with_borrow_mut(|state| {
            *state = Some(State {
                hwnd,
                palette,
                font: bar_font(hwnd, FONT, FONT_SIZE),
                icon_font: bar_font(hwnd, ICON_FONT, ICON_SIZE),
                config_warning: config.is_unreadable(),
                config,
                buttons: Vec::new(),
                expanded: false,
                size: SIZE { cx: 1, cy: 1 },
                pressed: None,
                hot: None,
                drag: None,
                sending: false,
                status: None,
                claude: std::ptr::null_mut(),
                active: false,
                shown: false,
                location_hook: std::ptr::null_mut(),
                watched_pid: 0,
                foreground_events: 0,
                // Started at sign-in, Claude may be coming up already.
                claude_lookups: CLAUDE_LOOKUPS,
                tray: Some(Tray::new(hwnd, WM_TRAY, palette)),
                layer: None,
            })
        });
        with_state(State::layout);
        with_state(State::show_tray);
        // Our own windows (the right-click menu) taking the foreground must not hide the bar.
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        for (first, last) in [
            (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
            (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
        ] {
            let hook = SetWinEventHook(
                first,
                last,
                std::ptr::null_mut(),
                Some(on_win_event),
                0,
                0,
                flags,
            );
            // Without these the bar would never know when to show.
            if hook.is_null() {
                return;
            }
        }
        foreground_changed(GetForegroundWindow());
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Runs `f` on the state. Messages sent to the bar while the state is already
/// borrowed (say, by our own `SetWindowPos`) are ignored rather than panicking.
fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|state| state.try_borrow_mut().ok()?.as_mut().map(f))
}

impl State {
    fn scale(&self, px: i32) -> i32 {
        scale(self.hwnd, px)
    }

    /// The window's height: one row of buttons and the pad around it.
    fn height(&self) -> i32 {
        self.scale(CHIP_HEIGHT) + 2 * self.scale(PAD)
    }

    /// Sizes the window to fit its buttons, or the status message while one is shown.
    fn layout(&mut self) {
        // The DPI may have changed.
        self.font = bar_font(self.hwnd, FONT, FONT_SIZE);
        self.icon_font = bar_font(self.hwnd, ICON_FONT, ICON_SIZE);
        let (pad, gap, height) = (self.scale(PAD), self.scale(GAP), self.height());
        let mut x = pad;
        self.buttons.clear();
        if let Some(status) = self.status {
            x = self.status_parts(status).0.right;
        } else {
            let mut order: Vec<Button> = Vec::new();
            if self.expanded {
                let mut phrases: Vec<usize> = (0..self.config.commands.len()).collect();
                // The others make way where the dragged phrase would drop.
                if let Some(&Reorder { from, to, .. }) = self.reorder()
                    && from < phrases.len()
                    && to < phrases.len()
                {
                    let dragged = phrases.remove(from);
                    phrases.insert(to, dragged);
                }
                order.extend(phrases.into_iter().map(Button::Phrase));
                order.push(Button::Add);
            }
            // The ⚡ sits on the pinned side, so it stays put as the phrases fold out.
            if self.config.position.corner.right() {
                order.push(Button::Toggle);
            } else {
                order.insert(0, Button::Toggle);
            }
            for button in order {
                let width = match button {
                    Button::Phrase(i) => {
                        let label = self.config.commands[i].label();
                        self.font.width(label) + 2 * self.scale(CHIP_PAD_X)
                    }
                    Button::Toggle | Button::Add => {
                        self.scale(ICON_SIZE) + 2 * self.scale(CHIP_PAD_X)
                    }
                };
                let rect = RECT {
                    left: x,
                    top: pad,
                    right: x + width,
                    bottom: height - pad,
                };
                self.buttons.push((button, rect));
                x += width + gap;
            }
            x -= gap;
        }
        self.size = SIZE {
            cx: x + pad,
            cy: height,
        };
        self.place();
        self.redraw();
        // Buttons moved under a mouse that stayed put.
        self.refresh_hot();
    }

    /// The label of `button`: its font, text and color.
    fn button_face(&self, button: Button) -> (&Font, &str, Color) {
        match button {
            Button::Toggle => (&self.icon_font, BOLT, self.palette.text_secondary),
            Button::Phrase(i) => (
                &self.font,
                self.config.commands[i].label(),
                self.palette.text_secondary,
            ),
            Button::Add => (&self.icon_font, ADD, self.palette.text_secondary),
        }
    }

    /// Notes which button the mouse is over now, repainting when that changed.
    fn refresh_hot(&mut self) {
        let hot = unsafe {
            let mut at = POINT { x: 0, y: 0 };
            GetCursorPos(&mut at);
            // Another window may be over the bar, or the bar hidden.
            if WindowFromPoint(at) == self.hwnd {
                ScreenToClient(self.hwnd, &mut at);
                self.button_under(at)
            } else {
                None
            }
        };
        if hot != self.hot {
            self.hot = hot;
            self.redraw();
        }
    }

    /// Puts the bar at its anchor on Claude's window; only resizes it while
    /// Claude is unknown or the user is moving the bar.
    fn place(&self) {
        let dragging = matches!(
            self.drag,
            Some(Drag {
                grip: Grip::Bar(_),
                moved: true,
                ..
            })
        );
        let target = self.claude_frame().filter(|_| !dragging).map(|claude| {
            let dpi = unsafe { GetDpiForWindow(self.claude) };
            let (x, y) = self
                .config
                .position
                .place(self.size.cx, self.size.cy, claude.into(), dpi);
            POINT { x, y }
        });
        let (at, flags) = match target {
            Some(at) => (at, 0),
            None => (POINT { x: 0, y: 0 }, SWP_NOMOVE),
        };
        unsafe {
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                at.x,
                at.y,
                self.size.cx,
                self.size.cy,
                flags | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
    }

    /// Shows the bar at its place while Claude is in front and not minimized; hides it otherwise.
    fn follow_claude(&mut self) {
        let claude = self.claude;
        let show = self.active
            && is_live(claude)
            && unsafe { IsWindowVisible(claude) != 0 && IsIconic(claude) == 0 };
        if show {
            self.place();
        } else if self.drag.is_some() || self.pressed.is_some() {
            // A hidden bar gets no button-up; end the press here.
            self.cancel_drag();
            unsafe { ReleaseCapture() };
        }
        if show && std::mem::take(&mut self.config_warning) {
            self.show_status(CONFIG_INVALID);
        }
        if show != self.shown {
            self.shown = show;
            unsafe { ShowWindow(self.hwnd, if show { SW_SHOWNOACTIVATE } else { SW_HIDE }) };
            self.refresh_hot();
        }
    }

    fn claude_frame(&self) -> Option<RECT> {
        is_live(self.claude).then(|| frame(self.claude)).flatten()
    }

    /// Moves or resizes of Claude's window arrive from its process only.
    fn watch(&mut self, pid: u32) {
        if pid == self.watched_pid {
            return;
        }
        unsafe {
            if !self.location_hook.is_null() {
                UnhookWinEvent(self.location_hook);
            }
            self.location_hook = SetWinEventHook(
                EVENT_OBJECT_LOCATIONCHANGE,
                EVENT_OBJECT_LOCATIONCHANGE,
                std::ptr::null_mut(),
                Some(on_win_event),
                pid,
                0,
                WINEVENT_OUTOFCONTEXT,
            );
        }
        // On failure, try again on the next foreground change.
        self.watched_pid = if self.location_hook.is_null() { 0 } else { pid };
    }

    /// Once the cursor has gone far enough to count as a drag, moves the bar with it,
    /// or the held phrase along the others.
    fn drag_with_cursor(&mut self) {
        let Some(drag) = &mut self.drag else { return };
        let mut at = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut at) };
        let (dx, dy) = (at.x - drag.cursor.x, at.y - drag.cursor.y);
        if !drag.moved {
            let (min_x, min_y) =
                unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
            if dx.abs() < min_x && dy.abs() < min_y {
                return;
            }
            drag.moved = true;
        }
        let mut relayout = false;
        match &mut drag.grip {
            Grip::Bar(origin) => {
                // The held phrase shows as held while it moves; the ⚡ does not.
                self.pressed = None;
                unsafe {
                    SetWindowPos(
                        self.hwnd,
                        std::ptr::null_mut(),
                        origin.x + dx,
                        origin.y + dy,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                };
            }
            Grip::Phrase(reorder) => {
                unsafe { ScreenToClient(self.hwnd, &mut at) };
                relayout = reorder.follow(at.x, &self.buttons);
            }
        }
        if relayout {
            self.layout();
        }
        self.redraw();
    }

    /// The phrase being dragged to another place, once the cursor has gone far enough.
    fn reorder(&self) -> Option<&Reorder> {
        match &self.drag {
            Some(Drag {
                grip: Grip::Phrase(reorder),
                moved: true,
                ..
            }) => Some(reorder),
            _ => None,
        }
    }

    /// Remembers where the user dropped the bar, relative to the nearest corner of Claude's window.
    fn pin(&mut self) {
        let (Some(claude), Some(bar)) = (self.claude_frame(), window_rect(self.hwnd)) else {
            return;
        };
        let dpi = unsafe { GetDpiForWindow(self.claude) };
        self.config.position = Anchor::pin(bar.into(), claude.into(), dpi);
        // The ⚡ moves to whichever side is now pinned.
        self.layout();
        self.save("Position not saved");
    }

    /// Puts `phrase` at `index`, or after the others when `index` is `None`.
    /// Does nothing when there is no longer a phrase at `index`.
    fn set_phrase(&mut self, index: Option<usize>, phrase: Phrase) {
        match index {
            Some(i) => match self.config.commands.get_mut(i) {
                Some(old) => *old = phrase,
                None => return,
            },
            None => self.config.commands.push(phrase),
        }
        self.layout();
        self.save("Phrases not saved");
    }

    fn delete_phrase(&mut self, i: usize) {
        if i < self.config.commands.len() {
            self.config.commands.remove(i);
            self.layout();
            self.save("Phrases not saved");
        }
    }

    /// Puts phrase `from` at place `to` among the phrases, where it was dropped.
    fn move_phrase(&mut self, from: usize, to: usize) {
        let count = self.config.commands.len();
        let moves = from != to && from < count && to < count;
        if moves {
            let phrase = self.config.commands.remove(from);
            self.config.commands.insert(to, phrase);
        }
        // The buttons were laid out for the drag.
        self.layout();
        if moves {
            self.save("Phrases not saved");
        }
    }

    fn save(&mut self, failed: &'static str) {
        match self.config.save() {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::InvalidData => self.show_status(CONFIG_INVALID),
            Err(_) => self.show_status(failed),
        }
    }

    /// The press was cut short (Alt+Tab, another window took the mouse): put the bar
    /// or the phrases back.
    fn cancel_drag(&mut self) {
        let drag = self.drag.take();
        self.pressed = None;
        match drag {
            Some(Drag {
                grip: Grip::Bar(_),
                moved: true,
                ..
            }) => self.place(),
            Some(Drag {
                grip: Grip::Phrase(_),
                moved: true,
                ..
            }) => self.layout(),
            _ => {}
        }
        self.redraw();
    }

    /// The status chip, and where its warning sign and message go inside it.
    fn status_parts(&self, status: &str) -> (RECT, RECT, RECT) {
        let (pad, pad_x) = (self.scale(PAD), self.scale(CHIP_PAD_X));
        let (top, bottom) = (pad, self.height() - pad);
        let icon_left = pad + pad_x;
        let icon = RECT {
            left: icon_left,
            top,
            right: icon_left + self.scale(ICON_SIZE),
            bottom,
        };
        let text_left = icon.right + self.scale(STATUS_GAP);
        let text = RECT {
            left: text_left,
            top,
            right: text_left + self.font.width(status),
            bottom,
        };
        let chip = RECT {
            left: pad,
            top,
            right: text.right + pad_x,
            bottom,
        };
        (chip, icon, text)
    }

    /// Draws the bar afresh and puts it up. A layered window gets no `WM_PAINT`,
    /// so whatever changes its look calls this.
    fn redraw(&mut self) {
        let mut layer = match self.layer.take() {
            Some(layer) if layer.fits(self.size) => layer,
            _ => match Layer::new(self.size) {
                Some(layer) => layer,
                None => return,
            },
        };
        layer.clear();
        self.draw(&mut layer);
        layer.present(self.hwnd);
        self.layer = Some(layer);
    }

    /// One chip's block: its shadow, its fill and its ring.
    fn chip(&self, layer: &mut Layer, rect: &RECT, fill: Color) {
        let palette = self.palette;
        let radius = self.scale(CHIP_RADIUS);
        if let Some(color) = palette.pill_shadow {
            let shadow = self.scale(CHIP_SHADOW);
            let below = RECT {
                top: rect.top + shadow,
                bottom: rect.bottom + shadow,
                ..*rect
            };
            layer.rounded(&below, radius, Some(color), None);
        }
        layer.rounded(rect, radius, Some(fill), palette.pill_ring);
    }

    fn draw(&self, layer: &mut Layer) {
        let palette = self.palette;
        if let Some(status) = self.status {
            let (chip, icon, text) = self.status_parts(status);
            self.chip(layer, &chip, palette.pill);
            layer.text(&self.icon_font, palette.text_warning, WARNING, &icon);
            layer.text(&self.font, palette.text_warning, status, &text);
            return;
        }
        // Clicks do nothing while sending or dragging, so nothing lights up.
        let still = !self.sending && self.drag.is_none();
        // The dragged phrase sits where the cursor has it, over the others.
        let floating = self.reorder().and_then(|reorder| {
            let button = Button::Phrase(reorder.from);
            let rect = self.rect_of(button)?;
            let left = reorder.left;
            let right = left + rect.right - rect.left;
            Some((
                button,
                RECT {
                    left,
                    right,
                    ..rect
                },
            ))
        });
        let buttons = self.buttons.iter().copied();
        let buttons = buttons
            .filter(|&(b, _)| floating.is_none_or(|(f, _)| f != b))
            .chain(floating);
        for (button, ref rect) in buttons {
            // Held down looks as under the mouse, as in Claude.
            let lit = self.pressed == Some(button) || (still && self.hot == Some(button));
            let fill = if lit {
                palette.pill_hover
            } else {
                palette.pill
            };
            self.chip(layer, rect, fill);
            let (font, label, color) = self.button_face(button);
            // While sending, phrase buttons show as a whole at 40%, like Claude's disabled
            // ones. ClearType's color fringes would show through on what is below, so
            // their text is plain anti-aliased.
            if self.sending && matches!(button, Button::Phrase(_)) {
                layer.text_gray(font, color, label, rect);
                let with_shadow = RECT {
                    bottom: rect.bottom + self.scale(CHIP_SHADOW),
                    ..*rect
                };
                layer.fade(&with_shadow, SENDING_ALPHA);
            } else {
                layer.text(font, color, label, rect);
            }
        }
    }

    fn rect_of(&self, button: Button) -> Option<RECT> {
        self.buttons
            .iter()
            .find(|&&(b, _)| b == button)
            .map(|&(_, rect)| rect)
    }

    fn button_at(&self, lparam: LPARAM) -> Option<Button> {
        self.button_under(point_of(lparam))
    }

    /// The button at `at`, in client pixels.
    fn button_under(&self, POINT { x, y }: POINT) -> Option<Button> {
        self.buttons
            .iter()
            .find(|(_, r)| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
            .map(|&(button, _)| button)
    }

    fn start_send(&mut self, i: usize) {
        self.sending = true;
        let phrase = self.config.commands[i].clone();
        let hwnd = self.hwnd as usize;
        std::thread::spawn(move || {
            let result = send(&mut WinHost::new(hwnd as HWND), &phrase);
            let outcome = match result {
                Ok(()) => SENT,
                Err(SendError::ClaudeNotFound) => SENT_NO_CLAUDE,
                Err(SendError::PromptNotFound) => SENT_NO_PROMPT,
            };
            unsafe { PostMessageW(hwnd as HWND, WM_SENT, outcome, 0) };
        });
    }

    /// Puts the icon in the tray, trying again every so often while the taskbar is not taking it.
    fn show_tray(&mut self) {
        let Some(tray) = &self.tray else { return };
        unsafe {
            if tray.add() {
                KillTimer(self.hwnd, TRAY_TIMER);
            } else {
                SetTimer(self.hwnd, TRAY_TIMER, TRAY_RETRY.as_millis() as u32, None);
            }
        }
    }

    /// Claude went light or dark: the bar and the tray icon follow.
    fn set_theme(&mut self, theme: Theme) {
        self.palette = theme.palette();
        menu::set_theme(self.palette);
        dialog::set_theme(self.palette);
        if let Some(tray) = &mut self.tray {
            tray.repaint(self.palette);
        }
        self.redraw();
    }

    fn show_status(&mut self, status: &'static str) {
        self.status = Some(status);
        self.layout();
        unsafe {
            SetTimer(
                self.hwnd,
                STATUS_TIMER,
                STATUS_TIME.as_millis() as u32,
                None,
            )
        };
    }
}

/// Another window came to the front: note whether it is Claude's, and which window is Claude's.
fn foreground_changed(foreground: HWND) {
    let Some((event, known)) = with_state(|s| {
        s.foreground_events += 1;
        (s.foreground_events, s.claude)
    }) else {
        return;
    };
    // Looking at other processes' windows may let further events in, so do it
    // outside the state and drop the answer if a newer change came meanwhile.
    let ours = !foreground.is_null() && unsafe { is_claude_process(foreground) };
    let claude = if !ours {
        None
    } else if is_live(known) && unsafe { IsWindowVisible(known) } != 0 {
        Some(known)
    } else {
        find_claude()
    };
    with_state(|s| {
        if s.foreground_events != event {
            return;
        }
        // Claude came to the front before its main window is up (no title, still hidden):
        // no further event says when it is, so look again shortly, a few times.
        if ours && claude.is_none() && s.claude_lookups > 0 {
            s.claude_lookups -= 1;
            unsafe {
                SetTimer(
                    s.hwnd,
                    CLAUDE_TIMER,
                    CLAUDE_LOOKUP_EVERY.as_millis() as u32,
                    None,
                )
            };
        }
        s.active = claude.is_some();
        if let Some(claude) = claude {
            s.claude = claude;
            let mut pid = 0;
            unsafe { GetWindowThreadProcessId(claude, &mut pid) };
            s.watch(pid);
        }
        s.follow_claude();
    });
}

unsafe extern "system" fn on_win_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _thread: u32,
    _time: u32,
) {
    match event {
        EVENT_SYSTEM_FOREGROUND => {
            with_state(|s| s.claude_lookups = CLAUDE_LOOKUPS);
            foreground_changed(hwnd);
        }
        // The bar stays up while Claude animates down; hide it now.
        EVENT_SYSTEM_MINIMIZESTART => {
            with_state(|s| {
                if s.claude == hwnd {
                    s.active = false;
                    s.follow_claude();
                }
            });
        }
        EVENT_SYSTEM_MINIMIZEEND => foreground_changed(unsafe { GetForegroundWindow() }),
        EVENT_OBJECT_LOCATIONCHANGE if object == OBJID_WINDOW && child == CHILDID_SELF as i32 => {
            with_state(|s| {
                if s.claude == hwnd {
                    s.follow_claude();
                }
            });
        }
        _ => {}
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => return MA_NOACTIVATE as LRESULT,
            WM_LBUTTONDOWN => {
                with_state(|s| {
                    let Some(button) = s.button_at(lparam).filter(|_| !s.sending) else {
                        return;
                    };
                    let grip = match button {
                        Button::Toggle => window_rect(hwnd).map(|bar| {
                            Grip::Bar(POINT {
                                x: bar.left,
                                y: bar.top,
                            })
                        }),
                        Button::Phrase(from) => s.rect_of(button).map(|rect| {
                            Grip::Phrase(Reorder {
                                from,
                                to: from,
                                grab: point_of(lparam).x - rect.left,
                                left: rect.left,
                            })
                        }),
                        Button::Add => None,
                    };
                    let mut cursor = POINT { x: 0, y: 0 };
                    GetCursorPos(&mut cursor);
                    s.pressed = Some(button);
                    s.drag = grip.map(|grip| Drag {
                        cursor,
                        grip,
                        moved: false,
                    });
                    SetCapture(hwnd);
                    s.redraw();
                });
                return 0;
            }
            WM_MOUSEMOVE => {
                // Asked again on every move: capture or hiding may have spent the last request.
                let mut leave = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut leave);
                with_state(|s| {
                    s.drag_with_cursor();
                    s.refresh_hot();
                });
                return 0;
            }
            WM_MOUSELEAVE => {
                with_state(State::refresh_hot);
                return 0;
            }
            WM_LBUTTONUP => {
                // Take the drag and the press before letting go of the mouse,
                // or WM_CAPTURECHANGED cancels them.
                let (drag, pressed) =
                    with_state(|s| (s.drag.take(), s.pressed.take())).unwrap_or((None, None));
                ReleaseCapture();
                with_state(|s| {
                    if let Some(Drag {
                        grip, moved: true, ..
                    }) = drag
                    {
                        match grip {
                            Grip::Bar(_) => s.pin(),
                            Grip::Phrase(Reorder { from, to, .. }) => s.move_phrase(from, to),
                        }
                    } else if let Some(button) = pressed
                        && s.button_at(lparam) == Some(button)
                    {
                        match button {
                            Button::Toggle => {
                                s.expanded = !s.expanded;
                                s.layout();
                            }
                            Button::Phrase(i) => s.start_send(i),
                            Button::Add => {
                                PostMessageW(hwnd, WM_ASK, 0, 0);
                            }
                        }
                    }
                    // The press is over: take its look away and let hover show again.
                    s.refresh_hot();
                    s.redraw();
                });
                return 0;
            }
            WM_CAPTURECHANGED => {
                with_state(State::cancel_drag);
                return 0;
            }
            WM_SENT => {
                with_state(|s| {
                    s.sending = false;
                    match wparam {
                        SENT_NO_CLAUDE => s.show_status("Claude not found"),
                        SENT_NO_PROMPT => s.show_status("No prompt box to type in"),
                        _ => {}
                    }
                    s.redraw();
                });
                return 0;
            }
            WM_TIMER if wparam == STATUS_TIMER => {
                KillTimer(hwnd, STATUS_TIMER);
                with_state(|s| {
                    s.status = None;
                    s.layout();
                });
                return 0;
            }
            WM_TIMER if wparam == CLAUDE_TIMER => {
                KillTimer(hwnd, CLAUDE_TIMER);
                foreground_changed(GetForegroundWindow());
                return 0;
            }
            WM_TIMER if wparam == TRAY_TIMER => {
                with_state(State::show_tray);
                return 0;
            }
            WM_RBUTTONUP => {
                let phrase = with_state(|s| match s.button_at(lparam) {
                    Some(Button::Phrase(i)) => Some(i),
                    _ => None,
                });
                bar_menu(hwnd, phrase.flatten());
                return 0;
            }
            WM_TRAY => {
                if matches!(lparam as u32, WM_LBUTTONUP | WM_RBUTTONUP) {
                    tray_menu(hwnd);
                }
                return 0;
            }
            // Arrives from inside our own SetWindowPos, while the state is borrowed.
            // The bar goes where its anchor says rather than where Windows suggests.
            WM_DPICHANGED => {
                PostMessageW(hwnd, WM_RELAYOUT, 0, 0);
                return 0;
            }
            WM_RELAYOUT => {
                with_state(State::layout);
                return 0;
            }
            WM_ASK => {
                ask(hwnd, wparam.checked_sub(1));
                return 0;
            }
            WM_THEME => {
                with_state(|s| s.set_theme(Theme::from_message(wparam)));
                return 0;
            }
            WM_DESTROY => {
                drop(with_state(|s| s.tray.take()));
                PostQuitMessage(0);
                return 0;
            }
            _ if Some(msg) == tray::taskbar_created() => {
                with_state(State::show_tray);
                return 0;
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

/// The bar's right-click menu; on a phrase button, `phrase` is that phrase's index.
fn bar_menu(hwnd: HWND, phrase: Option<usize>) {
    let mut items = Vec::new();
    if phrase.is_some() {
        items.extend([
            Item::entry(MENU_EDIT, "Edit"),
            Item::entry(MENU_DELETE, "Delete").danger(),
            Item::Separator,
        ]);
    }
    items.push(Item::entry(MENU_QUIT, "Quit"));
    let mut at = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut at) };
    match (menu::pick(hwnd, at, &items), phrase) {
        (Some(MENU_QUIT), _) => unsafe {
            DestroyWindow(hwnd);
        },
        // The box takes the foreground and hands it back when done.
        (Some(MENU_EDIT), Some(i)) => unsafe {
            PostMessageW(hwnd, WM_ASK, i + 1, 0);
        },
        (Some(MENU_DELETE), Some(i)) => {
            with_state(|s| s.delete_phrase(i));
            give_back_focus();
        }
        // Closed without a pick: the foreground stays wherever the user went meanwhile.
        _ => {
            let front = unsafe { GetForegroundWindow() };
            if front.is_null() || is_ours(front) {
                give_back_focus();
            }
        }
    }
}

/// The tray icon's menu, which keeps the system's own look.
unsafe fn tray_menu(hwnd: HWND) {
    unsafe {
        let menu = CreatePopupMenu();
        let (autostart_label, quit) = (wide("Start with Windows"), wide("Quit"));
        let on = autostart::is_on();
        let checked = if on { MF_CHECKED } else { MF_UNCHECKED };
        AppendMenuW(
            menu,
            MF_STRING | checked,
            MENU_AUTOSTART,
            autostart_label.as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, MENU_QUIT, quit.as_ptr());
        let mut at = POINT { x: 0, y: 0 };
        GetCursorPos(&mut at);
        // The menu only closes on an outside click if its owner is in the foreground.
        SetForegroundWindow(hwnd);
        let picked = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            at.x,
            at.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        match picked as usize {
            MENU_QUIT => {
                DestroyWindow(hwnd);
            }
            MENU_AUTOSTART if autostart::set(!on).is_err() => {
                // The bar is usually hidden while the tray is in use, so a status would go unseen.
                let (text, caption) = (
                    wide("Could not change the startup setting."),
                    wide("quickbar"),
                );
                MessageBoxW(hwnd, text.as_ptr(), caption.as_ptr(), MB_ICONWARNING);
            }
            _ => {}
        }
    }
}

/// A font of the bar, `size` 96-dpi pixels tall, for the screen `hwnd` is on.
fn bar_font(hwnd: HWND, face: &str, size: i32) -> Font {
    Font::new(face, scale(hwnd, size))
}

/// Opens the phrase box for phrase `index`, or for a new one when `None`,
/// and applies the answer. Runs outside the state: the box has its own message loop.
fn ask(hwnd: HWND, index: Option<usize>) {
    // A second click queued up before the first box disabled the bar.
    if dialog::is_open() {
        return;
    }
    let initial = match index {
        // Gone when the phrase went away before the box opened.
        Some(i) => match with_state(|s| s.config.commands.get(i).cloned()).flatten() {
            Some(phrase) => Some(phrase),
            None => return,
        },
        None => None,
    };
    let answer = dialog::ask(hwnd, initial.as_ref());
    if let Some(phrase) = answer {
        with_state(|s| s.set_phrase(index, phrase));
    }
    give_back_focus();
}

/// Our box or menu had the foreground; hand it back to Claude so the user can type on.
fn give_back_focus() {
    with_state(|s| {
        if is_live(s.claude) {
            unsafe { SetForegroundWindow(s.claude) };
        }
    });
}
