//! The small box for adding or changing a phrase: the phrase, an optional
//! label, and send or fill. OK stays greyed out while the phrase is empty.
//! Drawn after Claude's dialogs. The two fields are system EDITs in frames drawn
//! here; the switch and the buttons are drawn and worked here, as on the bar.

use std::cell::{Cell, RefCell};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    DT_LEFT, GetDC, HDC, HFONT, InvalidateRect, ReleaseDC, ScreenToClient, SelectObject,
    SetBkColor, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    EM_CANUNDO, EM_GETRECT, EM_GETSEL, EM_SETMARGINS, EM_SETSEL, EM_UNDO, WM_MOUSELEAVE,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent, VK_ESCAPE, VK_RETURN, VK_SHIFT, VK_SPACE, VK_TAB,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    EC_LEFTMARGIN, EC_RIGHTMARGIN, EN_CHANGE, EN_SETFOCUS, ES_AUTOHSCROLL, GWLP_WNDPROC,
    GetCursorPos, GetDlgCtrlID, GetForegroundWindow, GetMessageW, GetParent, GetWindowTextLengthW,
    GetWindowTextW, HMENU, HTCAPTION, HTCLIENT, HideCaret, IDC_ARROW, IDC_IBEAM, LoadCursorW, MSG,
    PostQuitMessage, RegisterClassW, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW,
    SetCursor, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowCaret, ShowWindow,
    TranslateMessage, WA_INACTIVE, WM_ACTIVATE, WM_CAPTURECHANGED, WM_CLOSE, WM_COMMAND,
    WM_CONTEXTMENU, WM_COPY, WM_CTLCOLOREDIT, WM_CUT, WM_DPICHANGED, WM_ERASEBKGND, WM_GETFONT,
    WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCHITTEST,
    WM_NCMOUSEMOVE, WM_PAINT, WM_PASTE, WM_RBUTTONDOWN, WM_SETCURSOR, WM_SETFOCUS, WM_SETFONT,
    WNDCLASSW, WNDPROC, WS_CHILD, WS_CLIPCHILDREN, WS_EX_TOPMOST, WS_POPUP, WS_TABSTOP, WS_VISIBLE,
    WindowFromPoint,
};

use crate::menu::{self, Item};
use crate::paint::{self, Align, Brush, Canvas, Color, FONT, FONT_SEMIBOLD, Font, Palette, scale};
use crate::send::{Mode, Phrase};
use crate::win::{clipboard_has_text, is_ours, point_of, wide, window_rect, work_area};

const PHRASE: i32 = 100;
const LABEL: i32 = 101;

// The fields' right-click menu.
const EDIT_UNDO: usize = 1;
const EDIT_CUT: usize = 2;
const EDIT_COPY: usize = 3;
const EDIT_PASTE: usize = 4;
const EDIT_SELECT_ALL: usize = 5;

// Layout in 96-dpi pixels, after Claude's compact dialogs.
const WIDTH: i32 = 360;
/// Around everything, the 1px window edge included.
const PAD: i32 = 20;
const TITLE: i32 = 24;
const TITLE_GAP: i32 = 8;
/// A field's name above it, and the room between the two.
const CAPTION: i32 = 13;
const CAPTION_GAP: i32 = 6;
/// Fields and buttons: how tall, their corners, the room left and right of their text.
const CONTROL: i32 = 24;
const RADIUS: i32 = 6;
const PAD_X: i32 = 8;
/// Between the fields, the switch's row and the buttons.
const GAP: i32 = 12;
const SWITCH_ROW: i32 = 20;
const SWITCH_WIDTH: i32 = 28;
const SWITCH_HEIGHT: i32 = 16;
/// The switch's knob, and the room around it.
const KNOB: i32 = 12;
const KNOB_PAD: i32 = 2;
const BUTTON_GAP: i32 = 8;
/// How far below a button or the knob its shadow falls.
const SHADOW: i32 = 1;
const FONT_SIZE: i32 = 13;
const TITLE_SIZE: i32 = 14;
const SEMIBOLD: i32 = 600;
/// Between the cursor and the box.
const OFFSET: i32 = 16;
/// The focus ring's glow, pixel by pixel out from its line: Claude's 6px blur, sampled.
const GLOW: [u8; 6] = [110, 79, 51, 31, 17, 8];

const KNOB_COLOR: Color = Color::rgb(0xff, 0xff, 0xff);
// Claude's is 20% black blurred over 2px; drawn sharp, it takes less.
const KNOB_SHADOW: Color = Color::rgba(0, 0, 0, 0x1a);

const FILL_TEXT: &str = "Fill in only, don't send";
const PHRASE_HINT: &str = "Keep going, then run the tests";
const LABEL_HINT: &str = "Shown on the button";

/// What in the box can be clicked or take the focus, in Tab order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Phrase,
    Label,
    Switch,
    Cancel,
    Ok,
}

const PARTS: [Part; 5] = [
    Part::Phrase,
    Part::Label,
    Part::Switch,
    Part::Cancel,
    Part::Ok,
];

/// Where everything goes, in client pixels.
struct Layout {
    size: SIZE,
    title: RECT,
    /// The fields' names and their frames: the phrase's, then the label's.
    captions: [RECT; 2],
    fields: [RECT; 2],
    /// The switch's row, all of which toggles it, and the switch itself.
    switch_row: RECT,
    switch: RECT,
    cancel: RECT,
    ok: RECT,
}

/// The box's fonts, for the screen it is on.
struct Fonts {
    text: Font,
    /// Field names and OK.
    strong: Font,
    title: Font,
}

/// The box while it is open, and how it was closed.
struct Dialog {
    hwnd: HWND,
    phrase: HWND,
    label: HWND,
    title: &'static str,
    fonts: Fonts,
    layout: Layout,
    /// Behind the fields' text.
    field_brush: Brush,
    /// Fill in only rather than send.
    fill: bool,
    /// OK can be used: there is a phrase.
    can_ok: bool,
    /// Where the keys go. Ringed on the fields always, on the switch and the buttons
    /// only while `keys`, as browsers do.
    focus: Part,
    /// The focus got where it is with Tab rather than the mouse.
    keys: bool,
    /// The box, or our menu over it, is the window in front: focus rings show.
    active: bool,
    hot: Option<Part>,
    /// Held down with the mouse.
    pressed: Option<Part>,
    status: Status,
}

enum Status {
    Open,
    Cancelled,
    Done(Phrase),
}

thread_local! {
    static DIALOG: RefCell<Option<Dialog>> = const { RefCell::new(None) };
    /// The fields' own window procedure, which ours passes everything but the menu on to.
    static FIELD_PROC: Cell<isize> = const { Cell::new(0) };
    static PALETTE: Cell<&'static Palette> = const { Cell::new(&Palette::LIGHT) };
}

/// Colors boxes from now on, and the one open, if any.
pub fn set_theme(palette: &'static Palette) {
    PALETTE.set(palette);
    with_dialog(|d| {
        d.field_brush = Brush::new(field_color(palette));
        paint::shape(d.hwnd, palette.window_border);
        for hwnd in [d.hwnd, d.phrase, d.label] {
            unsafe { InvalidateRect(hwnd, std::ptr::null(), 1) };
        }
    });
}

/// Asks for a phrase, filled in with `initial` when changing one.
/// Blocks until the box is closed; `None` when the user cancels.
/// `owner` is disabled meanwhile.
pub fn ask(owner: HWND, initial: Option<&Phrase>) -> Option<Phrase> {
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide("quickbar-phrase");
        // Fails harmlessly when already registered by an earlier box.
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(dialog_proc),
            hInstance: instance,
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        });
        let title = if initial.is_some() {
            "Edit phrase"
        } else {
            "Add phrase"
        };
        let mut cursor = POINT { x: 0, y: 0 };
        GetCursorPos(&mut cursor);
        // Made at the cursor first, so it is scaled for the screen it opens on.
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST,
            class.as_ptr(),
            wide(title).as_ptr(),
            WS_POPUP | WS_CLIPCHILDREN,
            cursor.x,
            cursor.y,
            1,
            1,
            owner,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return None;
        }
        let field = |id: i32, text: &str| {
            let (class, text) = (wide("EDIT"), wide(text));
            CreateWindowExW(
                0,
                class.as_ptr(),
                text.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
                0,
                0,
                1,
                1,
                hwnd,
                id as isize as HMENU,
                instance,
                std::ptr::null(),
            )
        };
        let phrase = field(PHRASE, initial.map_or("", |c| c.text.as_str()));
        let label = field(
            LABEL,
            initial.and_then(|c| c.label.as_deref()).unwrap_or(""),
        );
        let palette = PALETTE.get();
        let fonts = Fonts::new(hwnd);
        let layout = fonts.layout(hwnd);
        let size = layout.size;
        DIALOG.set(Some(Dialog {
            hwnd,
            phrase,
            label,
            title,
            fonts,
            layout,
            field_brush: Brush::new(field_color(palette)),
            fill: initial.is_some_and(|c| c.mode == Mode::Fill),
            can_ok: false,
            focus: Part::Phrase,
            keys: false,
            active: false,
            hot: None,
            pressed: None,
            status: Status::Open,
        }));
        own_field(phrase);
        own_field(label);
        fit_fields();
        update_ok();
        let at = place(size, cursor, scale(hwnd, OFFSET), work_area(cursor));
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            at.x,
            at.y,
            size.cx,
            size.cy,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        paint::shape(hwnd, palette.window_border);
        EnableWindow(owner, 0);
        ShowWindow(hwnd, SW_SHOW);
        // The user types here, so this one does take the foreground.
        SetForegroundWindow(hwnd);
        move_focus(Part::Phrase);

        let mut msg: MSG = std::mem::zeroed();
        while is_open() {
            match GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) {
                // Leave the quit for the main loop.
                0 => {
                    PostQuitMessage(msg.wParam as i32);
                    break;
                }
                -1 => break,
                _ => {}
            }
            let ours = msg.hwnd == hwnd || GetParent(msg.hwnd) == hwnd;
            if ours && msg.message == WM_KEYDOWN && dialog_key(msg.wParam as u16) {
                continue;
            }
            // The mouse hides the focus ring on the switch and the buttons again.
            if ours && matches!(msg.message, WM_LBUTTONDOWN | WM_RBUTTONDOWN) {
                with_dialog(|d| d.keys = false);
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let status = DIALOG.take().map(|d| d.status);
        // Enable the owner first, or Windows hands the foreground to some other app.
        EnableWindow(owner, 1);
        DestroyWindow(hwnd);
        match status {
            Some(Status::Done(phrase)) => Some(phrase),
            _ => None,
        }
    }
}

/// Whether a box is up and waiting for the user.
pub fn is_open() -> bool {
    DIALOG.with_borrow(|d| d.as_ref().is_some_and(|d| matches!(d.status, Status::Open)))
}

/// Runs `f` on the open box. Messages sent to the box while it is already borrowed
/// are ignored rather than panicking, so nothing that sends one runs inside `f`.
fn with_dialog<R>(f: impl FnOnce(&mut Dialog) -> R) -> Option<R> {
    DIALOG.with(|dialog| dialog.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Where a box of `size` goes: centered `offset` below the cursor `at`, or above it
/// when there is no room below, and inside the `work` area of the cursor's screen.
fn place(size: SIZE, at: POINT, offset: i32, work: RECT) -> POINT {
    let below = at.y + offset;
    let y = if below + size.cy <= work.bottom {
        below
    } else {
        at.y - offset - size.cy
    };
    POINT {
        x: (at.x - size.cx / 2)
            .min(work.right - size.cx)
            .max(work.left),
        y: y.min(work.bottom - size.cy).max(work.top),
    }
}

/// The part Tab moves the focus to from `from`, or Shift+Tab when `back`, going
/// round and passing over OK while it cannot be used.
fn tab(from: Part, back: bool, ok: bool) -> Part {
    let n = PARTS.len();
    let at = PARTS.iter().position(|&p| p == from).unwrap_or(0);
    (1..n)
        .map(|k| PARTS[if back { (at + n - k) % n } else { (at + k) % n }])
        .find(|&p| ok || p != Part::Ok)
        .unwrap_or(from)
}

impl Layout {
    /// With `s` to scale 96-dpi pixels, and the widths of the buttons' text.
    fn new(s: impl Fn(i32) -> i32, cancel_text: i32, ok_text: i32) -> Layout {
        let (left, right) = (s(PAD), s(WIDTH) - s(PAD));
        let mut y = s(PAD);
        let mut row = |height: i32, gap: i32| {
            let rect = RECT {
                left,
                top: y,
                right,
                bottom: y + height,
            };
            y += height + gap;
            rect
        };
        let title = row(s(TITLE), s(TITLE_GAP));
        let phrase_caption = row(s(CAPTION), s(CAPTION_GAP));
        let phrase = row(s(CONTROL), s(GAP));
        let label_caption = row(s(CAPTION), s(CAPTION_GAP));
        let label = row(s(CONTROL), s(GAP));
        let switch_row = row(s(SWITCH_ROW), s(GAP));
        let buttons = row(s(CONTROL), s(PAD));
        let top = switch_row.top + (s(SWITCH_ROW) - s(SWITCH_HEIGHT)) / 2;
        let ok = RECT {
            left: right - ok_text - 2 * s(PAD_X),
            ..buttons
        };
        Layout {
            size: SIZE {
                cx: s(WIDTH),
                cy: y,
            },
            title,
            captions: [phrase_caption, label_caption],
            fields: [phrase, label],
            switch_row,
            switch: RECT {
                left: right - s(SWITCH_WIDTH),
                top,
                right,
                bottom: top + s(SWITCH_HEIGHT),
            },
            cancel: RECT {
                left: ok.left - s(BUTTON_GAP) - cancel_text - 2 * s(PAD_X),
                right: ok.left - s(BUTTON_GAP),
                ..buttons
            },
            ok,
        }
    }

    /// The part at `at`, whether or not it can be used now.
    fn part_at(&self, POINT { x, y }: POINT) -> Option<Part> {
        [
            (Part::Phrase, &self.fields[0]),
            (Part::Label, &self.fields[1]),
            (Part::Switch, &self.switch_row),
            (Part::Cancel, &self.cancel),
            (Part::Ok, &self.ok),
        ]
        .into_iter()
        .find(|(_, r)| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
        .map(|(part, _)| part)
    }
}

impl Fonts {
    fn new(hwnd: HWND) -> Fonts {
        let s = |px| scale(hwnd, px);
        Fonts {
            text: Font::new(FONT, s(FONT_SIZE)),
            strong: Font::weighted(FONT_SEMIBOLD, s(FONT_SIZE), SEMIBOLD),
            title: Font::weighted(FONT_SEMIBOLD, s(TITLE_SIZE), SEMIBOLD),
        }
    }

    fn layout(&self, hwnd: HWND) -> Layout {
        Layout::new(
            |px| scale(hwnd, px),
            self.text.width("Cancel"),
            self.strong.width("OK"),
        )
    }
}

/// Gives the fields the box's font and puts them in their frames, after the box
/// was made or moved to a screen of another scale.
fn fit_fields() {
    let Some((hwnd, fields, frames, font, line)) = with_dialog(|d| {
        let font = &d.fonts.text;
        let fields = [d.phrase, d.label];
        (
            d.hwnd,
            fields,
            d.layout.fields,
            font.handle(),
            font.line_height(),
        )
    }) else {
        return;
    };
    let pad = scale(hwnd, PAD_X);
    for (field, frame) in fields.into_iter().zip(frames) {
        unsafe {
            SendMessageW(field, WM_SETFONT, font as WPARAM, 0);
            // Set after the font, which may bring margins of its own.
            SendMessageW(
                field,
                EM_SETMARGINS,
                (EC_LEFTMARGIN | EC_RIGHTMARGIN) as WPARAM,
                0,
            );
            SetWindowPos(
                field,
                std::ptr::null_mut(),
                frame.left + pad,
                frame.top + (frame.bottom - frame.top - line) / 2,
                frame.right - frame.left - 2 * pad,
                line,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

/// The box moved to a screen of another scale: it and its fonts follow, at
/// `suggested`'s top left corner.
fn rescale(hwnd: HWND, suggested: &RECT) {
    let fonts = Fonts::new(hwnd);
    let layout = fonts.layout(hwnd);
    let size = layout.size;
    let old = with_dialog(|d| {
        d.layout = layout;
        std::mem::replace(&mut d.fonts, fonts)
    });
    fit_fields();
    // Only now that the fields use the new font.
    drop(old);
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            suggested.left,
            suggested.top,
            size.cx,
            size.cy,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

/// Tab, Enter and Esc, wherever the focus is. Whether `key` was one of them.
fn dialog_key(key: u16) -> bool {
    let Some((focus, ok)) = with_dialog(|d| (d.focus, d.can_ok)) else {
        return false;
    };
    match key {
        VK_TAB => {
            let back = unsafe { GetKeyState(VK_SHIFT as i32) } < 0;
            with_dialog(|d| d.keys = true);
            move_focus(tab(focus, back, ok));
        }
        VK_RETURN if focus == Part::Cancel => close(Status::Cancelled),
        // Even from the switch, as Enter sends a form in a browser.
        VK_RETURN => press_ok(),
        VK_ESCAPE => close(Status::Cancelled),
        _ => return false,
    }
    true
}

/// Puts the focus on `part` from the keys: into its field with all the text
/// selected, or on the box itself for the switch and the buttons, whose keys it handles.
fn move_focus(part: Part) {
    let Some((hwnd, field)) = with_dialog(|d| {
        d.focus = part;
        d.redraw();
        (d.hwnd, d.field_of(part))
    }) else {
        return;
    };
    unsafe {
        match field {
            Some(field) => {
                SetFocus(field);
                SendMessageW(field, EM_SETSEL, 0, -1);
            }
            None => {
                SetFocus(hwnd);
            }
        }
    }
}

/// Does what clicking `part` does.
fn act(part: Part) {
    match part {
        Part::Switch => {
            with_dialog(|d| {
                d.fill = !d.fill;
                d.redraw();
            });
        }
        Part::Cancel => close(Status::Cancelled),
        Part::Ok => press_ok(),
        Part::Phrase | Part::Label => {}
    }
}

/// OK, unless there is no phrase yet.
fn press_ok() {
    if let Some(phrase) = read() {
        close(Status::Done(phrase));
    }
}

/// OK is available only while there is a phrase.
fn update_ok() {
    let Some(phrase) = with_dialog(|d| d.phrase) else {
        return;
    };
    let ok = !text_of(phrase).trim().is_empty();
    let changed = with_dialog(|d| {
        let changed = d.can_ok != ok;
        if changed {
            d.can_ok = ok;
            d.redraw();
        }
        changed
    });
    // A greyed OK is not lit under the mouse.
    if changed == Some(true) {
        refresh_hot();
    }
}

/// Notes which part the mouse is over now, repainting when that changed.
fn refresh_hot() {
    let mut at = POINT { x: 0, y: 0 };
    // Asks the windows there where it is on them, so not inside `with_dialog`.
    let under = unsafe {
        GetCursorPos(&mut at);
        WindowFromPoint(at)
    };
    with_dialog(|d| {
        let hot = if under == d.hwnd {
            unsafe { ScreenToClient(d.hwnd, &mut at) };
            d.layout.part_at(at)
        } else if under == d.phrase {
            Some(Part::Phrase)
        } else if under == d.label {
            Some(Part::Label)
        } else {
            None
        };
        let hot = hot.filter(|&part| d.can_ok || part != Part::Ok);
        if hot != d.hot {
            d.hot = hot;
            d.redraw();
        }
    });
}

/// Has `hwnd` told when the mouse leaves it; asked again on every move, as a
/// press or a menu may have spent the last request.
unsafe fn track_leave(hwnd: HWND) {
    let mut leave = TRACKMOUSEEVENT {
        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    unsafe { TrackMouseEvent(&mut leave) };
}

/// The phrase in the box, or `None` while there is none.
fn read() -> Option<Phrase> {
    let (phrase, label, fill) = with_dialog(|d| (d.phrase, d.label, d.fill))?;
    phrase_from(&text_of(phrase), &text_of(label), fill)
}

fn close(status: Status) {
    with_dialog(|d| d.status = status);
}

/// A phrase from what was typed: surrounding spaces trimmed, a blank label
/// meaning none, and no phrase at all without phrase text.
fn phrase_from(text: &str, label: &str, fill: bool) -> Option<Phrase> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let label = label.trim();
    Some(Phrase {
        text: text.into(),
        label: (!label.is_empty()).then(|| label.into()),
        mode: if fill { Mode::Fill } else { Mode::Send },
    })
}

fn text_of(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd).max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        let got = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
        String::from_utf16_lossy(&buf[..got.min(len)])
    }
}

/// `rect` moved `by` pixels down, as a shadow under it.
fn lower(rect: &RECT, by: i32) -> RECT {
    RECT {
        top: rect.top + by,
        bottom: rect.bottom + by,
        ..*rect
    }
}

/// Behind the fields' text: Claude's see-through field laid on the box.
fn field_color(p: &Palette) -> Color {
    p.field.over(p.surface)
}

/// `rect` pushed out by `by` pixels on every side.
fn grow(rect: &RECT, by: i32) -> RECT {
    RECT {
        left: rect.left - by,
        top: rect.top - by,
        right: rect.right + by,
        bottom: rect.bottom + by,
    }
}

impl Dialog {
    fn field_of(&self, part: Part) -> Option<HWND> {
        match part {
            Part::Phrase => Some(self.phrase),
            Part::Label => Some(self.label),
            _ => None,
        }
    }

    fn redraw(&self) {
        unsafe { InvalidateRect(self.hwnd, std::ptr::null(), 0) };
    }

    fn paint(&self) {
        let canvas = Canvas::begin(self.hwnd);
        let p = PALETTE.get();
        let (l, f) = (&self.layout, &self.fonts);
        let s = |px| scale(self.hwnd, px);
        let radius = s(RADIUS);
        canvas.fill(&canvas.bounds(), p.surface);
        canvas.text_aligned(&f.title, p.text, self.title, &l.title, Align::Left);

        // A line of 13px text is taller than the 13px caption: grown alike up and down,
        // so the text stays centered and its descenders are not cut off.
        let [phrase, label] = l.captions.map(|r| RECT {
            top: r.top - s(CAPTION),
            bottom: r.bottom + s(CAPTION),
            ..r
        });
        canvas.text_aligned(&f.strong, p.text, "Phrase", &phrase, Align::Left);
        canvas.text_aligned(&f.strong, p.text, "Label", &label, Align::Left);
        let optional = RECT {
            left: label.left + f.strong.width("Label"),
            ..label
        };
        canvas.text_aligned(&f.text, p.text_muted, " (optional)", &optional, Align::Left);
        let field = field_color(p);
        for (part, frame) in [(Part::Phrase, &l.fields[0]), (Part::Label, &l.fields[1])] {
            let ring = if self.active && self.focus == part {
                self.focus_ring(&canvas, frame, radius);
                p.focus_inset
            } else if self.hot == Some(part) {
                p.border_hover
            } else {
                p.border
            };
            canvas.rounded(frame, radius, Some(field), Some(ring));
        }

        canvas.text_aligned(&f.text, p.text, FILL_TEXT, &l.switch_row, Align::Left);
        let track = match (self.fill, self.hot == Some(Part::Switch)) {
            (true, false) => p.accent,
            (true, true) => p.accent_hover,
            (false, false) => p.track,
            (false, true) => p.track_hover,
        };
        let switch = &l.switch;
        let round = (switch.bottom - switch.top) / 2;
        let inset = self.keyboard_ring(&canvas, Part::Switch, switch, round);
        canvas.rounded(switch, round, Some(track), inset);
        let left = if self.fill {
            switch.right - s(KNOB_PAD) - s(KNOB)
        } else {
            switch.left + s(KNOB_PAD)
        };
        let knob = RECT {
            left,
            top: switch.top + s(KNOB_PAD),
            right: left + s(KNOB),
            bottom: switch.top + s(KNOB_PAD) + s(KNOB),
        };
        canvas.rounded(
            &lower(&knob, s(SHADOW)),
            s(KNOB) / 2,
            Some(KNOB_SHADOW),
            None,
        );
        canvas.rounded(&knob, s(KNOB) / 2, Some(KNOB_COLOR), None);

        // Cancel: the bar's buttons.
        let cancel = &l.cancel;
        let chip = if self.pressed == Some(Part::Cancel) {
            p.chip_down
        } else if self.hot == Some(Part::Cancel) {
            p.chip_hover
        } else {
            p.chip
        };
        match self.keyboard_ring(&canvas, Part::Cancel, cancel, radius) {
            Some(inset) => canvas.rounded(cancel, radius, Some(chip), Some(inset)),
            None => {
                if let Some(shadow) = p.chip_shadow {
                    canvas.rounded(&lower(cancel, s(SHADOW)), radius, Some(shadow), None);
                }
                canvas.rounded(cancel, radius, Some(chip), p.chip_ring);
            }
        }
        canvas.text(&f.text, p.text, "Cancel", cancel);

        let (back, text) = if !self.can_ok {
            (p.primary_off, p.on_primary_off)
        } else if self.pressed == Some(Part::Ok) {
            (p.primary_down, p.on_primary)
        } else if self.hot == Some(Part::Ok) {
            (p.primary_hover, p.on_primary)
        } else {
            (p.primary, p.on_primary)
        };
        let inset = self.keyboard_ring(&canvas, Part::Ok, &l.ok, radius);
        canvas.rounded(&l.ok, radius, Some(back), inset);
        canvas.text(&f.strong, text, "OK", &l.ok);
    }

    /// Claude's focus ring around `rect`, outside it: a 1px accent line and its glow.
    /// The line just inside, in `focus_inset`, is left to whatever is ringed.
    fn focus_ring(&self, canvas: &Canvas, rect: &RECT, radius: i32) {
        let p = PALETTE.get();
        let reach = scale(self.hwnd, GLOW.len() as i32).max(1);
        for i in 0..reach {
            let alpha = GLOW[i as usize * GLOW.len() / reach as usize];
            let out = 2 + i;
            let ring = p.focus_glow.faded(alpha);
            canvas.rounded(&grow(rect, out), radius + out, None, Some(ring));
        }
        canvas.rounded(&grow(rect, 1), radius + 1, None, Some(p.accent));
    }

    /// Rings `part` at `rect` when Tab brought the focus to it, and gives the color of
    /// the line just inside its edge then.
    fn keyboard_ring(
        &self,
        canvas: &Canvas,
        part: Part,
        rect: &RECT,
        radius: i32,
    ) -> Option<Color> {
        (self.active && self.keys && self.focus == part).then(|| {
            self.focus_ring(canvas, rect, radius);
            PALETTE.get().focus_inset
        })
    }
}

/// Gives a field of the box our right-click menu, its placeholder and hover.
unsafe fn own_field(field: HWND) {
    let old = unsafe { SetWindowLongPtrW(field, GWLP_WNDPROC, field_proc as *const () as isize) };
    FIELD_PROC.set(old);
}

unsafe extern "system" fn field_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_CONTEXTMENU => {
                field_menu(hwnd, lparam);
                return 0;
            }
            WM_MOUSEMOVE => {
                track_leave(hwnd);
                refresh_hot();
            }
            WM_MOUSELEAVE => refresh_hot(),
            _ => {}
        }
        let old: WNDPROC = std::mem::transmute(FIELD_PROC.get());
        let result = CallWindowProcW(old, hwnd, msg, wparam, lparam);
        match msg {
            WM_PAINT => placeholder(hwnd),
            // The field redraws its line straight away on these, over the placeholder;
            // while it is empty, have it painted again, placeholder and all.
            WM_SETFOCUS | WM_KILLFOCUS | EM_SETSEL | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_KEYDOWN
                if GetWindowTextLengthW(hwnd) == 0 =>
            {
                InvalidateRect(hwnd, std::ptr::null(), 1);
            }
            _ => {}
        }
        result
    }
}

/// Writes the grey hint over `field` while it is empty, as a browser's placeholder.
unsafe fn placeholder(field: HWND) {
    unsafe {
        if GetWindowTextLengthW(field) != 0 {
            return;
        }
        let hint = if GetDlgCtrlID(field) == PHRASE {
            PHRASE_HINT
        } else {
            LABEL_HINT
        };
        let mut rect: RECT = std::mem::zeroed();
        SendMessageW(field, EM_GETRECT, 0, &mut rect as *mut RECT as LPARAM);
        let font = SendMessageW(field, WM_GETFONT, 0, 0) as HFONT;
        // The caret blinks by inverting what is under it: keep it off while writing there.
        HideCaret(field);
        let dc = GetDC(field);
        let old = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, PALETTE.get().text_muted.colorref());
        paint::draw_text_as(dc, hint, rect, DT_LEFT);
        SelectObject(dc, old);
        ReleaseDC(field, dc);
        ShowCaret(field);
    }
}

/// The right-click menu of `field`, at the point in `lparam` or, opened from the
/// keyboard (-1, -1), under the field. Hands the focus back to the field.
unsafe fn field_menu(field: HWND, lparam: LPARAM) {
    unsafe {
        let at = point_of(lparam);
        let at = match window_rect(field) {
            Some(rect) if (at.x, at.y) == (-1, -1) => POINT {
                x: rect.left,
                y: rect.bottom,
            },
            _ => at,
        };
        let (mut start, mut end) = (0u32, 0u32);
        SendMessageW(
            field,
            EM_GETSEL,
            &mut start as *mut u32 as WPARAM,
            &mut end as *mut u32 as LPARAM,
        );
        let selected = start != end;
        let items = [
            Item::entry(EDIT_UNDO, "Undo")
                .shortcut("Ctrl+Z")
                .enabled(SendMessageW(field, EM_CANUNDO, 0, 0) != 0),
            Item::Separator,
            Item::entry(EDIT_CUT, "Cut")
                .shortcut("Ctrl+X")
                .enabled(selected),
            Item::entry(EDIT_COPY, "Copy")
                .shortcut("Ctrl+C")
                .enabled(selected),
            Item::entry(EDIT_PASTE, "Paste")
                .shortcut("Ctrl+V")
                .enabled(clipboard_has_text()),
            Item::Separator,
            Item::entry(EDIT_SELECT_ALL, "Select All").shortcut("Ctrl+A"),
        ];
        let picked = menu::pick(GetParent(field), at, &items);
        // Unless the user went on to another window meanwhile.
        let front = GetForegroundWindow();
        if front.is_null() || is_ours(front) {
            SetFocus(field);
        }
        let (msg, wparam, lparam) = match picked {
            Some(EDIT_UNDO) => (EM_UNDO, 0, 0),
            Some(EDIT_CUT) => (WM_CUT, 0, 0),
            Some(EDIT_COPY) => (WM_COPY, 0, 0),
            Some(EDIT_PASTE) => (WM_PASTE, 0, 0),
            Some(EDIT_SELECT_ALL) => (EM_SETSEL, 0, -1),
            _ => return,
        };
        SendMessageW(field, msg, wparam, lparam);
    }
}

unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                // Unpainted, it would be asked again and again; the default at least validates it.
                if with_dialog(|d| d.paint()).is_some() {
                    return 0;
                }
            }
            // Painting covers it all.
            WM_ERASEBKGND => return 1,
            WM_CTLCOLOREDIT => {
                let p = PALETTE.get();
                let dc = wparam as HDC;
                SetTextColor(dc, p.text.colorref());
                SetBkColor(dc, field_color(p).colorref());
                if let Some(brush) = with_dialog(|d| d.field_brush.handle()) {
                    return brush as LRESULT;
                }
            }
            WM_COMMAND => {
                let (id, code) = ((wparam & 0xffff) as i32, ((wparam >> 16) & 0xffff) as u32);
                let part = match id {
                    PHRASE => Part::Phrase,
                    LABEL => Part::Label,
                    _ => return 0,
                };
                match code {
                    EN_CHANGE => {
                        if part == Part::Phrase {
                            update_ok();
                        }
                        // The placeholder comes and goes.
                        InvalidateRect(lparam as HWND, std::ptr::null(), 1);
                    }
                    EN_SETFOCUS => {
                        with_dialog(|d| {
                            d.focus = part;
                            d.redraw();
                        });
                    }
                    _ => {}
                }
                return 0;
            }
            // Back to where the focus was, rather than on the box itself.
            WM_ACTIVATE if (wparam & 0xffff) as u32 != WA_INACTIVE => {
                if let Some(field) = with_dialog(|d| {
                    d.active = true;
                    d.redraw();
                    d.field_of(d.focus)
                }) {
                    SetFocus(field.unwrap_or(hwnd));
                }
                return 0;
            }
            // Off to another app, the focus ring goes, as in a browser; not for our own
            // right-click menu, which leaves the focus where it was.
            WM_ACTIVATE => {
                let ours = is_ours(lparam as HWND);
                with_dialog(|d| {
                    d.active = ours;
                    d.redraw();
                });
            }
            // Anywhere but the fields, the switch's row and the buttons moves the box.
            WM_NCHITTEST => {
                let hit = DefWindowProcW(hwnd, msg, wparam, lparam);
                if hit == HTCLIENT as LRESULT {
                    let mut at = point_of(lparam);
                    ScreenToClient(hwnd, &mut at);
                    if with_dialog(|d| d.layout.part_at(at)).flatten().is_none() {
                        return HTCAPTION as LRESULT;
                    }
                }
                return hit;
            }
            // A text cursor over the fields' frames as well as the fields.
            WM_SETCURSOR if wparam as HWND == hwnd && (lparam & 0xffff) as u32 == HTCLIENT => {
                let mut at = POINT { x: 0, y: 0 };
                GetCursorPos(&mut at);
                ScreenToClient(hwnd, &mut at);
                if let Some(Some(Part::Phrase | Part::Label)) =
                    with_dialog(|d| d.layout.part_at(at))
                {
                    SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_IBEAM));
                    return 1;
                }
            }
            WM_MOUSEMOVE => {
                track_leave(hwnd);
                refresh_hot();
                return 0;
            }
            WM_MOUSELEAVE => {
                refresh_hot();
                return 0;
            }
            WM_NCMOUSEMOVE => refresh_hot(),
            WM_LBUTTONDOWN => {
                let at = point_of(lparam);
                let Some((part, ok, field)) = with_dialog(|d| {
                    let part = d.layout.part_at(at);
                    (part, d.can_ok, part.and_then(|p| d.field_of(p)))
                }) else {
                    return 0;
                };
                match (part, field) {
                    (_, Some(field)) => {
                        SetFocus(field);
                    }
                    (Some(Part::Ok), _) if !ok => {}
                    (Some(part), None) => {
                        with_dialog(|d| d.pressed = Some(part));
                        move_focus(part);
                        SetCapture(hwnd);
                    }
                    (None, None) => {}
                }
                return 0;
            }
            WM_LBUTTONUP => {
                // Take the press before letting go of the mouse, or WM_CAPTURECHANGED drops it.
                let pressed = with_dialog(|d| d.pressed.take()).flatten();
                ReleaseCapture();
                if let Some(part) = pressed {
                    let at = point_of(lparam);
                    if with_dialog(|d| d.layout.part_at(at)).flatten() == Some(part) {
                        act(part);
                    }
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                }
                return 0;
            }
            WM_CAPTURECHANGED => {
                with_dialog(|d| {
                    if d.pressed.take().is_some() {
                        d.redraw();
                    }
                });
                return 0;
            }
            // Only while the focus is on the switch or a button: the fields take their own keys.
            // Once per press: held down, the key repeats.
            WM_KEYDOWN if wparam as u16 == VK_SPACE => {
                if lparam & (1 << 30) != 0 {
                    return 0;
                }
                if let Some(focus) = with_dialog(|d| d.focus) {
                    act(focus);
                }
                return 0;
            }
            WM_DPICHANGED => {
                rescale(hwnd, &*(lparam as *const RECT));
                return 0;
            }
            WM_CLOSE => {
                close(Status::Cancelled);
                return 0;
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_phrase_is_no_phrase() {
        assert_eq!(phrase_from("", "Label", false), None);
        assert_eq!(phrase_from("   ", "", true), None);
    }

    #[test]
    fn fields_are_trimmed_and_blank_label_is_none() {
        assert_eq!(
            phrase_from("  /cost ", "  ", false),
            Some(Phrase {
                text: "/cost".into(),
                label: None,
                mode: Mode::Send,
            })
        );
        assert_eq!(
            phrase_from("/review", " Review ", true),
            Some(Phrase {
                text: "/review".into(),
                label: Some("Review".into()),
                mode: Mode::Fill,
            })
        );
    }

    #[test]
    fn box_is_as_tall_as_the_design() {
        let layout = Layout::new(|px| px, 40, 20);
        assert_eq!(layout.size.cx, 360);
        assert_eq!(layout.size.cy, 238);
    }

    #[test]
    fn buttons_sit_right_with_cancel_first() {
        let layout = Layout::new(|px| px, 40, 20);
        assert_eq!((layout.ok.left, layout.ok.right), (304, 340));
        assert_eq!((layout.cancel.left, layout.cancel.right), (240, 296));
        assert_eq!(layout.switch.right, 340);
    }

    #[test]
    fn only_the_controls_are_clickable() {
        let layout = Layout::new(|px| px, 40, 20);
        let at = |x, y| layout.part_at(POINT { x, y });
        assert_eq!(at(30, 30), None);
        assert_eq!(at(30, layout.fields[0].top), Some(Part::Phrase));
        assert_eq!(at(30, layout.switch_row.top), Some(Part::Switch));
        assert_eq!(at(layout.ok.left, layout.ok.top), Some(Part::Ok));
        assert_eq!(at(layout.cancel.right, layout.ok.top), None);
    }

    #[test]
    fn box_opens_below_the_cursor_or_above_it_near_the_bottom() {
        let work = RECT {
            left: 0,
            top: 0,
            right: 1000,
            bottom: 800,
        };
        let size = SIZE { cx: 360, cy: 238 };
        let at = |x, y| {
            let p = place(size, POINT { x, y }, 16, work);
            (p.x, p.y)
        };
        assert_eq!(at(500, 100), (320, 116));
        assert_eq!(at(500, 700), (320, 446));
        // Kept on the screen at its edges.
        assert_eq!(at(10, 100), (0, 116));
    }

    #[test]
    fn tab_goes_round_and_passes_over_ok_while_greyed() {
        assert_eq!(tab(Part::Phrase, false, true), Part::Label);
        assert_eq!(tab(Part::Cancel, false, true), Part::Ok);
        assert_eq!(tab(Part::Ok, false, true), Part::Phrase);
        assert_eq!(tab(Part::Cancel, false, false), Part::Phrase);
        assert_eq!(tab(Part::Phrase, true, false), Part::Cancel);
        assert_eq!(tab(Part::Phrase, true, true), Part::Ok);
    }
}
