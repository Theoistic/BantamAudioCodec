//! The window: Win32 messages in, the view's picture out, and the things only a window can do (file dialogs, menus,
//! the clipboard, message boxes).

use crate::surface::Surface;
use crate::view::{clock, Request, View};
use crate::win32::*;
use crate::{Options, Shot};
use std::ffi::c_void;
use std::time::Instant;

const TITLE: &str = "BantamAudioCodec Previewer";

struct App {
    view: View,
    surface: Surface,
    hwnd: HWND,
    /// the position at the last frame drawn: a frame is drawn again only when something moved
    drawn_at: u64,
    started: Instant,
    quit_after: Option<f64>,
}

pub fn snapshot(shot: &Shot, out: &str) -> Result<(), String> {
    let mut view = View::new(true);
    if let Some(f) = &shot.file {
        if !view.load(f) {
            return Err(view.error.clone().unwrap_or_default());
        }
    }
    let rate = view.doc.as_ref().map_or(1, |d| d.rate);
    view.frozen = Some((shot.at.max(0.0) * rate as f64) as u64);
    view.spectro = shot.spectrogram;
    view.details = shot.details;
    view.query = shot.search.clone();
    let mut s = Surface::new((shot.width as f32 * shot.scale) as i32, (shot.height as f32 * shot.scale) as i32, shot.scale);
    // twice: the first frame works out where things are, the second follows the sound to its row and knows what
    // the mouse rests on
    view.draw(&mut s);
    if let Some((x, y)) = shot.mouse {
        view.mouse_move(x * shot.scale, y * shot.scale);
    }
    view.draw(&mut s);
    std::fs::write(out, crate::canvas::png(s.w as u32, s.h as u32, s.pixels())).map_err(|e| format!("{out}: {e}"))
}

pub fn run(o: Options) -> Result<(), String> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let inst = GetModuleHandleW(std::ptr::null());
        let class = wide("BacPreviewer");
        let icon = app_icon(32);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: wndproc,
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: inst,
            hIcon: icon,
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW as *const u16),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
            hIconSm: app_icon(16),
        };
        if RegisterClassExW(&wc) == 0 {
            return Err("the window class could not be registered".into());
        }
        let dpi = GetDpiForSystem().max(96);
        let scale = dpi as f32 / 96.0;
        let mut view = View::new(o.mute);
        if let Some(f) = &o.file {
            view.load(f);
        }
        if o.autoplay {
            view.toggle_play();
        }
        let app = Box::into_raw(Box::new(App { view, surface: Surface::new(16, 16, scale), hwnd: std::ptr::null_mut(), drawn_at: u64::MAX, started: Instant::now(), quit_after: o.quit_after }));
        let mut rc = RECT { left: 0, top: 0, right: (1090.0 * scale) as i32, bottom: (820.0 * scale) as i32 };
        AdjustWindowRectExForDpi(&mut rc, WS_OVERLAPPEDWINDOW, 0, 0, dpi);
        let hwnd = CreateWindowExW(0, class.as_ptr(), wide(TITLE).as_ptr(), WS_OVERLAPPEDWINDOW, CW_USEDEFAULT, CW_USEDEFAULT, rc.right - rc.left, rc.bottom - rc.top, std::ptr::null_mut(), std::ptr::null_mut(), inst, app as *mut c_void);
        if hwnd.is_null() {
            drop(Box::from_raw(app));
            return Err("the window could not be made".into());
        }
        // a dark title bar that matches the picture (the colour is taken by Windows 11 only)
        let on: u32 = 1;
        DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &on as *const u32 as *const c_void, 4);
        let caption: u32 = 0x001E_1B1A;
        DwmSetWindowAttribute(hwnd, DWMWA_CAPTION_COLOR, &caption as *const u32 as *const c_void, 4);
        DragAcceptFiles(hwnd, 1);
        set_title(&*app);
        SetTimer(hwnd, 1, 15, std::ptr::null());
        ShowWindow(hwnd, SW_SHOW);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        drop(Box::from_raw(app));
        Ok(())
    }
}

unsafe fn set_title(app: &App) {
    let t = match &app.view.doc {
        Some(d) => format!("{} - {TITLE}", d.name),
        None => TITLE.to_string(),
    };
    SetWindowTextW(app.hwnd, wide(&t).as_ptr());
}

unsafe fn redraw(hwnd: HWND) {
    InvalidateRect(hwnd, std::ptr::null(), 0);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_CREATE {
        // CREATESTRUCT starts with the pointer given to CreateWindowEx
        let app = *(lp as *const *mut App);
        (*app).hwnd = hwnd;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, app as isize);
        return 0;
    }
    let app = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if app.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let app = &mut *app;
    let point = |lp: isize| ((lp & 0xFFFF) as i16 as f32, ((lp >> 16) & 0xFFFF) as i16 as f32);
    match msg {
        WM_SIZE => {
            let mut rc = RECT::default();
            GetClientRect(hwnd, &mut rc);
            app.surface.resize(rc.right, rc.bottom, GetDpiForWindow(hwnd).max(96) as f32 / 96.0);
            redraw(hwnd);
        }
        WM_DPICHANGED => {
            let r = &*(lp as *const RECT);
            SetWindowPos(hwnd, std::ptr::null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
        }
        WM_GETMINMAXINFO => {
            let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
            let m = &mut *(lp as *mut MINMAXINFO);
            m.ptMinTrackSize = POINT { x: (760.0 * scale) as i32, y: (600.0 * scale) as i32 };
        }
        WM_ERASEBKGND => return 1,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            app.drawn_at = app.view.position();
            app.view.draw(&mut app.surface);
            app.surface.show(dc);
            EndPaint(hwnd, &ps);
        }
        WM_TIMER => {
            if app.view.position() != app.drawn_at {
                redraw(hwnd);
            }
            if app.quit_after.is_some_and(|s| app.started.elapsed().as_secs_f64() >= s) {
                DestroyWindow(hwnd);
            }
        }
        WM_MOUSEMOVE => {
            let (x, y) = point(lp);
            app.view.mouse_move(x, y);
            redraw(hwnd);
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let (x, y) = point(lp);
            let req = app.view.mouse_down(x, y);
            if app.view.dragging() {
                SetCapture(hwnd);
            }
            serve(app, req);
            redraw(hwnd);
        }
        WM_LBUTTONUP => {
            app.view.mouse_up();
            ReleaseCapture();
            redraw(hwnd);
        }
        WM_MOUSEWHEEL => {
            app.view.wheel(((wp >> 16) & 0xFFFF) as i16 as f32 / 120.0, app.surface.scale);
            redraw(hwnd);
        }
        WM_KEYDOWN => {
            let req = app.view.key(wp, GetKeyState(VK_CONTROL) < 0);
            serve(app, req);
            redraw(hwnd);
        }
        WM_CHAR => {
            if GetKeyState(VK_CONTROL) >= 0 {
                if let Some(c) = char::from_u32(wp as u32) {
                    app.view.typed(c);
                    redraw(hwnd);
                }
            }
        }
        WM_SETCURSOR if (lp & 0xFFFF) as u16 == HTCLIENT => {
            let id = [IDC_ARROW, IDC_HAND, IDC_IBEAM][app.view.cursor() as usize];
            SetCursor(LoadCursorW(std::ptr::null_mut(), id as *const u16));
            return 1;
        }
        WM_DROPFILES => {
            let drop = wp as HANDLE;
            let mut buf = [0u16; 1024];
            let n = DragQueryFileW(drop, 0, buf.as_mut_ptr(), buf.len() as u32) as usize;
            DragFinish(drop);
            open(app, &String::from_utf16_lossy(&buf[..n]));
        }
        WM_DESTROY => {
            if app.quit_after.is_some() {
                let (pos, rate) = (app.view.position(), app.view.doc.as_ref().map_or(1, |d| d.rate));
                let device = app.view.player.as_ref().is_some_and(|p| p.has_device());
                eprintln!("position {} after {:.2} s, sound device: {}", clock(pos, rate), app.started.elapsed().as_secs_f64(), if device { "open" } else { "none" });
            }
            // the sound stops before the window is gone
            app.view.player = None;
            PostQuitMessage(0);
        }
        _ => return DefWindowProcW(hwnd, msg, wp, lp),
    }
    0
}

unsafe fn open(app: &mut App, path: &str) {
    app.view.load(path);
    set_title(app);
    redraw(app.hwnd);
}

/// A file dialog. `filter`: pairs of a name and a pattern. Returns the chosen path.
unsafe fn file_dialog(hwnd: HWND, save: bool, title: &str, filter: &[(&str, &str)], name: &str, ext: &str) -> Option<String> {
    let mut f: Vec<u16> = Vec::new();
    for (label, pattern) in filter {
        f.extend(label.encode_utf16().chain([0]));
        f.extend(pattern.encode_utf16().chain([0]));
    }
    f.push(0);
    let mut buf = [0u16; 1024];
    for (b, c) in buf.iter_mut().zip(name.encode_utf16().take(1000)) {
        *b = c;
    }
    let (title, ext) = (wide(title), wide(ext));
    let mut o: OPENFILENAMEW = std::mem::zeroed();
    o.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    o.hwndOwner = hwnd;
    o.lpstrFilter = f.as_ptr();
    o.lpstrFile = buf.as_mut_ptr();
    o.nMaxFile = buf.len() as u32;
    o.lpstrTitle = title.as_ptr();
    o.lpstrDefExt = ext.as_ptr();
    o.Flags = OFN_EXPLORER | OFN_PATHMUSTEXIST | if save { OFN_OVERWRITEPROMPT } else { OFN_FILEMUSTEXIST };
    let ok = if save { GetSaveFileNameW(&mut o) } else { GetOpenFileNameW(&mut o) };
    (ok != 0).then(|| String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(0)]))
}

/// A menu at a point of the window. `items`: (id, text, checked, greyed); id 0 is a separator. Returns the id picked.
unsafe fn menu(hwnd: HWND, x: f32, y: f32, items: &[(usize, String, bool, bool)]) -> usize {
    let m = CreatePopupMenu();
    for (id, text, checked, grey) in items {
        if *id == 0 {
            AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
        } else {
            AppendMenuW(m, MF_STRING | if *checked { MF_CHECKED } else { 0 } | if *grey { MF_GRAYED } else { 0 }, *id, wide(text).as_ptr());
        }
    }
    let mut p = POINT { x: x as i32, y: y as i32 };
    ClientToScreen(hwnd, &mut p);
    let picked = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTALIGN, p.x, p.y, 0, hwnd, std::ptr::null());
    DestroyMenu(m);
    picked as usize
}

unsafe fn copy_text(hwnd: HWND, text: &str) {
    let w = wide(text);
    if OpenClipboard(hwnd) == 0 {
        return;
    }
    EmptyClipboard();
    let mem = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2);
    if !mem.is_null() {
        let p = GlobalLock(mem) as *mut u16;
        if !p.is_null() {
            std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
            GlobalUnlock(mem);
            SetClipboardData(CF_UNICODETEXT, mem);
        }
    }
    CloseClipboard();
}

const M_OPEN: usize = 1;
const M_SAVE: usize = 2;
const M_LABELS: usize = 3;
const M_COPY: usize = 4;
const M_INFO: usize = 5;
const M_ALL: usize = 100;

/// Does what the view asked for.
unsafe fn serve(app: &mut App, req: Request) {
    let hwnd = app.hwnd;
    let has = app.view.doc.is_some();
    let item = |id: usize, text: &str, grey: bool| (id, text.to_string(), false, grey);
    let act = match req {
        Request::None => 0,
        Request::Open => M_OPEN,
        Request::Save => M_SAVE,
        Request::MoreMenu(x, y) => menu(
            hwnd,
            x,
            y,
            &[
                item(M_OPEN, "Open...\tCtrl+O", false),
                item(M_SAVE, "Save sound as WAV...\tCtrl+S", !has),
                item(M_LABELS, "Save labels as JSON...", !has),
                item(M_COPY, "Copy transcript", !has),
                item(0, "", false),
                (M_INFO, "File details\tCtrl+I".to_string(), app.view.details, false),
            ],
        ),
        Request::KebabMenu(x, y) => menu(hwnd, x, y, &[item(M_COPY, "Copy transcript", !has), item(M_LABELS, "Save labels as JSON...", !has)]),
        Request::FilterMenu(x, y) => {
            let tags = app.view.doc.as_ref().map_or(Vec::new(), |d| d.tags.clone());
            let mut items = vec![(M_ALL, "All rows".to_string(), app.view.tag_filter.is_none(), false)];
            if !tags.is_empty() {
                items.push(item(0, "", false));
            }
            for (i, t) in tags.iter().enumerate() {
                items.push((M_ALL + 1 + i, t.clone(), app.view.tag_filter.as_ref() == Some(t), false));
            }
            let picked = menu(hwnd, x, y, &items);
            if picked >= M_ALL {
                app.view.tag_filter = tags.get(picked.wrapping_sub(M_ALL + 1)).cloned();
            }
            0
        }
    };
    let stem = app.view.doc.as_ref().map_or(String::new(), |d| d.name.rsplit_once('.').map_or(d.name.clone(), |n| n.0.to_string()));
    match act {
        M_OPEN => {
            if let Some(p) = file_dialog(hwnd, false, "Open", &[("Voice audio (BAC, WAV, FLAC)", "*.bac;*.wav;*.flac"), ("All files", "*.*")], "", "bac") {
                open(app, &p);
            }
        }
        M_SAVE if has => {
            if let Some(p) = file_dialog(hwnd, true, "Save sound as WAV", &[("WAV", "*.wav")], &format!("{stem}.wav"), "wav") {
                let src = app.view.doc.as_ref().map(|d| d.path.clone()).unwrap_or_default();
                let done = crate::doc::read_pcm(&src).and_then(|pcm| std::fs::write(&p, bac::pcm::write_wav(&pcm)).map_err(|e| format!("{p}: {e}")));
                app.view.error = done.err();
            }
        }
        M_LABELS if has => {
            if let Some(p) = file_dialog(hwnd, true, "Save labels as JSON", &[("JSON", "*.json")], &format!("{stem}.json"), "json") {
                let json = app.view.doc.as_ref().map(|d| d.labels.to_json(d.rate)).unwrap_or_default();
                app.view.error = std::fs::write(&p, json).map_err(|e| format!("{p}: {e}")).err();
            }
        }
        M_COPY if has => copy_text(hwnd, &app.view.transcript_text()),
        M_INFO => app.view.details = !app.view.details,
        _ => {}
    }
    if app.view.error.is_some() && matches!(act, M_SAVE | M_LABELS) {
        MessageBoxW(hwnd, wide(app.view.error.as_deref().unwrap_or("")).as_ptr(), wide(TITLE).as_ptr(), MB_ICONERROR);
    }
}

/// The program's icon (`icon.rs`) for the window's corner and the task bar.
unsafe fn app_icon(size: i32) -> HANDLE {
    let argb = crate::icon::render(size as u32);
    let mask = vec![0u8; (size * size) as usize / 8 + size as usize];
    let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: CreateBitmap(size, size, 1, 1, mask.as_ptr() as *const c_void), hbmColor: CreateBitmap(size, size, 1, 32, argb.as_ptr() as *const c_void) };
    let icon = CreateIconIndirect(&info);
    DeleteObject(info.hbmMask);
    DeleteObject(info.hbmColor);
    icon
}
