//! Drawing shared by the bar, the command box and the menu: window shape and
//! shadow from DWM, anti-aliased rounded blocks from GDI+, ClearType text from GDI.

use windows_sys::Win32::Foundation::{HWND, RECT, SIZE};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush,
    DRAW_TEXT_FORMAT, DT_CENTER, DT_LEFT, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DeleteDC,
    DeleteObject, DrawTextW, EndPaint, FillRect, GetDC, GetTextExtentPoint32W, GetTextMetricsW,
    HBITMAP, HBRUSH, HDC, HFONT, HGDIOBJ, PAINTSTRUCT, ReleaseDC, SRCCOPY, SelectObject, SetBkMode,
    SetTextColor, TEXTMETRICW, TRANSPARENT,
};
use windows_sys::Win32::Graphics::GdiPlus::{
    FillModeAlternate, FlushIntentionFlush, GdipAddPathArc, GdipAddPathRectangle,
    GdipClosePathFigure, GdipCreateFromHDC, GdipCreatePath, GdipCreatePen1, GdipCreateSolidFill,
    GdipDeleteBrush, GdipDeleteGraphics, GdipDeletePath, GdipDeletePen, GdipDrawPath, GdipFillPath,
    GdipFlush, GdipSetPixelOffsetMode, GdipSetSmoothingMode, GdiplusShutdown, GdiplusStartup,
    GdiplusStartupInput, GpGraphics, GpPath, Ok, PixelOffsetModeHalf, SmoothingModeAntiAlias,
    UnitPixel,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

use crate::win::wide;

/// A color with alpha, as GDI+ takes it: `0xAARRGGBB`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Color(u32);

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color::rgba(r, g, b, 255)
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color((a as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | b as u32)
    }

    fn channels(self) -> [u8; 4] {
        let [b, g, r, a] = self.0.to_le_bytes();
        [r, g, b, a]
    }

    /// This color at `alpha` (out of 255) of its own opacity, as when a whole part is faded.
    pub fn faded(self, alpha: u8) -> Color {
        let [r, g, b, a] = self.channels();
        Color::rgba(r, g, b, ((a as u32 * alpha as u32 + 127) / 255) as u8)
    }

    /// The opaque color this one makes when laid on `below`, which should be opaque.
    pub fn over(self, below: Color) -> Color {
        let ([r, g, b, a], [br, bg, bb, _]) = (self.channels(), below.channels());
        let mix = |top: u8, under: u8| {
            let (top, under, a) = (top as u32, under as u32, a as u32);
            ((top * a + under * (255 - a) + 127) / 255) as u8
        };
        Color::rgb(mix(r, br), mix(g, bg), mix(b, bb))
    }

    /// For GDI, which knows no alpha: `0x00BBGGRR`.
    pub fn colorref(self) -> u32 {
        let [r, g, b, _] = self.channels();
        r as u32 | (g as u32) << 8 | (b as u32) << 16
    }
}

/// Every color of one theme, named after the part of Claude's Code page it comes from
/// and, where Claude has one, the `--cds-*` CSS variable behind it. Text colors are
/// opaque; fills may be see-through.
// Kept whole after Claude, though nothing here is ghost, orange or a pressed toggle yet.
#[allow(dead_code)]
pub struct Palette {
    /// Behind the bar, the command box and the menu: Claude's popovers
    /// (`--cds-surface-popover`), which the prompt card shares.
    pub surface: Color,
    /// The 1px edge DWM draws around the bar and the command box, opaque, on the
    /// outermost ring of client pixels (the prompt card's edge as seen on screen).
    pub window_border: Color,
    /// Thin ring around cards and fields (`--cds-border`).
    pub border: Color,
    /// The ring under the mouse (`--cds-border-strong`).
    pub border_hover: Color,
    /// Body text (`--cds-text-primary`).
    pub text: Color,
    /// Secondary text (`--cds-text-secondary`).
    pub text_secondary: Color,
    /// Placeholders, shortcuts (`--cds-text-muted`).
    pub text_muted: Color,
    /// Warning text (`--cds-text-warning`).
    pub text_warning: Color,
    /// Error text (`--cds-text-danger`).
    pub text_danger: Color,
    /// Behind a borderless button under the mouse (`--cds-fill-ghost-hover`).
    pub ghost_hover: Color,
    /// Switches, checks and the focus ring (`--cds-fill-accent`), and under the
    /// mouse (`--cds-fill-accent-hover`).
    pub accent: Color,
    pub accent_hover: Color,
    /// The soft glow around the focus ring: the last shadow of `--cds-focus-shadow`.
    pub focus_glow: Color,
    /// A toggle that is on: behind it (`--cds-bg-accent`), and its text (`--cds-text-accent`).
    pub accent_pressed: Color,
    pub text_accent: Color,
    /// Send buttons, Claude's orange (`--cds-fill-brand`), under the mouse
    /// (`--cds-fill-brand-hover`), and their text (`--cds-on-brand`).
    pub brand: Color,
    pub brand_hover: Color,
    pub on_brand: Color,
    /// Inside a text field (`--cds-fill-field`).
    pub field: Color,
    /// The main button of a box (OK), Claude's primary button: as is (`--cds-fill-primary`),
    /// under the mouse (`--cds-fill-primary-hover`), held down, its text (`--cds-on-primary`),
    /// and unavailable (Claude's 40%, laid on the box).
    pub primary: Color,
    pub primary_hover: Color,
    pub primary_down: Color,
    pub on_primary: Color,
    pub primary_off: Color,
    pub on_primary_off: Color,
    /// A switch that is off (`--cds-switch-track`), and under the mouse
    /// (`--cds-switch-track-hover`), laid on the surface.
    pub track: Color,
    pub track_hover: Color,
    /// The line just inside the edge of whatever has the focus ring: the inset of
    /// `--cds-focus-shadow`, which is `--cds-page-bg`.
    pub focus_inset: Color,
    /// Buttons on the bar and the other buttons of a box (Cancel): Claude's
    /// secondary button (`--cds-fill-secondary`, its `-ring`, `-hover`, held down),
    /// laid on the surface. Opaque, but for the ring and the shadow under it.
    pub chip: Color,
    pub chip_ring: Option<Color>,
    pub chip_shadow: Option<Color>,
    pub chip_hover: Color,
    pub chip_down: Color,
    /// Behind a menu item that deletes, under the mouse (`--cds-fill-danger`) and held
    /// down, and its text there (`--cds-on-danger`).
    pub danger: Color,
    pub danger_down: Color,
    pub on_danger: Color,
    /// The 1px edge DWM draws around a menu (Claude's popovers are darker than its cards).
    pub menu_border: Color,
    /// Behind a menu item under the mouse or picked with the keys
    /// (`--cds-fill-ghost-hover`), and held down (`--cds-fill-ghost-selected`),
    /// laid on the menu.
    pub menu_hover: Color,
    pub menu_down: Color,
    /// The line between groups of menu items (`--cds-border`, laid on the menu).
    pub separator: Color,
    /// A menu item that cannot be used now (`--cds-text-primary`), and its shortcut
    /// (`--cds-text-muted`), at Claude's 40% laid on the menu.
    pub text_off: Color,
    pub shortcut_off: Color,
}

impl Palette {
    pub const LIGHT: Palette = Palette {
        surface: Color::rgb(0xff, 0xff, 0xff),
        // Measured on the prompt card; its ring over white alone would make #e6e6e6.
        window_border: Color::rgb(0xe0, 0xe0, 0xdf),
        border: Color::rgba(0x0b, 0x0b, 0x0b, 0x1a),
        border_hover: Color::rgba(0x0b, 0x0b, 0x0b, 0x33),
        text: Color::rgb(0x0b, 0x0b, 0x0b),
        text_secondary: Color::rgb(0x52, 0x51, 0x4e),
        text_muted: Color::rgb(0x89, 0x87, 0x81),
        text_warning: Color::rgb(0x73, 0x45, 0x00),
        text_danger: Color::rgb(0x8e, 0x26, 0x26),
        ghost_hover: Color::rgba(0x0b, 0x0b, 0x0b, 0x0d),
        accent: Color::rgb(0x2a, 0x78, 0xd6),
        accent_hover: Color::rgb(0x39, 0x87, 0xe5),
        focus_glow: Color::rgb(0xcd, 0xe2, 0xfb),
        accent_pressed: Color::rgb(0xcd, 0xe2, 0xfb),
        text_accent: Color::rgb(0x18, 0x4f, 0x95),
        brand: Color::rgb(0xc6, 0x61, 0x3f),
        brand_hover: Color::rgb(0xd9, 0x77, 0x57),
        on_brand: Color::rgb(0xff, 0xff, 0xff),
        field: Color::rgba(0xff, 0xff, 0xff, 0x80),
        primary: Color::rgb(0x0b, 0x0b, 0x0b),
        primary_hover: Color::rgb(0x2c, 0x2c, 0x2a),
        primary_down: Color::rgb(0x3c, 0x3c, 0x3b),
        on_primary: Color::rgb(0xff, 0xff, 0xff),
        primary_off: Color::rgb(0x9d, 0x9d, 0x9d),
        on_primary_off: Color::rgb(0xff, 0xff, 0xff),
        track: Color::rgb(0xce, 0xce, 0xce),
        track_hover: Color::rgb(0xaa, 0xaa, 0xaa),
        focus_inset: Color::rgb(0xf9, 0xf9, 0xf7),
        chip: Color::rgb(0xff, 0xff, 0xff),
        chip_ring: Some(Color::rgba(0x0b, 0x0b, 0x0b, 0x1a)),
        chip_shadow: Some(Color::rgba(0x00, 0x00, 0x00, 0x0d)),
        chip_hover: Color::rgb(0xf3, 0xf3, 0xf3),
        chip_down: Color::rgb(0xe7, 0xe7, 0xe7),
        danger: Color::rgb(0xd0, 0x3b, 0x3b),
        danger_down: Color::rgb(0xbb, 0x35, 0x35),
        on_danger: Color::rgb(0xff, 0xff, 0xff),
        // Measured on a popover.
        menu_border: Color::rgb(0xcf, 0xcf, 0xce),
        menu_hover: Color::rgb(0xf3, 0xf3, 0xf3),
        menu_down: Color::rgb(0xe7, 0xe7, 0xe7),
        separator: Color::rgb(0xe6, 0xe6, 0xe6),
        // Claude's 40% of the full colors, laid on the menu.
        text_off: Color::rgb(0x9d, 0x9d, 0x9d),
        shortcut_off: Color::rgb(0xd0, 0xcf, 0xcd),
    };

    pub const DARK: Palette = Palette {
        surface: Color::rgb(0x20, 0x20, 0x1f),
        window_border: Color::rgb(0x37, 0x37, 0x36),
        border: Color::rgba(0xff, 0xff, 0xff, 0x1a),
        border_hover: Color::rgba(0xff, 0xff, 0xff, 0x33),
        text: Color::rgb(0xf0, 0xef, 0xec),
        text_secondary: Color::rgb(0xc3, 0xc2, 0xb7),
        text_muted: Color::rgb(0x89, 0x87, 0x81),
        text_warning: Color::rgb(0xdb, 0x93, 0x00),
        text_danger: Color::rgb(0xec, 0x7e, 0x7e),
        ghost_hover: Color::rgba(0xff, 0xff, 0xff, 0x13),
        accent: Color::rgb(0x2a, 0x78, 0xd6),
        accent_hover: Color::rgb(0x39, 0x87, 0xe5),
        focus_glow: Color::rgba(0x18, 0x4f, 0x95, 0x99),
        accent_pressed: Color::rgb(0x03, 0x20, 0x42),
        text_accent: Color::rgb(0x6d, 0xa7, 0xec),
        brand: Color::rgb(0xc6, 0x61, 0x3f),
        brand_hover: Color::rgb(0xd9, 0x77, 0x57),
        on_brand: Color::rgb(0xff, 0xff, 0xff),
        field: Color::rgba(0xff, 0xff, 0xff, 0x0d),
        primary: Color::rgb(0xff, 0xff, 0xff),
        primary_hover: Color::rgb(0xe1, 0xe0, 0xd9),
        primary_down: Color::rgb(0xcb, 0xca, 0xc3),
        on_primary: Color::rgb(0x0b, 0x0b, 0x0b),
        primary_off: Color::rgb(0x79, 0x79, 0x79),
        on_primary_off: Color::rgb(0x18, 0x18, 0x17),
        track: Color::rgb(0x36, 0x36, 0x35),
        track_hover: Color::rgb(0x4d, 0x4d, 0x4c),
        focus_inset: Color::rgb(0x0b, 0x0b, 0x0b),
        chip: Color::rgb(0x36, 0x36, 0x35),
        chip_ring: None,
        chip_shadow: None,
        chip_hover: Color::rgb(0x3f, 0x3f, 0x3f),
        chip_down: Color::rgb(0x4d, 0x4d, 0x4c),
        danger: Color::rgb(0xd0, 0x3b, 0x3b),
        danger_down: Color::rgb(0xbb, 0x35, 0x35),
        on_danger: Color::rgb(0xff, 0xff, 0xff),
        menu_border: Color::rgb(0x37, 0x37, 0x36),
        menu_hover: Color::rgb(0x31, 0x31, 0x30),
        menu_down: Color::rgb(0x40, 0x40, 0x3f),
        separator: Color::rgb(0x37, 0x37, 0x36),
        text_off: Color::rgb(0x73, 0x73, 0x71),
        shortcut_off: Color::rgb(0x4a, 0x49, 0x46),
    };
}

/// Keeps GDI+ running; shuts it down when dropped. Start it before any `Canvas`
/// and keep it for the life of the process.
pub struct GdiPlus(usize);

impl GdiPlus {
    pub fn start() -> Option<GdiPlus> {
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        let mut token = 0;
        let status = unsafe { GdiplusStartup(&mut token, &input, std::ptr::null_mut()) };
        (status == Ok).then_some(GdiPlus(token))
    }
}

impl Drop for GdiPlus {
    fn drop(&mut self) {
        unsafe { GdiplusShutdown(self.0) };
    }
}

// Both come with Windows 11, the only Windows these windows are made for, so no fallback.
pub const FONT: &str = "Segoe UI Variable Text";
/// GDI reaches the variable font's weights only through its named instances.
pub const FONT_SEMIBOLD: &str = "Segoe UI Variable Text Semibold";
pub const ICON_FONT: &str = "Segoe Fluent Icons";

/// `px` 96-dpi pixels in the pixels of the screen `hwnd` is on.
pub fn scale(hwnd: HWND, px: i32) -> i32 {
    px * unsafe { GetDpiForWindow(hwnd) } as i32 / 96
}

/// Has DWM round `hwnd`, give it a shadow and a 1px `border` drawn over the outermost
/// ring of client pixels, so keep content off that ring. Square and bare before Windows 11.
pub fn shape(hwnd: HWND, border: Color) {
    let corners = DWMWCP_ROUND;
    let border = border.colorref();
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &corners as *const _ as *const _,
            size_of_val(&corners) as u32,
        );
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as u32,
            &border as *const _ as *const _,
            size_of_val(&border) as u32,
        );
    }
}

/// A ClearType font `height` pixels tall; deleted when dropped.
pub struct Font(HFONT);

impl Font {
    /// Regular weight.
    pub fn new(face: &str, height: i32) -> Font {
        Font::weighted(face, height, 400)
    }

    /// `weight` from 100 to 900, as in CSS.
    pub fn weighted(face: &str, height: i32, weight: i32) -> Font {
        let face = wide(face);
        Font(unsafe {
            CreateFontW(
                -height,
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                1,
                0,
                0,
                5,
                0,
                face.as_ptr(),
            )
        })
    }

    pub fn handle(&self) -> HFONT {
        self.0
    }

    /// How tall a line of it is, in pixels.
    pub fn line_height(&self) -> i32 {
        let mut metrics: TEXTMETRICW = unsafe { std::mem::zeroed() };
        unsafe {
            let dc = GetDC(std::ptr::null_mut());
            let old = SelectObject(dc, self.0);
            GetTextMetricsW(dc, &mut metrics);
            SelectObject(dc, old);
            ReleaseDC(std::ptr::null_mut(), dc);
        }
        metrics.tmHeight
    }

    /// How wide `text` comes out in one line, in pixels.
    pub fn width(&self, text: &str) -> i32 {
        let text = wide(text);
        let mut size = SIZE { cx: 0, cy: 0 };
        unsafe {
            let dc = GetDC(std::ptr::null_mut());
            let old = SelectObject(dc, self.0);
            GetTextExtentPoint32W(dc, text.as_ptr(), text.len() as i32 - 1, &mut size);
            SelectObject(dc, old);
            ReleaseDC(std::ptr::null_mut(), dc);
        }
        size.cx
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { DeleteObject(self.0) };
        }
    }
}

/// A solid GDI brush, for controls that paint themselves; deleted when dropped.
pub struct Brush(HBRUSH);

impl Brush {
    /// Of an opaque color.
    pub fn new(color: Color) -> Brush {
        Brush(unsafe { CreateSolidBrush(color.colorref()) })
    }

    pub fn handle(&self) -> HBRUSH {
        self.0
    }
}

impl Drop for Brush {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { DeleteObject(self.0) };
        }
    }
}

/// Where a line of text goes across its rectangle.
pub enum Align {
    Left,
    Center,
    Right,
}

/// One `WM_PAINT` of a window. Draws off screen and puts the whole client area
/// up at once when dropped, so nothing flickers.
pub struct Canvas {
    hwnd: HWND,
    ps: PAINTSTRUCT,
    dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    bounds: RECT,
    /// Null when GDI+ is not running; rounded blocks are then left out.
    graphics: *mut GpGraphics,
}

impl Canvas {
    pub fn begin(hwnd: HWND) -> Canvas {
        unsafe {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let window = BeginPaint(hwnd, &mut ps);
            let mut bounds: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut bounds);
            let dc = CreateCompatibleDC(window);
            let bitmap = CreateCompatibleBitmap(window, bounds.right, bounds.bottom);
            let old_bitmap = SelectObject(dc, bitmap);
            SetBkMode(dc, TRANSPARENT as i32);
            let mut graphics = std::ptr::null_mut();
            if GdipCreateFromHDC(dc, &mut graphics) == Ok {
                GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
                // Pixel centers on .5, so a shape's integer edges fall between pixels.
                GdipSetPixelOffsetMode(graphics, PixelOffsetModeHalf);
            } else {
                graphics = std::ptr::null_mut();
            }
            Canvas {
                hwnd,
                ps,
                dc,
                bitmap,
                old_bitmap,
                bounds,
                graphics,
            }
        }
    }

    /// The whole client area.
    pub fn bounds(&self) -> RECT {
        self.bounds
    }

    /// Fills `rect` with an opaque color.
    pub fn fill(&self, rect: &RECT, color: Color) {
        unsafe { fill(self.dc, rect, color.colorref()) };
    }

    /// An anti-aliased block over `rect` with corners of `radius` pixels, filled and/or
    /// ringed with a 1px line inside its edge. Either color may be see-through.
    pub fn rounded(&self, rect: &RECT, radius: i32, fill: Option<Color>, ring: Option<Color>) {
        if self.graphics.is_null() {
            return;
        }
        unsafe {
            if let Some(color) = fill
                && let Some(path) = Path::rounded(rect, radius as f32, 0.0)
            {
                let mut brush = std::ptr::null_mut();
                if GdipCreateSolidFill(color.0, &mut brush) == Ok {
                    GdipFillPath(self.graphics, brush.cast(), path.0);
                    GdipDeleteBrush(brush.cast());
                }
            }
            // The line runs along pixel centers half a pixel in.
            if let Some(color) = ring
                && let Some(path) = Path::rounded(rect, radius as f32, 0.5)
            {
                let mut pen = std::ptr::null_mut();
                if GdipCreatePen1(color.0, 1.0, UnitPixel, &mut pen) == Ok {
                    GdipDrawPath(self.graphics, pen, path.0);
                    GdipDeletePen(pen);
                }
            }
        }
    }

    /// `text` in one line, centered in `rect`.
    pub fn text(&self, font: &Font, color: Color, text: &str, rect: &RECT) {
        self.text_aligned(font, color, text, rect, Align::Center);
    }

    /// `text` in one line, centered from top to bottom in `rect` and put to `align` across it.
    pub fn text_aligned(&self, font: &Font, color: Color, text: &str, rect: &RECT, align: Align) {
        let across = match align {
            Align::Left => DT_LEFT,
            Align::Center => DT_CENTER,
            Align::Right => DT_RIGHT,
        };
        unsafe {
            // Blocks GDI+ still holds would land on top of the text.
            if !self.graphics.is_null() {
                GdipFlush(self.graphics, FlushIntentionFlush);
            }
            let old = SelectObject(self.dc, font.handle());
            SetTextColor(self.dc, color.colorref());
            draw_text_as(self.dc, text, *rect, across);
            SelectObject(self.dc, old);
        }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            if !self.graphics.is_null() {
                GdipDeleteGraphics(self.graphics);
            }
            BitBlt(
                self.ps.hdc,
                0,
                0,
                self.bounds.right,
                self.bounds.bottom,
                self.dc,
                0,
                0,
                SRCCOPY,
            );
            SelectObject(self.dc, self.old_bitmap);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
            EndPaint(self.hwnd, &self.ps);
        }
    }
}

/// A GDI+ outline, deleted when dropped.
struct Path(*mut GpPath);

impl Path {
    /// A rounded rectangle along `rect` pulled `inset` pixels in on every side.
    unsafe fn rounded(rect: &RECT, radius: f32, inset: f32) -> Option<Path> {
        let (left, top) = (rect.left as f32 + inset, rect.top as f32 + inset);
        let (right, bottom) = (rect.right as f32 - inset, rect.bottom as f32 - inset);
        let (width, height) = (right - left, bottom - top);
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let d = (2.0 * (radius - inset)).clamp(0.0, width.min(height));
        let mut path = std::ptr::null_mut();
        unsafe {
            if GdipCreatePath(FillModeAlternate, &mut path) != Ok {
                return None;
            }
            let path = Path(path);
            if d > 0.0 {
                GdipAddPathArc(path.0, left, top, d, d, 180.0, 90.0);
                GdipAddPathArc(path.0, right - d, top, d, d, 270.0, 90.0);
                GdipAddPathArc(path.0, right - d, bottom - d, d, d, 0.0, 90.0);
                GdipAddPathArc(path.0, left, bottom - d, d, d, 90.0, 90.0);
            } else {
                GdipAddPathRectangle(path.0, left, top, width, height);
            }
            GdipClosePathFigure(path.0);
            Some(path)
        }
    }
}

impl Drop for Path {
    fn drop(&mut self) {
        unsafe { GdipDeletePath(self.0) };
    }
}

/// Fills `rect` with `color`, a GDI `0x00BBGGRR`.
pub unsafe fn fill(dc: HDC, rect: &RECT, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(dc, rect, brush);
        DeleteObject(brush);
    }
}

/// `text` in one line, centered in `rect`, in the font and color selected into `dc`.
pub unsafe fn draw_text(dc: HDC, text: &str, rect: RECT) {
    unsafe { draw_text_as(dc, text, rect, DT_CENTER) };
}

/// Like `draw_text`, put to `across` (`DT_LEFT`, `DT_CENTER` or `DT_RIGHT`) instead.
pub unsafe fn draw_text_as(dc: HDC, text: &str, mut rect: RECT, across: DRAW_TEXT_FORMAT) {
    let text = wide(text);
    unsafe {
        DrawTextW(
            dc,
            text.as_ptr(),
            text.len() as i32 - 1,
            &mut rect,
            across | DT_VCENTER | DT_SINGLELINE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_color_covers_what_is_below() {
        let red = Color::rgb(0xff, 0, 0);
        assert_eq!(red.over(Color::rgb(0, 0, 0xff)), red);
    }

    #[test]
    fn see_through_color_mixes_with_what_is_below() {
        // Claude's dark ring (10% white, 0x1a) on its dark card: the edge it shows.
        let dark = &Palette::DARK;
        assert_eq!(dark.border.over(dark.surface), dark.window_border);
        assert_eq!(dark.window_border, Color::rgb(0x37, 0x37, 0x36));
        // Claude's light ring (10% near-black) on its white card.
        let light = &Palette::LIGHT;
        assert_eq!(
            light.border.over(light.surface),
            Color::rgb(0xe6, 0xe6, 0xe6)
        );
    }

    #[test]
    fn faded_color_keeps_part_of_its_opacity() {
        // 40% of an opaque color, and of 10% black.
        assert_eq!(Color::rgb(1, 2, 3).faded(0x66), Color::rgba(1, 2, 3, 0x66));
        assert_eq!(
            Color::rgba(0, 0, 0, 0x1a).faded(0x66),
            Color::rgba(0, 0, 0, 0x0a)
        );
    }

    #[test]
    fn colorref_is_blue_green_red() {
        assert_eq!(Color::rgb(0x12, 0x34, 0x56).colorref(), 0x56_34_12);
    }
}
