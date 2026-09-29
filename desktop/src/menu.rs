//! The right-click menu of the bar and of the phrase box's fields, drawn after
//! Claude's menus. Works like `TrackPopupMenu` with `TPM_RETURNCMD`: `pick` opens it
//! and returns once it is closed, with what was picked.

use std::cell::{Cell, RefCell};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_RETURN, VK_SPACE, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
    IDC_ARROW, LoadCursorW, MSG, PostQuitMessage, RegisterClassW, SWP_NOZORDER, SWP_SHOWWINDOW,
    SetForegroundWindow, SetWindowPos, TranslateMessage, WA_INACTIVE, WM_ACTIVATE, WM_CANCELMODE,
    WM_CAPTURECHANGED, WM_CLOSE, WM_DPICHANGED, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::paint::{self, Align, Canvas, FONT, Font, Palette, scale};
use crate::win::{point_of, wide, work_area};

// Layout in 96-dpi pixels, after Claude's compact menus.
/// Around the items, the 1px window edge included.
const PAD: i32 = 5;
const ROW: i32 = 24;
const ROW_PAD_X: i32 = 8;
const ROW_RADIUS: i32 = 6;
/// Between an item's text and its shortcut, at the least.
const SHORTCUT_GAP: i32 = 24;
/// A separator: its line with this much above and below it, indented this much.
const SEPARATOR_GAP: i32 = 4;
const SEPARATOR_INDENT: i32 = 8;
const MIN_WIDTH: i32 = 128;
const FONT_SIZE: i32 = 13;

/// One line of a menu.
#[derive(Clone, Copy)]
pub enum Item {
    Entry(Entry),
    /// A line between groups of entries.
    Separator,
}

#[derive(Clone, Copy)]
pub struct Entry {
    /// What `pick` returns when this entry is picked.
    id: usize,
    label: &'static str,
    /// Shown on the right, as `Ctrl+C`.
    shortcut: Option<&'static str>,
    /// Greyed out and passed over when not.
    enabled: bool,
    /// Deletes something: shown in red.
    danger: bool,
}

impl Item {
    pub fn entry(id: usize, label: &'static str) -> Item {
        Item::Entry(Entry {
            id,
            label,
            shortcut: None,
            enabled: true,
            danger: false,
        })
    }

    pub fn shortcut(self, shortcut: &'static str) -> Item {
        self.with(|e| e.shortcut = Some(shortcut))
    }

    pub fn enabled(self, enabled: bool) -> Item {
        self.with(|e| e.enabled = enabled)
    }

    pub fn danger(self) -> Item {
        self.with(|e| e.danger = true)
    }

    fn with(self, f: impl FnOnce(&mut Entry)) -> Item {
        match self {
            Item::Entry(mut entry) => {
                f(&mut entry);
                Item::Entry(entry)
            }
            Item::Separator => Item::Separator,
        }
    }

    /// The entry, when it can be picked now.
    fn usable(&self) -> Option<&Entry> {
        match self {
            Item::Entry(entry) if entry.enabled => Some(entry),
            _ => None,
        }
    }
}

/// An open menu.
struct Menu {
    hwnd: HWND,
    items: Vec<Item>,
    /// Each item's rectangle, in client pixels.
    rows: Vec<RECT>,
    /// The whole menu, in client pixels.
    size: SIZE,
    font: Font,
    /// The item lit up, by the mouse or the keys.
    hot: Option<usize>,
    /// A mouse button went down on the menu and is still down.
    pressed: bool,
    /// Where the mouse was last seen, in client pixels, to tell a move from one
    /// Windows makes up when the menu shows or a key is pressed.
    mouse: POINT,
    status: Status,
}

enum Status {
    Open,
    /// Closed by a press outside, whose release is still to come and to be swallowed.
    Dismissed,
    Closed(Option<usize>),
}

thread_local! {
    static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) };
    static PALETTE: Cell<&'static Palette> = const { Cell::new(&Palette::LIGHT) };
}

/// Colors menus from now on, and the one open, if any.
pub fn set_theme(palette: &'static Palette) {
    PALETTE.set(palette);
    with_menu(|m| {
        paint::shape(m.hwnd, palette.menu_border);
        unsafe { InvalidateRect(m.hwnd, std::ptr::null(), 0) };
    });
}

/// Opens a menu of `items` at `at` in screen pixels, flipped to the other side where
/// the screen ends, and waits for the user. The id of the entry picked, or `None`
/// when the menu was closed otherwise: Esc, a click outside (which goes no further),
/// another window coming to the front.
///
/// The menu takes the foreground to get the keys; handing it back is up to the caller.
pub fn pick(owner: HWND, at: POINT, items: &[Item]) -> Option<usize> {
    // Opened from inside an open menu's loop: a second right-click queued up behind the first.
    if MENU.with_borrow(Option::is_some) {
        return None;
    }
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide("quickbar-menu");
        // Fails harmlessly when already registered by an earlier menu.
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(menu_proc),
            hInstance: instance,
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        });
        // Made at `at` first, so it is scaled for the screen it opens on.
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            at.x,
            at.y,
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
        let s = |px| scale(hwnd, px);
        let font = Font::new(FONT, s(FONT_SIZE));
        let widths: Vec<_> = items
            .iter()
            .map(|item| match item {
                Item::Entry(e) => (font.width(e.label), e.shortcut.map(|k| font.width(k))),
                Item::Separator => (0, None),
            })
            .collect();
        let (rows, size) = layout(items, &widths, s);
        let to = place(size, at, work_area(at));
        paint::shape(hwnd, PALETTE.get().menu_border);
        let mut mouse = POINT { x: 0, y: 0 };
        GetCursorPos(&mut mouse);
        mouse.x -= to.x;
        mouse.y -= to.y;
        MENU.set(Some(Menu {
            hwnd,
            items: items.to_vec(),
            rows,
            size,
            font,
            hot: None,
            pressed: false,
            mouse,
            status: Status::Open,
        }));
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            to.x,
            to.y,
            size.cx,
            size.cy,
            SWP_NOZORDER | SWP_SHOWWINDOW,
        );
        // Keys go to the window in front; clicks anywhere come here while it has the mouse.
        SetForegroundWindow(hwnd);
        SetCapture(hwnd);

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
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let status = MENU.take().map(|m| m.status);
        ReleaseCapture();
        DestroyWindow(hwnd);
        match status {
            Some(Status::Closed(picked)) => picked,
            _ => None,
        }
    }
}

fn is_open() -> bool {
    MENU.with_borrow(|m| {
        m.as_ref()
            .is_some_and(|m| !matches!(m.status, Status::Closed(_)))
    })
}

/// Runs `f` on the open menu. Messages sent to the menu while it is already
/// borrowed (say, by our own `ReleaseCapture`) are ignored rather than panicking.
fn with_menu<R>(f: impl FnOnce(&mut Menu) -> R) -> Option<R> {
    MENU.with(|menu| menu.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Each item's rectangle in client pixels and the menu's size, given each item's
/// text and shortcut widths and `s` to scale 96-dpi pixels.
fn layout(
    items: &[Item],
    widths: &[(i32, Option<i32>)],
    s: impl Fn(i32) -> i32,
) -> (Vec<RECT>, SIZE) {
    let label = widths.iter().map(|w| w.0).max().unwrap_or(0);
    let shortcut = widths.iter().filter_map(|w| w.1).max();
    let content = label + shortcut.map_or(0, |w| s(SHORTCUT_GAP) + w);
    let width = (2 * (s(PAD) + s(ROW_PAD_X)) + content).max(s(MIN_WIDTH));
    let mut y = s(PAD);
    let rows = items
        .iter()
        .map(|item| {
            let height = match item {
                Item::Entry(_) => s(ROW),
                Item::Separator => 2 * s(SEPARATOR_GAP) + s(1),
            };
            let row = RECT {
                left: s(PAD),
                top: y,
                right: width - s(PAD),
                bottom: y + height,
            };
            y += height;
            row
        })
        .collect();
    (
        rows,
        SIZE {
            cx: width,
            cy: y + s(PAD),
        },
    )
}

/// Where a menu of `size` opened at `at` goes: right of and below it, or left of or
/// above it where the screen's `work` area would cut it off, and inside that area.
fn place(size: SIZE, at: POINT, work: RECT) -> POINT {
    let x = if at.x + size.cx <= work.right {
        at.x
    } else {
        at.x - size.cx
    };
    let y = if at.y + size.cy <= work.bottom {
        at.y
    } else {
        at.y - size.cy
    };
    POINT {
        x: x.min(work.right - size.cx).max(work.left),
        y: y.min(work.bottom - size.cy).max(work.top),
    }
}

#[derive(Clone, Copy)]
enum Step {
    Next,
    Previous,
    First,
    Last,
}

/// The item to light up after `step` from `hot`, among the `usable` ones, going round
/// from one end to the other. `None` when no item is usable.
fn step(usable: &[bool], hot: Option<usize>, step: Step) -> Option<usize> {
    let n = usable.len();
    if n == 0 {
        return None;
    }
    let (start, forward) = match step {
        Step::First => (0, true),
        Step::Last => (n - 1, false),
        Step::Next => (hot.map_or(0, |i| (i + 1) % n), true),
        Step::Previous => (hot.map_or(n - 1, |i| (i + n - 1) % n), false),
    };
    (0..n)
        .map(|k| {
            if forward {
                (start + k) % n
            } else {
                (start + n - k) % n
            }
        })
        .find(|&i| usable[i])
}

impl Menu {
    /// The usable item at `at`, in client pixels.
    fn item_at(&self, POINT { x, y }: POINT) -> Option<usize> {
        self.rows
            .iter()
            .position(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
            .filter(|&i| self.items[i].usable().is_some())
    }

    fn inside(&self, POINT { x, y }: POINT) -> bool {
        x >= 0 && x < self.size.cx && y >= 0 && y < self.size.cy
    }

    fn light(&mut self, hot: Option<usize>) {
        if hot != self.hot {
            self.hot = hot;
            unsafe { InvalidateRect(self.hwnd, std::ptr::null(), 0) };
        }
    }

    fn close(&mut self, picked: Option<usize>) {
        self.status = Status::Closed(picked);
    }

    /// Picks item `i`, when it is usable.
    fn choose(&mut self, i: usize) {
        if let Some(entry) = self.items[i].usable() {
            let id = entry.id;
            self.close(Some(id));
        }
    }

    fn moved(&mut self, at: POINT) {
        if (at.x, at.y) == (self.mouse.x, self.mouse.y) {
            return;
        }
        self.mouse = at;
        self.light(self.item_at(at));
    }

    fn press(&mut self, at: POINT) {
        if self.inside(at) {
            self.pressed = true;
            self.hot = self.item_at(at);
            unsafe { InvalidateRect(self.hwnd, std::ptr::null(), 0) };
        } else {
            self.status = Status::Dismissed;
        }
    }

    /// Either button picks, as with `TPM_RIGHTBUTTON`.
    fn release(&mut self, at: POINT) {
        if matches!(self.status, Status::Dismissed) {
            self.close(None);
            return;
        }
        self.pressed = false;
        match self.item_at(at) {
            Some(i) => self.choose(i),
            None => unsafe {
                InvalidateRect(self.hwnd, std::ptr::null(), 0);
            },
        }
    }

    fn key(&mut self, key: u16) {
        let how = match key {
            VK_ESCAPE => return self.close(None),
            VK_RETURN | VK_SPACE => {
                if let Some(i) = self.hot {
                    self.choose(i);
                }
                return;
            }
            VK_DOWN => Step::Next,
            VK_UP => Step::Previous,
            VK_HOME => Step::First,
            VK_END => Step::Last,
            _ => return,
        };
        let usable: Vec<bool> = self.items.iter().map(|i| i.usable().is_some()).collect();
        self.light(step(&usable, self.hot, how));
    }

    fn paint(&self) {
        let canvas = Canvas::begin(self.hwnd);
        let p = PALETTE.get();
        canvas.fill(&canvas.bounds(), p.surface);
        let s = |px| scale(self.hwnd, px);
        for (i, (item, row)) in self.items.iter().zip(&self.rows).enumerate() {
            let entry = match item {
                Item::Entry(entry) => entry,
                Item::Separator => {
                    let top = row.top + s(SEPARATOR_GAP);
                    let line = RECT {
                        left: row.left + s(SEPARATOR_INDENT),
                        top,
                        right: row.right - s(SEPARATOR_INDENT),
                        bottom: top + s(1),
                    };
                    canvas.fill(&line, p.separator);
                    continue;
                }
            };
            let lit = self.hot == Some(i);
            let down = lit && self.pressed;
            let (back, text, shortcut) = match (entry.enabled, entry.danger, lit) {
                (false, _, _) => (None, p.text_off, p.shortcut_off),
                (true, true, true) => (
                    Some(if down { p.danger_down } else { p.danger }),
                    p.on_danger,
                    p.on_danger,
                ),
                (true, true, false) => (None, p.text_danger, p.text_muted),
                (true, false, true) => (
                    Some(if down { p.menu_down } else { p.menu_hover }),
                    p.text,
                    p.text_muted,
                ),
                (true, false, false) => (None, p.text, p.text_muted),
            };
            if let Some(back) = back {
                canvas.rounded(row, s(ROW_RADIUS), Some(back), None);
            }
            let inner = RECT {
                left: row.left + s(ROW_PAD_X),
                right: row.right - s(ROW_PAD_X),
                ..*row
            };
            canvas.text_aligned(&self.font, text, entry.label, &inner, Align::Left);
            if let Some(key) = entry.shortcut {
                canvas.text_aligned(&self.font, shortcut, key, &inner, Align::Right);
            }
        }
    }
}

unsafe extern "system" fn menu_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                // Unpainted, it would be asked again and again; the default at least validates it.
                if with_menu(|m| m.paint()).is_some() {
                    return 0;
                }
            }
            WM_MOUSEMOVE => {
                with_menu(|m| m.moved(point_of(lparam)));
                return 0;
            }
            WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
                with_menu(|m| m.press(point_of(lparam)));
                return 0;
            }
            WM_LBUTTONUP | WM_RBUTTONUP => {
                with_menu(|m| m.release(point_of(lparam)));
                return 0;
            }
            WM_KEYDOWN => {
                with_menu(|m| m.key(wparam as u16));
                return 0;
            }
            // Alt, as with the system's menus; Alt+Tab and Alt+F4 go on as usual.
            WM_SYSKEYDOWN => {
                with_menu(|m| m.close(None));
            }
            WM_ACTIVATE if (wparam & 0xffff) as u32 == WA_INACTIVE => {
                with_menu(|m| m.close(None));
            }
            WM_CAPTURECHANGED if lparam as HWND != hwnd => {
                with_menu(|m| m.close(None));
                return 0;
            }
            WM_CANCELMODE | WM_CLOSE => {
                with_menu(|m| m.close(None));
                return 0;
            }
            // Open only briefly: keep the size it opened with.
            WM_DPICHANGED => return 0,
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: RECT = RECT {
        left: 0,
        top: 0,
        right: 1000,
        bottom: 800,
    };
    const MENU_SIZE: SIZE = SIZE { cx: 128, cy: 91 };

    fn at(x: i32, y: i32) -> (i32, i32) {
        let p = place(MENU_SIZE, POINT { x, y }, WORK);
        (p.x, p.y)
    }

    #[test]
    fn menu_opens_right_and_below_where_there_is_room() {
        assert_eq!(at(100, 100), (100, 100));
    }

    #[test]
    fn menu_flips_where_the_screen_ends() {
        assert_eq!(at(950, 100), (822, 100));
        assert_eq!(at(100, 750), (100, 659));
        assert_eq!(at(950, 750), (822, 659));
    }

    #[test]
    fn menu_stays_on_the_screen() {
        // No room on either side: pushed back in.
        let narrow = RECT { right: 150, ..WORK };
        let p = place(MENU_SIZE, POINT { x: 100, y: 0 }, narrow);
        assert_eq!(p.x, 0);
    }

    #[test]
    fn keys_pass_over_items_that_cannot_be_used() {
        // Undo (off) / line / Cut / Copy (off) / Paste / line / Select All
        let usable = [false, false, true, false, true, false, true];
        assert_eq!(step(&usable, None, Step::Next), Some(2));
        assert_eq!(step(&usable, Some(2), Step::Next), Some(4));
        assert_eq!(step(&usable, None, Step::Previous), Some(6));
        assert_eq!(step(&usable, Some(4), Step::Previous), Some(2));
        assert_eq!(step(&usable, None, Step::First), Some(2));
        assert_eq!(step(&usable, Some(2), Step::Last), Some(6));
    }

    #[test]
    fn keys_go_round_from_one_end_to_the_other() {
        let usable = [true, false, true];
        assert_eq!(step(&usable, Some(2), Step::Next), Some(0));
        assert_eq!(step(&usable, Some(0), Step::Previous), Some(2));
    }

    #[test]
    fn keys_light_nothing_when_nothing_can_be_used() {
        assert_eq!(step(&[false, false], None, Step::Next), None);
        assert_eq!(step(&[], None, Step::First), None);
    }

    #[test]
    fn menu_is_as_tall_as_claude_s() {
        // The bar's menu on a phrase: Edit / Delete / line / Quit.
        let items = [
            Item::entry(1, "Edit"),
            Item::entry(2, "Delete").danger(),
            Item::Separator,
            Item::entry(3, "Quit"),
        ];
        let widths = [(30, None), (40, None), (0, None), (30, None)];
        let (rows, size) = layout(&items, &widths, |px| px);
        assert_eq!(size.cy, 91);
        assert_eq!(size.cx, 128);
        assert_eq!(rows[2].bottom - rows[2].top, 9);
    }

    #[test]
    fn shortcuts_widen_the_menu() {
        let items = [Item::entry(1, "Select All").shortcut("Ctrl+A")];
        let (_, size) = layout(&items, &[(80, Some(50))], |px| px);
        assert_eq!(size.cx, 2 * (5 + 8) + 80 + 24 + 50);
    }
}
