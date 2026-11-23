//! The picture the window shows: a block of pixels that the canvas draws shapes into and the system draws text
//! onto. It needs no window, which is how a snapshot is made.

use crate::canvas::{Canvas, Rect};
use crate::win32::*;

/// The fonts of the previewer, in sizes for a scale of 1.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Font {
    Small,
    Body,
    Bold,
    Head,
    Title,
    Icon,
    IconBig,
}

const FONTS: [(Font, f32, i32, &str); 7] = [
    (Font::Small, 12.5, FW_NORMAL, "Segoe UI"),
    (Font::Body, 14.5, FW_NORMAL, "Segoe UI"),
    (Font::Bold, 14.5, FW_SEMIBOLD, "Segoe UI"),
    (Font::Head, 17.5, FW_SEMIBOLD, "Segoe UI"),
    (Font::Title, 20.0, FW_SEMIBOLD, "Segoe UI"),
    (Font::Icon, 15.0, FW_NORMAL, "Segoe Fluent Icons"),
    (Font::IconBig, 19.0, FW_NORMAL, "Segoe Fluent Icons"),
];

pub struct Surface {
    dc: HDC,
    bitmap: HANDLE,
    first_bitmap: HANDLE,
    bits: *mut u32,
    pub w: i32,
    pub h: i32,
    /// 1 at 96 dots per inch
    pub scale: f32,
    fonts: Vec<(Font, HANDLE, f32)>,
    clip: Option<Rect>,
}

impl Surface {
    pub fn new(w: i32, h: i32, scale: f32) -> Surface {
        let dc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
        let mut s = Surface { dc, bitmap: std::ptr::null_mut(), first_bitmap: std::ptr::null_mut(), bits: std::ptr::null_mut(), w: 0, h: 0, scale: 0.0, fonts: Vec::new(), clip: None };
        unsafe { SetBkMode(dc, TRANSPARENT) };
        s.resize(w, h, scale);
        s
    }

    /// Fits the pixels to a new size and the fonts to a new scale.
    pub fn resize(&mut self, w: i32, h: i32, scale: f32) {
        let (w, h) = (w.max(1), h.max(1));
        unsafe {
            if (w, h) != (self.w, self.h) {
                let info = BITMAPINFOHEADER { biSize: 40, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, ..Default::default() };
                let mut bits = std::ptr::null_mut();
                let bitmap = CreateDIBSection(self.dc, &info, 0, &mut bits, std::ptr::null_mut(), 0);
                let old = SelectObject(self.dc, bitmap);
                if self.bitmap.is_null() {
                    self.first_bitmap = old;
                } else {
                    DeleteObject(self.bitmap);
                }
                (self.bitmap, self.bits, self.w, self.h) = (bitmap, bits as *mut u32, w, h);
            }
            if scale != self.scale {
                for (_, f, _) in self.fonts.drain(..) {
                    DeleteObject(f);
                }
                for (font, size, weight, face) in FONTS {
                    let px = (size * scale).round();
                    let f = CreateFontW(-(px as i32), 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, 0, 0, CLEARTYPE_QUALITY, 0, wide(face).as_ptr());
                    // the height of a line in this font
                    SelectObject(self.dc, f);
                    let mut size = SIZE::default();
                    GetTextExtentPoint32W(self.dc, wide("Ag").as_ptr(), 2, &mut size);
                    self.fonts.push((font, f, size.cy as f32));
                }
                self.scale = scale;
            }
        }
    }

    /// The canvas over the pixels, for shapes.
    pub fn canvas(&mut self) -> Canvas<'_> {
        unsafe { GdiFlush() };
        let px = unsafe { std::slice::from_raw_parts_mut(self.bits, (self.w * self.h) as usize) };
        let mut c = Canvas::new(self.w, self.h, px);
        if let Some(r) = self.clip {
            c.clip = ((r.x as i32).max(0), (r.y as i32).max(0), (r.right() as i32).min(self.w), (r.bottom() as i32).min(self.h));
        }
        c
    }

    pub fn pixels(&self) -> &[u32] {
        unsafe {
            GdiFlush();
            std::slice::from_raw_parts(self.bits, (self.w * self.h) as usize)
        }
    }

    /// Shapes and text stay inside `r` until the clip is taken away with None.
    pub fn set_clip(&mut self, r: Option<Rect>) {
        self.clip = r;
        unsafe {
            SelectClipRgn(self.dc, std::ptr::null_mut());
            if let Some(r) = r {
                IntersectClipRect(self.dc, r.x as i32, r.y as i32, r.right() as i32, r.bottom() as i32);
            }
        }
    }

    fn font(&self, f: Font) -> (HANDLE, f32) {
        let e = self.fonts.iter().find(|e| e.0 == f).unwrap();
        (e.1, e.2)
    }

    /// The height of a line of text.
    pub fn line(&self, f: Font) -> f32 {
        self.font(f).1
    }

    pub fn measure(&mut self, s: &str, f: Font) -> f32 {
        let w: Vec<u16> = s.encode_utf16().collect();
        let mut size = SIZE::default();
        unsafe {
            SelectObject(self.dc, self.font(f).0);
            GetTextExtentPoint32W(self.dc, w.as_ptr(), w.len() as i32, &mut size);
        }
        size.cx as f32
    }

    /// Text with its top-left corner at (x, y). Returns its width.
    pub fn text(&mut self, x: f32, y: f32, s: &str, f: Font, color: u32) -> f32 {
        let w: Vec<u16> = s.encode_utf16().collect();
        let mut size = SIZE::default();
        unsafe {
            SelectObject(self.dc, self.font(f).0);
            // GDI takes 0x00BBGGRR
            SetTextColor(self.dc, (color & 0xFF) << 16 | (color & 0xFF00) | (color >> 16) & 0xFF);
            TextOutW(self.dc, x.round() as i32, y.round() as i32, w.as_ptr(), w.len() as i32);
            GetTextExtentPoint32W(self.dc, w.as_ptr(), w.len() as i32, &mut size);
        }
        size.cx as f32
    }

    /// Text in the middle of the height of `r`: at its left (align 0), centred (1) or at its right (2).
    pub fn text_in(&mut self, r: Rect, s: &str, f: Font, color: u32, align: u8) -> f32 {
        let w = self.measure(s, f);
        let x = match align {
            0 => r.x,
            1 => r.x + (r.w - w) * 0.5,
            _ => r.right() - w,
        };
        self.text(x, r.y + (r.h - self.line(f)) * 0.5, s, f, color)
    }

    /// Text cut with an ellipsis so that it fits `max` pixels.
    pub fn fit(&mut self, s: &str, f: Font, max: f32) -> String {
        if self.measure(s, f) <= max {
            return s.to_string();
        }
        let chars: Vec<char> = s.chars().collect();
        let (mut lo, mut hi) = (0, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let t: String = chars[..mid].iter().collect::<String>() + "…";
            if self.measure(&t, f) <= max {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        chars[..lo].iter().collect::<String>() + "…"
    }

    /// Text broken into lines of at most `max` pixels, at spaces; a word wider than a line is cut where it fills it.
    pub fn wrap(&mut self, text: &str, f: Font, max: f32) -> Vec<String> {
        let mut lines = Vec::new();
        for para in text.split('\n') {
            let mut line = String::new();
            for word in para.split_whitespace() {
                let longer = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
                if self.measure(&longer, f) <= max {
                    line = longer;
                    continue;
                }
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                let mut rest: Vec<char> = word.chars().collect();
                while rest.len() > 1 && self.measure(&rest.iter().collect::<String>(), f) > max {
                    let (mut lo, mut hi) = (1, rest.len());
                    while lo < hi {
                        let mid = (lo + hi).div_ceil(2);
                        if self.measure(&rest[..mid].iter().collect::<String>(), f) <= max {
                            lo = mid;
                        } else {
                            hi = mid - 1;
                        }
                    }
                    lines.push(rest.drain(..lo).collect());
                }
                line = rest.into_iter().collect();
            }
            lines.push(line);
        }
        lines
    }

    /// Copies the picture to a window's device context.
    pub fn show(&self, dc: HDC) {
        unsafe { BitBlt(dc, 0, 0, self.w, self.h, self.dc, 0, 0, SRCCOPY) };
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.first_bitmap);
            DeleteObject(self.bitmap);
            for (_, f, _) in &self.fonts {
                DeleteObject(*f);
            }
            DeleteDC(self.dc);
        }
    }
}
