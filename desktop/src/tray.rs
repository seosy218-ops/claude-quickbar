//! The notification-area icon: a ⚡ like the bar's, whose menu (on either click) the bar's window shows.

use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, GetSystemMetrics, HICON, ICONINFO, IDI_APPLICATION, LoadIconW,
    RegisterWindowMessageW, SM_CXSMICON,
};

use crate::paint::{Font, Palette, draw_text, fill};
use crate::win::wide;

pub struct Tray {
    hwnd: HWND,
    /// Sent to `hwnd` with the mouse message in `lparam`.
    message: u32,
    /// `None` when drawing failed and the stock icon stands in.
    icon: Option<HICON>,
}

impl Tray {
    pub fn new(hwnd: HWND, message: u32, palette: &Palette) -> Tray {
        Tray {
            hwnd,
            message,
            icon: draw_icon(palette),
        }
    }

    /// Draws the icon again in `palette`; the tray shows it at once if it has the icon.
    pub fn repaint(&mut self, palette: &Palette) {
        let old = std::mem::replace(&mut self.icon, draw_icon(palette));
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &self.data());
            if let Some(icon) = old {
                DestroyIcon(icon);
            }
        }
    }

    /// Puts the icon in the tray; false while the taskbar is not taking it (early at sign-in).
    pub fn add(&self) -> bool {
        let data = self.data();
        // An add that timed out may still have gone through.
        unsafe {
            Shell_NotifyIconW(NIM_ADD, &data) != 0 || Shell_NotifyIconW(NIM_MODIFY, &data) != 0
        }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: self.message,
            hIcon: self
                .icon
                .unwrap_or_else(|| unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) }),
            ..unsafe { std::mem::zeroed() }
        };
        let tip = wide("quickbar");
        data.szTip[..tip.len()].copy_from_slice(&tip);
        data
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &self.data());
            if let Some(icon) = self.icon {
                DestroyIcon(icon);
            }
        }
    }
}

/// Broadcast when Explorer restarts; every icon has to be added again.
/// `None` when the message could not be registered.
pub fn taskbar_created() -> Option<u32> {
    static MESSAGE: OnceLock<u32> = OnceLock::new();
    let message = *MESSAGE.get_or_init(|| {
        let name = wide("TaskbarCreated");
        unsafe { RegisterWindowMessageW(name.as_ptr()) }
    });
    (message != 0).then_some(message)
}

/// The bar's ⚡ on the bar's background, at the tray's icon size.
fn draw_icon(palette: &Palette) -> Option<HICON> {
    unsafe {
        let size = GetSystemMetrics(SM_CXSMICON);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                // Top-down rows.
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            },
            ..std::mem::zeroed()
        };
        let mut bits = std::ptr::null_mut();
        let color = CreateDIBSection(
            std::ptr::null_mut(),
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        if color.is_null() {
            return None;
        }
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            DeleteObject(color);
            return None;
        }
        let old_bitmap = SelectObject(dc, color);
        let rect = RECT {
            left: 0,
            top: 0,
            right: size,
            bottom: size,
        };
        fill(dc, &rect, palette.surface.colorref());
        let symbol = Font::new("Segoe UI Symbol", size);
        let old_font = SelectObject(dc, symbol.handle());
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, palette.text.colorref());
        draw_text(dc, "\u{26a1}", rect);
        SelectObject(dc, old_font);
        SelectObject(dc, old_bitmap);
        DeleteDC(dc);
        GdiFlush();
        // GDI leaves alpha at 0, which would make every pixel see-through.
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
        for pixel in pixels {
            *pixel |= 0xff00_0000;
        }
        // All zeros: opaque everywhere. Rows of a 1-bit bitmap are padded to 16 bits.
        let mask_bits = vec![0u8; (size as usize).div_ceil(16) * 2 * size as usize];
        let mask = CreateBitmap(size, size, 1, 1, mask_bits.as_ptr() as *const _);
        let icon = CreateIconIndirect(&ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        });
        DeleteObject(mask);
        DeleteObject(color);
        (!icon.is_null()).then_some(icon)
    }
}
