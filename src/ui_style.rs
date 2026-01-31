//! Small GDI theme for native controls; no rendering loop or graphics runtime.
use usb_eject::windows::wide;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::*},
    UI::Controls::*,
};

pub const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | ((g as u32) << 8) | ((b as u32) << 16)
}
pub const INK: COLORREF = rgb(27, 42, 56);
pub const MUTED: COLORREF = rgb(100, 115, 130);
pub const ACCENT: COLORREF = rgb(17, 119, 106);
pub const TINT: COLORREF = rgb(230, 244, 240);
pub const SURFACE: COLORREF = rgb(245, 247, 249);
pub const LINE: COLORREF = rgb(225, 231, 236);
pub const WHITE: COLORREF = rgb(255, 255, 255);

pub struct Theme {
    pub dpi: u32,
    pub normal: HFONT,
    pub title: HFONT,
    pub heading: HFONT,
    pub small: HFONT,
    pub white: HBRUSH,
}
impl Theme {
    pub fn new(dpi: u32) -> Self {
        let dpi = dpi.max(96);
        let font = |size: i32, weight: i32| unsafe {
            CreateFontW(
                -(size * dpi as i32 / 96),
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                CLEARTYPE_QUALITY as u32,
                FF_SWISS as u32,
                wide("Segoe UI").as_ptr(),
            )
        };
        Self {
            dpi,
            normal: font(15, 400),
            title: font(25, 600),
            heading: font(16, 500),
            small: font(13, 400),
            white: unsafe { CreateSolidBrush(WHITE) },
        }
    }
    pub fn s(&self, n: i32) -> i32 {
        n * self.dpi as i32 / 96
    }
    pub fn caption(hwnd: HWND) {
        let corner = DWMWCP_ROUND;
        // Windows 10 may reject optional Windows 11 attributes; ordinary native
        // non-client rendering remains available in that case.
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                (&corner as *const i32).cast(),
                4,
            );
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_CAPTION_COLOR as u32,
                (&WHITE as *const u32).cast(),
                4,
            );
        }
    }
    pub fn button(&self, item: &DRAWITEMSTRUCT, primary: bool, text: &str) {
        let disabled = item.itemState & ODS_DISABLED != 0;
        let pressed = item.itemState & ODS_SELECTED != 0;
        let fill = if disabled {
            SURFACE
        } else if primary {
            if pressed { rgb(10, 91, 80) } else { ACCENT }
        } else if pressed {
            TINT
        } else {
            SURFACE
        };
        let foreground = if disabled {
            rgb(155, 165, 174)
        } else if primary {
            WHITE
        } else {
            INK
        };
        rounded(
            item.hDC,
            item.rcItem,
            self.s(9),
            fill,
            if primary && !disabled { fill } else { LINE },
        );
        text_at(
            item.hDC,
            self.normal,
            text,
            item.rcItem,
            foreground,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        if item.itemState & ODS_FOCUS != 0 {
            let mut focus = item.rcItem;
            focus.left += 4;
            focus.top += 4;
            focus.right -= 4;
            focus.bottom -= 4;
            unsafe {
                DrawFocusRect(item.hDC, &focus);
            }
        }
    }
}
impl Drop for Theme {
    fn drop(&mut self) {
        unsafe {
            for object in [
                self.normal,
                self.title,
                self.heading,
                self.small,
                self.white,
            ] {
                if !object.is_null() {
                    DeleteObject(object);
                }
            }
        }
    }
}
pub fn rounded(dc: HDC, rect: RECT, radius: i32, fill: COLORREF, border: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(fill);
        let pen = CreatePen(PS_SOLID, 1, border);
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, pen);
        RoundRect(
            dc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius,
            radius,
        );
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        DeleteObject(brush);
        DeleteObject(pen);
    }
}
pub fn text_at(dc: HDC, font: HFONT, value: &str, mut rect: RECT, color: COLORREF, flags: u32) {
    let value = wide(value);
    unsafe {
        let old = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, color);
        DrawTextW(
            dc,
            value.as_ptr(),
            (value.len() - 1) as i32,
            &mut rect,
            flags | DT_NOPREFIX,
        );
        SelectObject(dc, old);
    }
}
pub fn separator(dc: HDC, y: i32, width: i32) {
    unsafe {
        let pen = CreatePen(PS_SOLID, 1, LINE);
        let old = SelectObject(dc, pen);
        MoveToEx(dc, 0, y, std::ptr::null_mut());
        LineTo(dc, width, y);
        SelectObject(dc, old);
        DeleteObject(pen);
    }
}
