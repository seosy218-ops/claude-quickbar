//! Drawing shared by the bar, the phrase box and the menu: anti-aliased rounded blocks
//! from GDI+, ClearType text from GDI. The phrase box and the menu are plain windows
//! that DWM rounds, rings and shades (`shape`, `Canvas`); the bar is a layered window
//! with nothing but its chips (`Layer`).

use windows_sys::Win32::Foundation::{HWND, POINT, RECT, SIZE};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, BeginPaint,
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection, CreateFontW,
    CreateSolidBrush, DIB_RGB_COLORS, DRAW_TEXT_FORMAT, DT_CENTER, DT_LEFT, DT_RIGHT,
    DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, GdiFlush,
    GetDC, GetTextExtentPoint32W, GetTextMetricsW, HBITMAP, HBRUSH, HDC, HFONT, HGDIOBJ,
    PAINTSTRUCT, ReleaseDC, SRCCOPY, SelectObject, SetBkMode, SetTextColor, TEXTMETRICW,
    TRANSPARENT,
};
use windows_sys::Win32::Graphics::GdiPlus::{
    FillModeAlternate, FlushIntentionFlush, FlushIntentionSync, GdipAddPathArc,
    GdipAddPathRectangle, GdipClosePathFigure, GdipCreateBitmapFromScan0, GdipCreateFromHDC,
    GdipCreatePath, GdipCreatePen1, GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics,
    GdipDeletePath, GdipDeletePen, GdipDisposeImage, GdipDrawPath, GdipFillPath, GdipFlush,
    GdipGetImageGraphicsContext, GdipSetPixelOffsetMode, GdipSetSmoothingMode, GdiplusShutdown,
    GdiplusStartup, GdiplusStartupInput, GpBitmap, GpGraphics, GpPath, Ok, PixelFormatAlpha,
    PixelFormatGDI, PixelFormatPAlpha, PixelOffsetModeHalf, SmoothingModeAntiAlias, UnitPixel,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, ULW_ALPHA, UpdateLayeredWindow};

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
    /// Behind the phrase box and the menu: Claude's popovers
    /// (`--cds-surface-popover`), which the prompt card shares.
    pub surface: Color,
    /// The 1px edge DWM draws around the phrase box, opaque, on the
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
    /// The other buttons of a box (Cancel): Claude's secondary button
    /// (`--cds-fill-secondary`, its `-ring`, `-hover`, held down), laid on the surface.
    /// Opaque, but for the ring and the shadow under it.
    pub chip: Color,
    pub chip_ring: Option<Color>,
    pub chip_shadow: Option<Color>,
    pub chip_hover: Color,
    pub chip_down: Color,
    /// Buttons on the bar: the pills above Claude's prompt (Local, the folder, the
    /// branch), as is (`--cds-fill-secondary`) and under the mouse
    /// (`--cds-fill-secondary-hover`), laid on the Code page (`--cds-neutral-40`);
    /// their ring and shadow come from `shadow-field`. The fills are kept opaque, so
    /// the text on them can be ClearType; the ring and the shadow are see-through and
    /// land on whatever the bar floats over.
    /// Claude has no look of its own for a held pill, so held down is under the mouse.
    /// Their text and icons are `text_secondary`.
    pub pill: Color,
    pub pill_ring: Option<Color>,
    pub pill_shadow: Option<Color>,
    pub pill_hover: Color,
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
        // 10% white, and 5% near-black under the mouse, on the page's #f3f3f0.
        pill: Color::rgb(0xf4, 0xf4, 0xf2),
        pill_ring: Some(Color::rgba(0x0b, 0x0b, 0x0b, 0x1a)),
        pill_shadow: Some(Color::rgba(0x00, 0x00, 0x00, 0x0d)),
        pill_hover: Color::rgb(0xe7, 0xe7, 0xe4),
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
        // Measured on the Local pill, on the page's #131313; 10% white alone would
        // make #2b2b2b. Under the mouse, 14% white there. The shadow does not show.
        pill: Color::rgb(0x29, 0x29, 0x29),
        pill_ring: None,
        pill_shadow: None,
        pill_hover: Color::rgb(0x34, 0x34, 0x34),
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
        if !self.graphics.is_null() {
            unsafe { rounded(self.graphics, rect, radius, fill, ring) };
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

/// An anti-aliased block over `rect`, as `Canvas::rounded` and `Layer::rounded` draw it.
unsafe fn rounded(
    graphics: *mut GpGraphics,
    rect: &RECT,
    radius: i32,
    fill: Option<Color>,
    ring: Option<Color>,
) {
    unsafe {
        if let Some(color) = fill
            && let Some(path) = Path::rounded(rect, radius as f32, 0.0)
        {
            let mut brush = std::ptr::null_mut();
            if GdipCreateSolidFill(color.0, &mut brush) == Ok {
                GdipFillPath(graphics, brush.cast(), path.0);
                GdipDeleteBrush(brush.cast());
            }
        }
        // The line runs along pixel centers half a pixel in.
        if let Some(color) = ring
            && let Some(path) = Path::rounded(rect, radius as f32, 0.5)
        {
            let mut pen = std::ptr::null_mut();
            if GdipCreatePen1(color.0, 1.0, UnitPixel, &mut pen) == Ok {
                GdipDrawPath(graphics, pen, path.0);
                GdipDeletePen(pen);
            }
        }
    }
}

/// `PixelFormat32bppPARGB` of gdipluspixelformats.h, which windows-sys has only in parts.
const PARGB: i32 = (11 | (32 << 8) | PixelFormatAlpha | PixelFormatGDI | PixelFormatPAlpha) as i32;

/// A 32-bit, top-down DIB section with a memory DC: what a `Layer` draws on.
struct Dib {
    dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    bits: *mut u8,
}

impl Dib {
    fn new(size: SIZE) -> Option<Dib> {
        unsafe {
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size.cx,
                // Negative: the first row in memory is the top one.
                biHeight: -size.cy,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(
                std::ptr::null_mut(),
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            );
            if bitmap.is_null() || bits.is_null() {
                return None;
            }
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            let old_bitmap = SelectObject(dc, bitmap);
            SetBkMode(dc, TRANSPARENT as i32);
            Some(Dib {
                dc,
                bitmap,
                old_bitmap,
                bits: bits.cast(),
            })
        }
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old_bitmap);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

/// The whole picture of a layered window (`WS_EX_LAYERED`), alpha per pixel: what is
/// left clear shows what is below and lets the mouse through to it, even into another
/// process. Such a window gets no `WM_PAINT`; draw on a `Layer` kept with the window
/// and `present` it whenever the picture changes. There is no DWM edge, corner or shadow.
///
/// Colors go on premultiplied, so see-through fills and rings land on whatever is below.
/// Text comes in two kinds: `text` is ClearType, like the rest of the app, but only right
/// on ground that is opaque where the glyphs fall; `text_gray` is plain anti-aliased and
/// right on any ground, and stays clean when the part is faded afterwards.
pub struct Layer {
    size: SIZE,
    picture: Dib,
    /// Where `text_gray` draws its glyphs white on black before they are laid on.
    ink: Dib,
    /// Null when GDI+ is not running; blocks are then square.
    image: *mut GpBitmap,
    graphics: *mut GpGraphics,
    /// Alpha under a line of ClearType text, kept from one line to the next.
    saved: Vec<u8>,
}

impl Layer {
    /// All clear, `size` in screen pixels.
    pub fn new(size: SIZE) -> Option<Layer> {
        if size.cx <= 0 || size.cy <= 0 {
            return None;
        }
        let (picture, ink) = (Dib::new(size)?, Dib::new(size)?);
        let (mut image, mut graphics) = (std::ptr::null_mut(), std::ptr::null_mut());
        unsafe {
            // GDI+ draws straight into the DIB's memory, premultiplied.
            if GdipCreateBitmapFromScan0(
                size.cx,
                size.cy,
                size.cx * 4,
                PARGB,
                picture.bits,
                &mut image,
            ) == Ok
                && GdipGetImageGraphicsContext(image.cast(), &mut graphics) == Ok
            {
                GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
                GdipSetPixelOffsetMode(graphics, PixelOffsetModeHalf);
            } else {
                graphics = std::ptr::null_mut();
            }
        }
        let mut layer = Layer {
            size,
            picture,
            ink,
            image,
            graphics,
            saved: Vec::new(),
        };
        layer.clear();
        Some(layer)
    }

    /// Whether it is `size`, in screen pixels.
    pub fn fits(&self, size: SIZE) -> bool {
        (self.size.cx, self.size.cy) == (size.cx, size.cy)
    }

    fn pixels(&mut self) -> &mut [u8] {
        let len = (self.size.cx * self.size.cy * 4) as usize;
        unsafe { std::slice::from_raw_parts_mut(self.picture.bits, len) }
    }

    fn read(&self, dib: &Dib) -> &[u8] {
        let len = (self.size.cx * self.size.cy * 4) as usize;
        unsafe { std::slice::from_raw_parts(dib.bits, len) }
    }

    /// Lands what GDI+ and GDI still hold, before the other one or the CPU touches the pixels.
    fn sync(&self) {
        unsafe {
            if !self.graphics.is_null() {
                GdipFlush(self.graphics, FlushIntentionSync);
            }
            GdiFlush();
        }
    }

    /// `rect` cut to the layer.
    fn clip(&self, rect: &RECT) -> RECT {
        RECT {
            left: rect.left.max(0),
            top: rect.top.max(0),
            right: rect.right.min(self.size.cx),
            bottom: rect.bottom.min(self.size.cy),
        }
    }

    /// Byte offsets of the pixels in `rect`, row by row.
    fn offsets(&self, rect: &RECT) -> impl Iterator<Item = usize> + use<> {
        let (rect, width) = (self.clip(rect), self.size.cx);
        (rect.top..rect.bottom)
            .flat_map(move |y| (rect.left..rect.right).map(move |x| ((y * width + x) * 4) as usize))
    }

    /// All clear.
    pub fn clear(&mut self) {
        self.sync();
        self.pixels().fill(0);
    }

    /// As `Canvas::rounded`. Without GDI+ the block is square and the ring left out.
    pub fn rounded(&mut self, rect: &RECT, radius: i32, fill: Option<Color>, ring: Option<Color>) {
        if !self.graphics.is_null() {
            unsafe { rounded(self.graphics, rect, radius, fill, ring) };
            return;
        }
        let Some(color) = fill else { return };
        let [r, g, b, a] = color.channels();
        let pre = |c: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
        let pixel = [pre(b), pre(g), pre(r), a];
        self.sync();
        for at in self.offsets(rect) {
            self.pixels()[at..at + 4].copy_from_slice(&pixel);
        }
    }

    /// A line of ClearType `text` in an opaque `color`, centered in `rect`. Where the
    /// glyphs fall the layer must already be opaque (a solid block); elsewhere the
    /// colors come out wrong.
    pub fn text(&mut self, font: &Font, color: Color, text: &str, rect: &RECT) {
        self.sync();
        // GDI writes 0 into the alpha of every pixel it draws; ClearType mixes with
        // the colors below, which are right, so only the alpha has to come back.
        let mut saved = std::mem::take(&mut self.saved);
        saved.clear();
        let offsets = self.offsets(rect);
        saved.extend(offsets.map(|at| self.read(&self.picture)[at + 3]));
        unsafe {
            let old = SelectObject(self.picture.dc, font.handle());
            SetTextColor(self.picture.dc, color.colorref());
            draw_text(self.picture.dc, text, self.clip(rect));
            SelectObject(self.picture.dc, old);
        }
        self.sync();
        for (at, &alpha) in self.offsets(rect).zip(&saved) {
            self.pixels()[at + 3] = alpha;
        }
        self.saved = saved;
    }

    /// A line of plain anti-aliased `text` in an opaque `color`, centered in `rect`,
    /// on any ground. The glyphs are those of `text`, only without color fringes.
    pub fn text_gray(&mut self, font: &Font, color: Color, text: &str, rect: &RECT) {
        let rect = self.clip(rect);
        unsafe {
            fill(self.ink.dc, &rect, 0);
            let old = SelectObject(self.ink.dc, font.handle());
            SetTextColor(self.ink.dc, 0xff_ff_ff);
            draw_text(self.ink.dc, text, rect);
            SelectObject(self.ink.dc, old);
        }
        self.sync();
        // ClearType white on black leaves how much of each third of a pixel the glyph
        // covers; their mean is how much of the pixel it covers.
        let [r, g, b, _] = color.channels();
        let color = [b as u32, g as u32, r as u32, 255];
        for at in self.offsets(&rect) {
            let ink = &self.read(&self.ink)[at..at + 3];
            let cover = (ink[0] as u32 + ink[1] as u32 + ink[2] as u32 + 1) / 3;
            if cover == 0 {
                continue;
            }
            let pixel = &mut self.pixels()[at..at + 4];
            for (channel, top) in pixel.iter_mut().zip(color) {
                *channel = ((top * cover + *channel as u32 * (255 - cover) + 127) / 255) as u8;
            }
        }
    }

    /// Everything in `rect` at `alpha` (out of 255) of what it was, as CSS `opacity`.
    pub fn fade(&mut self, rect: &RECT, alpha: u8) {
        self.sync();
        for at in self.offsets(rect) {
            for channel in &mut self.pixels()[at..at + 4] {
                *channel = ((*channel as u32 * alpha as u32 + 127) / 255) as u8;
            }
        }
    }

    /// Puts the picture up as `hwnd`'s, which must be `WS_EX_LAYERED`, and sizes the
    /// window to it; where the window is stays with `SetWindowPos`.
    pub fn present(&self, hwnd: HWND) -> bool {
        self.sync();
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let from = POINT { x: 0, y: 0 };
        unsafe {
            UpdateLayeredWindow(
                hwnd,
                std::ptr::null_mut(),
                std::ptr::null(),
                &self.size,
                self.picture.dc,
                &from,
                0,
                &blend,
                ULW_ALPHA,
            ) != 0
        }
    }
}

impl Drop for Layer {
    fn drop(&mut self) {
        unsafe {
            if !self.graphics.is_null() {
                GdipDeleteGraphics(self.graphics);
            }
            if !self.image.is_null() {
                GdipDisposeImage(self.image.cast());
            }
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

    /// The pixel at `x`, `y` as blue, green, red, alpha, premultiplied.
    fn pixel(layer: &Layer, x: i32, y: i32) -> [u8; 4] {
        let at = ((y * layer.size.cx + x) * 4) as usize;
        let bytes = layer.read(&layer.picture);
        [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]
    }

    #[test]
    fn layer_leaves_round_corners_and_gaps_clear() {
        let _gdiplus = GdiPlus::start();
        let mut layer = Layer::new(SIZE { cx: 40, cy: 20 }).expect("layer");
        let chip = RECT {
            left: 2,
            top: 2,
            right: 30,
            bottom: 18,
        };
        let fill = Color::rgb(0x29, 0x29, 0x29);
        layer.rounded(&chip, 6, Some(fill), None);
        assert_eq!(pixel(&layer, 15, 10), [0x29, 0x29, 0x29, 0xff]);
        assert_eq!(pixel(&layer, 2, 2)[3], 0, "corner");
        assert_eq!(pixel(&layer, 35, 10), [0; 4], "gap");
        // ClearType text gives the alpha it wrote over back.
        let font = Font::new(FONT, 13);
        layer.text(&font, Color::rgb(0xc3, 0xc2, 0xb7), "Wg", &chip);
        for x in 8..24 {
            assert_eq!(pixel(&layer, x, 10)[3], 0xff, "under text at {x}");
        }
        // Faded, every channel goes down alike.
        layer.fade(&chip, 0x66);
        assert_eq!(pixel(&layer, 3, 10), [0x10, 0x10, 0x10, 0x66]);
    }

    #[test]
    fn gray_text_lands_on_clear_ground() {
        let _gdiplus = GdiPlus::start();
        let mut layer = Layer::new(SIZE { cx: 40, cy: 20 }).expect("layer");
        let font = Font::new(FONT, 16);
        let all = RECT {
            left: 0,
            top: 0,
            right: 40,
            bottom: 20,
        };
        layer.text_gray(&font, Color::rgb(0xff, 0xff, 0xff), "II", &all);
        let alphas: Vec<u8> = (0..40).map(|x| pixel(&layer, x, 10)[3]).collect();
        assert!(alphas.iter().any(|&a| a > 0x80), "{alphas:?}");
        assert!(alphas.contains(&0), "{alphas:?}");
        // Premultiplied: no channel above the alpha.
        for x in 0..40 {
            let [b, g, r, a] = pixel(&layer, x, 10);
            assert!(b <= a && g <= a && r <= a);
        }
    }
}
