//! The Win32 declarations the previewer uses, and nothing else.
#![allow(non_snake_case, non_camel_case_types, dead_code, clippy::upper_case_acronyms, clippy::too_many_arguments)]

use std::ffi::c_void;

pub type HANDLE = *mut c_void;
pub type HWND = *mut c_void;
pub type HDC = *mut c_void;
pub type BOOL = i32;
pub type WNDPROC = unsafe extern "system" fn(HWND, u32, usize, isize) -> isize;

#[repr(C)]
pub struct WNDCLASSEXW {
    pub cbSize: u32,
    pub style: u32,
    pub lpfnWndProc: WNDPROC,
    pub cbClsExtra: i32,
    pub cbWndExtra: i32,
    pub hInstance: HANDLE,
    pub hIcon: HANDLE,
    pub hCursor: HANDLE,
    pub hbrBackground: HANDLE,
    pub lpszMenuName: *const u16,
    pub lpszClassName: *const u16,
    pub hIconSm: HANDLE,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SIZE {
    pub cx: i32,
    pub cy: i32,
}

#[repr(C)]
pub struct MSG {
    pub hwnd: HWND,
    pub message: u32,
    pub wParam: usize,
    pub lParam: isize,
    pub time: u32,
    pub pt: POINT,
}

#[repr(C)]
pub struct PAINTSTRUCT {
    pub hdc: HDC,
    pub fErase: BOOL,
    pub rcPaint: RECT,
    pub fRestore: BOOL,
    pub fIncUpdate: BOOL,
    pub rgbReserved: [u8; 32],
}

#[repr(C)]
#[derive(Default)]
pub struct BITMAPINFOHEADER {
    pub biSize: u32,
    pub biWidth: i32,
    pub biHeight: i32,
    pub biPlanes: u16,
    pub biBitCount: u16,
    pub biCompression: u32,
    pub biSizeImage: u32,
    pub biXPelsPerMeter: i32,
    pub biYPelsPerMeter: i32,
    pub biClrUsed: u32,
    pub biClrImportant: u32,
}

#[repr(C)]
pub struct MINMAXINFO {
    pub ptReserved: POINT,
    pub ptMaxSize: POINT,
    pub ptMaxPosition: POINT,
    pub ptMinTrackSize: POINT,
    pub ptMaxTrackSize: POINT,
}

#[repr(C)]
pub struct OPENFILENAMEW {
    pub lStructSize: u32,
    pub hwndOwner: HWND,
    pub hInstance: HANDLE,
    pub lpstrFilter: *const u16,
    pub lpstrCustomFilter: *mut u16,
    pub nMaxCustFilter: u32,
    pub nFilterIndex: u32,
    pub lpstrFile: *mut u16,
    pub nMaxFile: u32,
    pub lpstrFileTitle: *mut u16,
    pub nMaxFileTitle: u32,
    pub lpstrInitialDir: *const u16,
    pub lpstrTitle: *const u16,
    pub Flags: u32,
    pub nFileOffset: u16,
    pub nFileExtension: u16,
    pub lpstrDefExt: *const u16,
    pub lCustData: isize,
    pub lpfnHook: *const c_void,
    pub lpTemplateName: *const u16,
    pub pvReserved: *mut c_void,
    pub dwReserved: u32,
    pub FlagsEx: u32,
}

#[repr(C)]
pub struct ICONINFO {
    pub fIcon: BOOL,
    pub xHotspot: u32,
    pub yHotspot: u32,
    pub hbmMask: HANDLE,
    pub hbmColor: HANDLE,
}

#[repr(C)]
pub struct WAVEFORMATEX {
    pub wFormatTag: u16,
    pub nChannels: u16,
    pub nSamplesPerSec: u32,
    pub nAvgBytesPerSec: u32,
    pub nBlockAlign: u16,
    pub wBitsPerSample: u16,
    pub cbSize: u16,
}

#[repr(C)]
pub struct WAVEHDR {
    pub lpData: *mut u8,
    pub dwBufferLength: u32,
    pub dwBytesRecorded: u32,
    pub dwUser: usize,
    pub dwFlags: u32,
    pub dwLoops: u32,
    pub lpNext: *mut WAVEHDR,
    pub reserved: usize,
}

/// MMTIME: the kind of time, then a union of which only the first four bytes are read here
#[repr(C)]
pub struct MMTIME {
    pub wType: u32,
    pub value: u32,
    pub rest: u32,
}

pub const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
pub const CW_USEDEFAULT: i32 = 0x8000_0000u32 as i32;
pub const SW_SHOW: i32 = 5;
pub const GWLP_USERDATA: i32 = -21;
pub const IDC_ARROW: usize = 32512;
pub const IDC_IBEAM: usize = 32513;
pub const IDC_HAND: usize = 32649;

pub const WM_CREATE: u32 = 0x0001;
pub const WM_DESTROY: u32 = 0x0002;
pub const WM_SIZE: u32 = 0x0005;
pub const WM_PAINT: u32 = 0x000F;
pub const WM_ERASEBKGND: u32 = 0x0014;
pub const WM_SETCURSOR: u32 = 0x0020;
pub const WM_GETMINMAXINFO: u32 = 0x0024;
pub const WM_SETICON: u32 = 0x0080;
pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_CHAR: u32 = 0x0102;
pub const WM_TIMER: u32 = 0x0113;
pub const WM_MOUSEMOVE: u32 = 0x0200;
pub const WM_LBUTTONDOWN: u32 = 0x0201;
pub const WM_LBUTTONUP: u32 = 0x0202;
pub const WM_LBUTTONDBLCLK: u32 = 0x0203;
pub const WM_MOUSEWHEEL: u32 = 0x020A;
pub const WM_DROPFILES: u32 = 0x0233;
pub const WM_DPICHANGED: u32 = 0x02E0;
pub const HTCLIENT: u16 = 1;

pub const VK_BACK: usize = 0x08;
pub const VK_RETURN: usize = 0x0D;
pub const VK_CONTROL: i32 = 0x11;
pub const VK_ESCAPE: usize = 0x1B;
pub const VK_SPACE: usize = 0x20;
pub const VK_END: usize = 0x23;
pub const VK_HOME: usize = 0x24;
pub const VK_LEFT: usize = 0x25;
pub const VK_UP: usize = 0x26;
pub const VK_RIGHT: usize = 0x27;
pub const VK_DOWN: usize = 0x28;

pub const SRCCOPY: u32 = 0x00CC_0020;
pub const TRANSPARENT: i32 = 1;
pub const CLEARTYPE_QUALITY: u32 = 5;
pub const DEFAULT_CHARSET: u32 = 1;
pub const FW_NORMAL: i32 = 400;
pub const FW_SEMIBOLD: i32 = 600;

pub const MF_STRING: u32 = 0;
pub const MF_SEPARATOR: u32 = 0x800;
pub const MF_CHECKED: u32 = 8;
pub const MF_GRAYED: u32 = 1;
pub const TPM_RETURNCMD: u32 = 0x100;
pub const TPM_RIGHTALIGN: u32 = 8;
pub const MB_ICONERROR: u32 = 0x10;
pub const MB_ICONINFORMATION: u32 = 0x40;
pub const CF_UNICODETEXT: u32 = 13;
pub const GMEM_MOVEABLE: u32 = 2;
pub const OFN_OVERWRITEPROMPT: u32 = 2;
pub const OFN_PATHMUSTEXIST: u32 = 0x800;
pub const OFN_FILEMUSTEXIST: u32 = 0x1000;
pub const OFN_EXPLORER: u32 = 0x80000;
pub const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
pub const DWMWA_CAPTION_COLOR: u32 = 35;
pub const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4;
pub const SWP_NOZORDER: u32 = 4;
pub const SWP_NOACTIVATE: u32 = 0x10;

pub const WAVE_MAPPER: u32 = u32::MAX;
pub const CALLBACK_EVENT: u32 = 0x0005_0000;
pub const WHDR_INQUEUE: u32 = 0x10;
pub const TIME_SAMPLES: u32 = 2;
pub const TIME_BYTES: u32 = 4;

#[link(name = "user32")]
extern "system" {
    pub fn RegisterClassExW(c: *const WNDCLASSEXW) -> u16;
    pub fn CreateWindowExW(ex: u32, class: *const u16, name: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: HWND, menu: HANDLE, inst: HANDLE, param: *mut c_void) -> HWND;
    pub fn DefWindowProcW(h: HWND, m: u32, w: usize, l: isize) -> isize;
    pub fn DestroyWindow(h: HWND) -> BOOL;
    pub fn ShowWindow(h: HWND, cmd: i32) -> BOOL;
    pub fn GetMessageW(m: *mut MSG, h: HWND, min: u32, max: u32) -> BOOL;
    pub fn TranslateMessage(m: *const MSG) -> BOOL;
    pub fn DispatchMessageW(m: *const MSG) -> isize;
    pub fn PostQuitMessage(code: i32);
    pub fn GetClientRect(h: HWND, r: *mut RECT) -> BOOL;
    pub fn BeginPaint(h: HWND, ps: *mut PAINTSTRUCT) -> HDC;
    pub fn EndPaint(h: HWND, ps: *const PAINTSTRUCT) -> BOOL;
    pub fn LoadCursorW(inst: HANDLE, name: *const u16) -> HANDLE;
    pub fn SetCursor(c: HANDLE) -> HANDLE;
    pub fn SetWindowLongPtrW(h: HWND, idx: i32, v: isize) -> isize;
    pub fn GetWindowLongPtrW(h: HWND, idx: i32) -> isize;
    pub fn SetWindowPos(h: HWND, after: HWND, x: i32, y: i32, w: i32, hh: i32, flags: u32) -> BOOL;
    pub fn SetWindowTextW(h: HWND, s: *const u16) -> BOOL;
    pub fn SendMessageW(h: HWND, m: u32, w: usize, l: isize) -> isize;
    pub fn InvalidateRect(h: HWND, r: *const RECT, erase: BOOL) -> BOOL;
    pub fn SetTimer(h: HWND, id: usize, ms: u32, f: *const c_void) -> usize;
    pub fn GetDpiForWindow(h: HWND) -> u32;
    pub fn GetDpiForSystem() -> u32;
    pub fn SetProcessDpiAwarenessContext(ctx: isize) -> BOOL;
    pub fn AdjustWindowRectExForDpi(r: *mut RECT, style: u32, menu: BOOL, ex: u32, dpi: u32) -> BOOL;
    pub fn SetCapture(h: HWND) -> HWND;
    pub fn ReleaseCapture() -> BOOL;
    pub fn GetKeyState(vk: i32) -> i16;
    pub fn MessageBoxW(h: HWND, text: *const u16, caption: *const u16, kind: u32) -> i32;
    pub fn CreatePopupMenu() -> HANDLE;
    pub fn AppendMenuW(menu: HANDLE, flags: u32, id: usize, text: *const u16) -> BOOL;
    pub fn TrackPopupMenu(menu: HANDLE, flags: u32, x: i32, y: i32, reserved: i32, h: HWND, r: *const RECT) -> BOOL;
    pub fn DestroyMenu(menu: HANDLE) -> BOOL;
    pub fn ClientToScreen(h: HWND, p: *mut POINT) -> BOOL;
    pub fn OpenClipboard(h: HWND) -> BOOL;
    pub fn EmptyClipboard() -> BOOL;
    pub fn SetClipboardData(fmt: u32, mem: HANDLE) -> HANDLE;
    pub fn CloseClipboard() -> BOOL;
    pub fn CreateIconIndirect(info: *const ICONINFO) -> HANDLE;
}

#[link(name = "gdi32")]
extern "system" {
    pub fn CreateCompatibleDC(dc: HDC) -> HDC;
    pub fn DeleteDC(dc: HDC) -> BOOL;
    pub fn CreateDIBSection(dc: HDC, bmi: *const BITMAPINFOHEADER, usage: u32, bits: *mut *mut c_void, sect: HANDLE, off: u32) -> HANDLE;
    pub fn CreateBitmap(w: i32, h: i32, planes: u32, bits_per_pixel: u32, bits: *const c_void) -> HANDLE;
    pub fn SelectObject(dc: HDC, o: HANDLE) -> HANDLE;
    pub fn DeleteObject(o: HANDLE) -> BOOL;
    pub fn BitBlt(dst: HDC, x: i32, y: i32, w: i32, h: i32, src: HDC, sx: i32, sy: i32, rop: u32) -> BOOL;
    pub fn CreateFontW(h: i32, w: i32, esc: i32, orient: i32, weight: i32, italic: u32, underline: u32, strike: u32, charset: u32, outprec: u32, clipprec: u32, quality: u32, pitch: u32, face: *const u16) -> HANDLE;
    pub fn SetTextColor(dc: HDC, c: u32) -> u32;
    pub fn SetBkMode(dc: HDC, mode: i32) -> i32;
    pub fn TextOutW(dc: HDC, x: i32, y: i32, s: *const u16, n: i32) -> BOOL;
    pub fn GetTextExtentPoint32W(dc: HDC, s: *const u16, n: i32, size: *mut SIZE) -> BOOL;
    pub fn GdiFlush() -> BOOL;
    pub fn IntersectClipRect(dc: HDC, l: i32, t: i32, r: i32, b: i32) -> i32;
    pub fn SelectClipRgn(dc: HDC, rgn: HANDLE) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    pub fn GetModuleHandleW(name: *const u16) -> HANDLE;
    pub fn GlobalAlloc(flags: u32, bytes: usize) -> HANDLE;
    pub fn GlobalLock(mem: HANDLE) -> *mut c_void;
    pub fn GlobalUnlock(mem: HANDLE) -> BOOL;
    pub fn CreateEventW(attrs: *const c_void, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    pub fn SetEvent(h: HANDLE) -> BOOL;
    pub fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    pub fn CloseHandle(h: HANDLE) -> BOOL;
    pub fn GetConsoleWindow() -> HWND;
    pub fn GetConsoleProcessList(list: *mut u32, n: u32) -> u32;
    pub fn FreeConsole() -> BOOL;
}

#[link(name = "winmm")]
extern "system" {
    pub fn waveOutOpen(out: *mut HANDLE, device: u32, format: *const WAVEFORMATEX, callback: usize, instance: usize, flags: u32) -> u32;
    pub fn waveOutClose(h: HANDLE) -> u32;
    pub fn waveOutPrepareHeader(h: HANDLE, hdr: *mut WAVEHDR, size: u32) -> u32;
    pub fn waveOutUnprepareHeader(h: HANDLE, hdr: *mut WAVEHDR, size: u32) -> u32;
    pub fn waveOutWrite(h: HANDLE, hdr: *mut WAVEHDR, size: u32) -> u32;
    pub fn waveOutReset(h: HANDLE) -> u32;
    pub fn waveOutPause(h: HANDLE) -> u32;
    pub fn waveOutRestart(h: HANDLE) -> u32;
    pub fn waveOutGetPosition(h: HANDLE, time: *mut MMTIME, size: u32) -> u32;
}

#[link(name = "shell32")]
extern "system" {
    pub fn DragAcceptFiles(h: HWND, accept: BOOL);
    pub fn DragQueryFileW(drop: HANDLE, index: u32, buf: *mut u16, n: u32) -> u32;
    pub fn DragFinish(drop: HANDLE);
}

#[link(name = "comdlg32")]
extern "system" {
    pub fn GetOpenFileNameW(o: *mut OPENFILENAMEW) -> BOOL;
    pub fn GetSaveFileNameW(o: *mut OPENFILENAMEW) -> BOOL;
}

#[link(name = "dwmapi")]
extern "system" {
    pub fn DwmSetWindowAttribute(h: HWND, attr: u32, value: *const c_void, size: u32) -> i32;
}

/// A Rust string as the zero-ended UTF-16 that Win32 takes.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
