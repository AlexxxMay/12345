//! Windows: окно и ввод через Win32 (user32/gdi32/comdlg32/shell32/kernel32).
//!
//! Кадр выводится через `SetDIBitsToDevice` (формат пикселя 0x00RRGGBB совпадает
//! с 32-битным DIB). Масштабирование под DPI — Per-Monitor V2 (манифест + API).
#![cfg_attr(not(target_os = "windows"), allow(dead_code, unused_imports))]
#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]

use super::{Event, Filter, FrameOutput, Handler};
use crate::ui::{Cursor, Key, Mods, MouseButton};
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::{Duration, Instant};

type HWND = *mut c_void;
type HINSTANCE = *mut c_void;
type HICON = *mut c_void;
type HCURSOR = *mut c_void;
type HBRUSH = *mut c_void;
type HDC = *mut c_void;
type HMENU = *mut c_void;
type HDROP = *mut c_void;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type BOOL = i32;
type WNDPROC = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

#[repr(C)]
struct WNDCLASSEXW {
    cbSize: u32,
    style: u32,
    lpfnWndProc: WNDPROC,
    cbClsExtra: i32,
    cbWndExtra: i32,
    hInstance: HINSTANCE,
    hIcon: HICON,
    hCursor: HCURSOR,
    hbrBackground: HBRUSH,
    lpszMenuName: *const u16,
    lpszClassName: *const u16,
    hIconSm: HICON,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct MSG {
    hwnd: HWND,
    message: u32,
    wParam: WPARAM,
    lParam: LPARAM,
    time: u32,
    pt: POINT,
    lPrivate: u32,
}

#[repr(C)]
struct PAINTSTRUCT {
    hdc: HDC,
    fErase: BOOL,
    rcPaint: RECT,
    fRestore: BOOL,
    fIncUpdate: BOOL,
    rgbReserved: [u8; 32],
}

#[repr(C)]
struct BITMAPINFOHEADER {
    biSize: u32,
    biWidth: i32,
    biHeight: i32,
    biPlanes: u16,
    biBitCount: u16,
    biCompression: u32,
    biSizeImage: u32,
    biXPelsPerMeter: i32,
    biYPelsPerMeter: i32,
    biClrUsed: u32,
    biClrImportant: u32,
}

#[repr(C)]
struct BITMAPINFO {
    bmiHeader: BITMAPINFOHEADER,
    bmiColors: [u32; 1],
}

#[repr(C)]
struct TRACKMOUSEEVENT {
    cbSize: u32,
    dwFlags: u32,
    hwndTrack: HWND,
    dwHoverTime: u32,
}

#[repr(C)]
struct MINMAXINFO {
    ptReserved: POINT,
    ptMaxSize: POINT,
    ptMaxPosition: POINT,
    ptMinTrackSize: POINT,
    ptMaxTrackSize: POINT,
}

#[repr(C)]
struct OPENFILENAMEW {
    lStructSize: u32,
    hwndOwner: HWND,
    hInstance: HINSTANCE,
    lpstrFilter: *const u16,
    lpstrCustomFilter: *mut u16,
    nMaxCustFilter: u32,
    nFilterIndex: u32,
    lpstrFile: *mut u16,
    nMaxFile: u32,
    lpstrFileTitle: *mut u16,
    nMaxFileTitle: u32,
    lpstrInitialDir: *const u16,
    lpstrTitle: *const u16,
    Flags: u32,
    nFileOffset: u16,
    nFileExtension: u16,
    lpstrDefExt: *const u16,
    lCustData: LPARAM,
    lpfnHook: *const c_void,
    lpTemplateName: *const u16,
    pvReserved: *mut c_void,
    dwReserved: u32,
    FlagsEx: u32,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassExW(wc: *const WNDCLASSEXW) -> u16;
    fn CreateWindowExW(ex: u32, class: *const u16, title: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: HWND, menu: HMENU, inst: HINSTANCE, param: *mut c_void) -> HWND;
    fn DefWindowProcW(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn ShowWindow(h: HWND, cmd: i32) -> BOOL;
    fn UpdateWindow(h: HWND) -> BOOL;
    fn DestroyWindow(h: HWND) -> BOOL;
    fn PeekMessageW(msg: *mut MSG, h: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const c_void, wait_all: BOOL, ms: u32, mask: u32) -> u32;
    fn PostQuitMessage(code: i32);
    fn GetClientRect(h: HWND, r: *mut RECT) -> BOOL;
    fn GetDC(h: HWND) -> HDC;
    fn ReleaseDC(h: HWND, dc: HDC) -> i32;
    fn BeginPaint(h: HWND, ps: *mut PAINTSTRUCT) -> HDC;
    fn EndPaint(h: HWND, ps: *const PAINTSTRUCT) -> BOOL;
    fn LoadCursorW(inst: HINSTANCE, name: *const u16) -> HCURSOR;
    fn LoadIconW(inst: HINSTANCE, name: *const u16) -> HICON;
    fn SetCursor(c: HCURSOR) -> HCURSOR;
    fn SetCapture(h: HWND) -> HWND;
    fn ReleaseCapture() -> BOOL;
    fn TrackMouseEvent(t: *mut TRACKMOUSEEVENT) -> BOOL;
    fn GetKeyState(vk: i32) -> i16;
    fn SetWindowTextW(h: HWND, s: *const u16) -> BOOL;
    fn MessageBoxW(h: HWND, text: *const u16, caption: *const u16, t: u32) -> i32;
    fn GetSystemMetrics(i: i32) -> i32;
    fn SetWindowPos(h: HWND, after: HWND, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> BOOL;
    fn AdjustWindowRectEx(r: *mut RECT, style: u32, menu: BOOL, ex: u32) -> BOOL;
    fn InvalidateRect(h: HWND, r: *const RECT, erase: BOOL) -> BOOL;
    fn SetProcessDPIAware() -> BOOL;
}

#[link(name = "gdi32")]
extern "system" {
    fn SetDIBitsToDevice(dc: HDC, xd: i32, yd: i32, w: u32, h: u32, xs: i32, ys: i32, start: u32, lines: u32, bits: *const c_void, bmi: *const BITMAPINFO, usage: u32) -> i32;
    fn GetDeviceCaps(dc: HDC, index: i32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn AttachConsole(pid: u32) -> BOOL;
    fn GetUserDefaultUILanguage() -> u16;
}

#[link(name = "shell32")]
extern "system" {
    fn DragAcceptFiles(h: HWND, accept: BOOL);
    fn DragQueryFileW(d: HDROP, i: u32, buf: *mut u16, n: u32) -> u32;
    fn DragFinish(d: HDROP);
}

#[link(name = "comdlg32")]
extern "system" {
    fn GetOpenFileNameW(ofn: *mut OPENFILENAMEW) -> BOOL;
    fn GetSaveFileNameW(ofn: *mut OPENFILENAMEW) -> BOOL;
}

const WM_DESTROY: u32 = 0x0002;
const WM_SIZE: u32 = 0x0005;
const WM_PAINT: u32 = 0x000F;
const WM_CLOSE: u32 = 0x0010;
const WM_QUIT: u32 = 0x0012;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_SETCURSOR: u32 = 0x0020;
const WM_GETMINMAXINFO: u32 = 0x0024;
const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_CHAR: u32 = 0x0102;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_SYSKEYUP: u32 = 0x0105;
const WM_SYSCHAR: u32 = 0x0106;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_LBUTTONDBLCLK: u32 = 0x0203;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_MBUTTONDOWN: u32 = 0x0207;
const WM_MBUTTONUP: u32 = 0x0208;
const WM_MOUSEWHEEL: u32 = 0x020A;
const WM_DROPFILES: u32 = 0x0233;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_DPICHANGED: u32 = 0x02E0;
const WM_KILLFOCUS: u32 = 0x0008;

const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
const WS_EX_ACCEPTFILES: u32 = 0x0000_0010;
const CS_HREDRAW: u32 = 0x0002;
const CS_VREDRAW: u32 = 0x0001;
const CS_OWNDC: u32 = 0x0020;
const SW_SHOW: i32 = 5;
const SW_SHOWMAXIMIZED: i32 = 3;
const PM_REMOVE: u32 = 0x0001;
const QS_ALLINPUT: u32 = 0x04FF;
const INFINITE: u32 = 0xFFFF_FFFF;
const HTCLIENT: isize = 1;
const TME_LEAVE: u32 = 0x0002;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;
const LOGPIXELSX: i32 = 88;

const IDC_ARROW: usize = 32512;
const IDC_IBEAM: usize = 32513;
const IDC_CROSS: usize = 32515;
const IDC_SIZEALL: usize = 32646;
const IDC_HAND: usize = 32649;
const IDI_APPLICATION: usize = 32512;

const OFN_OVERWRITEPROMPT: u32 = 0x0000_0002;
const OFN_HIDEREADONLY: u32 = 0x0000_0004;
const OFN_NOCHANGEDIR: u32 = 0x0000_0008;
const OFN_ALLOWMULTISELECT: u32 = 0x0000_0200;
const OFN_PATHMUSTEXIST: u32 = 0x0000_0800;
const OFN_FILEMUSTEXIST: u32 = 0x0000_1000;
const OFN_EXPLORER: u32 = 0x0008_0000;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);

fn main_hwnd() -> HWND {
    MAIN_HWND.load(Ordering::Relaxed) as HWND
}

/// Копия последнего кадра для перерисовки, пока приложение занято (модальные диалоги).
struct Present {
    pixels: Vec<u32>,
    w: usize,
    h: usize,
}

thread_local! {
    static APP: RefCell<Option<Box<dyn Handler>>> = RefCell::new(None);
    static EVENTS: RefCell<Vec<Event>> = RefCell::new(Vec::new());
    static PRESENT: RefCell<Present> = RefCell::new(Present { pixels: Vec::new(), w: 0, h: 0 });
    static CURSOR: Cell<Cursor> = Cell::new(Cursor::Arrow);
    static TRACKING: Cell<bool> = Cell::new(false);
    static BUTTONS: Cell<u8> = Cell::new(0);
    static HIGH_SURROGATE: Cell<u16> = Cell::new(0);
    static TITLE: RefCell<String> = RefCell::new(String::new());
    static WAKE: Cell<Option<Instant>> = Cell::new(None);
    static DIRTY: Cell<bool> = Cell::new(true);
    static QUIT: Cell<bool> = Cell::new(false);
}

fn push(e: Event) {
    EVENTS.with(|q| q.borrow_mut().push(e));
    DIRTY.with(|d| d.set(true));
}

type GetDpiForWindowFn = unsafe extern "system" fn(HWND) -> u32;
type SetDpiCtxFn = unsafe extern "system" fn(isize) -> BOOL;

fn user32_proc(name: &str) -> *mut c_void {
    unsafe {
        let lib = LoadLibraryW(wide("user32.dll").as_ptr());
        if lib.is_null() {
            return null_mut();
        }
        let mut n = name.as_bytes().to_vec();
        n.push(0);
        GetProcAddress(lib, n.as_ptr())
    }
}

fn enable_dpi_awareness() {
    unsafe {
        let p = user32_proc("SetProcessDpiAwarenessContext");
        if !p.is_null() {
            let f: SetDpiCtxFn = std::mem::transmute(p);
            // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 = -4
            if f(-4) != 0 {
                return;
            }
        }
        SetProcessDPIAware();
    }
}

fn window_scale(hwnd: HWND) -> f32 {
    unsafe {
        let p = user32_proc("GetDpiForWindow");
        if !p.is_null() && !hwnd.is_null() {
            let f: GetDpiForWindowFn = std::mem::transmute(p);
            let d = f(hwnd);
            if d > 0 {
                return d as f32 / 96.0;
            }
        }
        let dc = GetDC(null_mut());
        let d = GetDeviceCaps(dc, LOGPIXELSX);
        ReleaseDC(null_mut(), dc);
        if d > 0 {
            d as f32 / 96.0
        } else {
            1.0
        }
    }
}

fn mods() -> Mods {
    unsafe {
        let down = |vk: i32| (GetKeyState(vk) as u16 & 0x8000) != 0;
        Mods { ctrl: down(0x11), shift: down(0x10), alt: down(0x12), meta: down(0x5B) || down(0x5C) }
    }
}

fn map_vk(vk: usize) -> Option<Key> {
    Some(match vk {
        0x1B => Key::Escape,
        0x0D => Key::Enter,
        0x09 => Key::Tab,
        0x08 => Key::Backspace,
        0x2E => Key::Delete,
        0x25 => Key::Left,
        0x27 => Key::Right,
        0x26 => Key::Up,
        0x28 => Key::Down,
        0x24 => Key::Home,
        0x23 => Key::End,
        0x21 => Key::PageUp,
        0x22 => Key::PageDown,
        0x20 => Key::Space,
        0x70..=0x7B => Key::F((vk - 0x70 + 1) as u8),
        0x30..=0x39 => Key::Char((b'0' + (vk - 0x30) as u8) as char),
        0x41..=0x5A => Key::Char((b'a' + (vk - 0x41) as u8) as char),
        0x60..=0x69 => Key::Char((b'0' + (vk - 0x60) as u8) as char),
        0xBB | 0x6B => Key::Char('+'),
        0xBD | 0x6D => Key::Char('-'),
        _ => return None,
    })
}

fn lparam_xy(l: LPARAM) -> (f32, f32) {
    let x = (l & 0xFFFF) as u16 as i16 as f32;
    let y = ((l >> 16) & 0xFFFF) as u16 as i16 as f32;
    (x, y)
}

fn present(hwnd: HWND, dc: Option<HDC>) {
    PRESENT.with(|p| {
        let p = p.borrow();
        if p.w == 0 || p.h == 0 || p.pixels.len() < p.w * p.h {
            return;
        }
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: p.w as i32,
                biHeight: -(p.h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [0],
        };
        unsafe {
            let (dc, release) = match dc {
                Some(d) => (d, false),
                None => (GetDC(hwnd), true),
            };
            SetDIBitsToDevice(dc, 0, 0, p.w as u32, p.h as u32, 0, 0, 0, p.h as u32, p.pixels.as_ptr() as *const c_void, &bmi, 0);
            if release {
                ReleaseDC(hwnd, dc);
            }
        }
    });
}

/// Передать накопленные события приложению и построить кадр. `false` — приложение занято.
fn pump_frame(hwnd: HWND) -> bool {
    APP.with(|cell| {
        let Ok(mut guard) = cell.try_borrow_mut() else { return false };
        let Some(app) = guard.as_mut() else { return false };
        let events: Vec<Event> = EVENTS.with(|q| std::mem::take(&mut *q.borrow_mut()));
        for e in events {
            app.event(e);
        }
        DIRTY.with(|d| d.set(false));
        let out: FrameOutput = app.frame();
        // События, пришедшие во время кадра (например, из модального диалога), — в следующий кадр.
        if EVENTS.with(|q| !q.borrow().is_empty()) {
            DIRTY.with(|d| d.set(true));
        }
        let fb = app.framebuffer();
        PRESENT.with(|p| {
            let mut p = p.borrow_mut();
            p.w = fb.width;
            p.h = fb.height;
            p.pixels.clear();
            p.pixels.extend_from_slice(&fb.color);
        });
        drop(guard);
        present(hwnd, None);
        TITLE.with(|t| {
            let mut t = t.borrow_mut();
            if *t != out.title {
                *t = out.title.clone();
                unsafe { SetWindowTextW(hwnd, wide(&out.title).as_ptr()) };
            }
        });
        if CURSOR.with(|c| c.get()) != out.cursor {
            CURSOR.with(|c| c.set(out.cursor));
            apply_cursor(out.cursor);
        }
        WAKE.with(|w| w.set(out.wake_ms.map(|ms| Instant::now() + Duration::from_millis(ms))));
        if out.quit {
            QUIT.with(|q| q.set(true));
        }
        true
    })
}

fn apply_cursor(c: Cursor) {
    let id = match c {
        Cursor::Arrow => IDC_ARROW,
        Cursor::Hand => IDC_HAND,
        Cursor::Text => IDC_IBEAM,
        Cursor::Crosshair => IDC_CROSS,
        Cursor::Move => IDC_SIZEALL,
    };
    unsafe {
        SetCursor(LoadCursorW(null_mut(), id as *const u16));
    }
}

fn send_resize(hwnd: HWND) {
    let mut r = RECT::default();
    unsafe { GetClientRect(hwnd, &mut r) };
    let w = (r.right - r.left).max(1) as usize;
    let h = (r.bottom - r.top).max(1) as usize;
    push(Event::Resize { width: w, height: h, scale: window_scale(hwnd) });
}

fn button_event(hwnd: HWND, b: MouseButton, down: bool, l: LPARAM) {
    let (x, y) = lparam_xy(l);
    push(Event::MouseMove { x, y });
    push(Event::MouseButton { button: b, down });
    BUTTONS.with(|bs| {
        let bit = 1u8 << (b as u8);
        let v = if down { bs.get() | bit } else { bs.get() & !bit };
        bs.set(v);
        unsafe {
            if down && v == bit {
                SetCapture(hwnd);
            } else if v == 0 {
                ReleaseCapture();
            }
        }
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CLOSE => {
            push(Event::CloseRequested);
            pump_frame(hwnd);
            return 0;
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            return 0;
        }
        WM_ERASEBKGND => return 1,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            present(hwnd, Some(dc));
            EndPaint(hwnd, &ps);
            return 0;
        }
        WM_SIZE => {
            send_resize(hwnd);
            // Живое изменение размера (во время модального цикла перетаскивания рамки).
            pump_frame(hwnd);
            return 0;
        }
        WM_DPICHANGED => {
            let r = &*(lp as *const RECT);
            SetWindowPos(hwnd, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            send_resize(hwnd);
            return 0;
        }
        WM_GETMINMAXINFO => {
            let mmi = &mut *(lp as *mut MINMAXINFO);
            let s = window_scale(hwnd);
            mmi.ptMinTrackSize = POINT { x: (720.0 * s) as i32, y: (480.0 * s) as i32 };
            return 0;
        }
        WM_SETCURSOR => {
            if (lp & 0xFFFF) as isize == HTCLIENT {
                apply_cursor(CURSOR.with(|c| c.get()));
                return 1;
            }
        }
        WM_MOUSEMOVE => {
            if !TRACKING.with(|t| t.get()) {
                let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                TrackMouseEvent(&mut tme);
                TRACKING.with(|t| t.set(true));
            }
            let (x, y) = lparam_xy(lp);
            push(Event::MouseMove { x, y });
            return 0;
        }
        WM_MOUSELEAVE => {
            TRACKING.with(|t| t.set(false));
            if BUTTONS.with(|b| b.get()) == 0 {
                push(Event::MouseLeave);
            }
            return 0;
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            button_event(hwnd, MouseButton::Left, true, lp);
            return 0;
        }
        WM_LBUTTONUP => {
            button_event(hwnd, MouseButton::Left, false, lp);
            return 0;
        }
        WM_RBUTTONDOWN => {
            button_event(hwnd, MouseButton::Right, true, lp);
            return 0;
        }
        WM_RBUTTONUP => {
            button_event(hwnd, MouseButton::Right, false, lp);
            return 0;
        }
        WM_MBUTTONDOWN => {
            button_event(hwnd, MouseButton::Middle, true, lp);
            return 0;
        }
        WM_MBUTTONUP => {
            button_event(hwnd, MouseButton::Middle, false, lp);
            return 0;
        }
        WM_MOUSEWHEEL => {
            let delta = ((wp >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
            push(Event::Modifiers(mods()));
            push(Event::Wheel { delta });
            return 0;
        }
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let m = mods();
            if let Some(k) = map_vk(wp) {
                push(Event::Key { key: k, mods: m, down });
            } else {
                push(Event::Modifiers(m));
            }
            // F10 и одиночный Alt не должны активировать системное меню.
            if wp == 0x79 || wp == 0x12 {
                return 0;
            }
            if msg == WM_SYSKEYDOWN || msg == WM_SYSKEYUP {
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
            return 0;
        }
        WM_CHAR | WM_SYSCHAR => {
            let c = wp as u16;
            if (0xD800..0xDC00).contains(&c) {
                HIGH_SURROGATE.with(|h| h.set(c));
                return 0;
            }
            let s = if (0xDC00..0xE000).contains(&c) {
                let hi = HIGH_SURROGATE.with(|h| h.replace(0));
                String::from_utf16_lossy(&[hi, c])
            } else {
                String::from_utf16_lossy(&[c])
            };
            if s.chars().all(|ch| !ch.is_control()) && msg == WM_CHAR {
                push(Event::Text(s));
            }
            return 0;
        }
        WM_KILLFOCUS => {
            push(Event::Modifiers(Mods::default()));
        }
        WM_DROPFILES => {
            let drop = wp as HDROP;
            let n = DragQueryFileW(drop, 0xFFFF_FFFF, null_mut(), 0);
            let mut files = Vec::new();
            for i in 0..n {
                let len = DragQueryFileW(drop, i, null_mut(), 0);
                let mut buf = vec![0u16; len as usize + 1];
                DragQueryFileW(drop, i, buf.as_mut_ptr(), buf.len() as u32);
                files.push(PathBuf::from(from_wide(&buf)));
            }
            DragFinish(drop);
            if !files.is_empty() {
                push(Event::DropFiles(files));
            }
            return 0;
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

/// Главный цикл окна.
pub fn run(handler: Box<dyn Handler>, title: &str, width: u32, height: u32) -> Result<(), String> {
    enable_dpi_awareness();
    unsafe {
        let hinst = GetModuleHandleW(null());
        let class = wide("AlphaCadWindow");
        let mut icon = LoadIconW(hinst, 1 as *const u16);
        if icon.is_null() {
            icon = LoadIconW(null_mut(), IDI_APPLICATION as *const u16);
        }
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_OWNDC,
            lpfnWndProc: wndproc,
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: icon,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW as *const u16),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
            hIconSm: icon,
        };
        if RegisterClassExW(&wc) == 0 {
            return Err("RegisterClassExW failed".into());
        }
        let s = window_scale(null_mut());
        let sw = GetSystemMetrics(0);
        let sh = GetSystemMetrics(1);
        let mut cw = (width as f32 * s) as i32;
        let mut ch = (height as f32 * s) as i32;
        let maximize = cw > sw * 9 / 10 || ch > sh * 9 / 10;
        cw = cw.min(sw * 9 / 10);
        ch = ch.min(sh * 9 / 10);
        let mut r = RECT { left: 0, top: 0, right: cw, bottom: ch };
        AdjustWindowRectEx(&mut r, WS_OVERLAPPEDWINDOW, 0, WS_EX_ACCEPTFILES);
        let (ww, wh) = (r.right - r.left, r.bottom - r.top);
        let x = ((sw - ww) / 2).max(0);
        let y = ((sh - wh) / 2).max(0);

        APP.with(|a| *a.borrow_mut() = Some(handler));
        let hwnd = CreateWindowExW(WS_EX_ACCEPTFILES, class.as_ptr(), wide(title).as_ptr(), WS_OVERLAPPEDWINDOW, x, y, ww, wh, null_mut(), null_mut(), hinst, null_mut());
        if hwnd.is_null() {
            return Err("CreateWindowExW failed".into());
        }
        MAIN_HWND.store(hwnd as isize, Ordering::Relaxed);
        DragAcceptFiles(hwnd, 1);
        send_resize(hwnd);
        pump_frame(hwnd);
        ShowWindow(hwnd, if maximize { SW_SHOWMAXIMIZED } else { SW_SHOW });
        UpdateWindow(hwnd);

        let mut msg: MSG = std::mem::zeroed();
        'main: loop {
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                if msg.message == WM_QUIT {
                    break 'main;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let wake_due = WAKE.with(|w| w.get()).map(|t| Instant::now() >= t).unwrap_or(false);
            if DIRTY.with(|d| d.get()) || wake_due {
                if wake_due {
                    WAKE.with(|w| w.set(None));
                }
                pump_frame(hwnd);
            }
            if QUIT.with(|q| q.get()) {
                DestroyWindow(hwnd);
                QUIT.with(|q| q.set(false));
                continue;
            }
            if DIRTY.with(|d| d.get()) {
                continue;
            }
            let timeout = match WAKE.with(|w| w.get()) {
                Some(t) => t.saturating_duration_since(Instant::now()).as_millis().min(u32::MAX as u128 - 1) as u32,
                None => INFINITE,
            };
            MsgWaitForMultipleObjects(0, null(), 0, timeout, QS_ALLINPUT);
        }
        APP.with(|a| *a.borrow_mut() = None);
        let _ = InvalidateRect;
    }
    Ok(())
}

/// Строка фильтра: "Название\0*.a;*.b\0…\0\0".
fn filter_string(filters: &[Filter]) -> Vec<u16> {
    let mut s = String::new();
    for (name, exts) in filters {
        let pat: Vec<String> = exts.iter().map(|e| format!("*.{e}")).collect();
        let pat = pat.join(";");
        s.push_str(&format!("{name} ({pat})\0{pat}\0"));
    }
    s.push_str("*.*\0*.*\0\0");
    s.encode_utf16().collect()
}

pub fn open_file_dialog(title: &str, filters: &[Filter], multiple: bool) -> Vec<PathBuf> {
    let filter = filter_string(filters);
    let title_w = wide(title);
    let mut buf = vec![0u16; 32 * 1024];
    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = main_hwnd();
    ofn.lpstrFilter = filter.as_ptr();
    ofn.nFilterIndex = 1;
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrTitle = title_w.as_ptr();
    ofn.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR | if multiple { OFN_ALLOWMULTISELECT } else { 0 };
    if unsafe { GetOpenFileNameW(&mut ofn) } == 0 {
        return Vec::new();
    }
    // Разбор: "путь\0" или "каталог\0файл1\0файл2\0\0".
    let mut parts = Vec::new();
    let mut start = 0;
    for i in 0..buf.len() {
        if buf[i] == 0 {
            if i == start {
                break;
            }
            parts.push(String::from_utf16_lossy(&buf[start..i]));
            start = i + 1;
        }
    }
    if parts.len() <= 1 {
        return parts.into_iter().map(PathBuf::from).collect();
    }
    let dir = PathBuf::from(&parts[0]);
    parts[1..].iter().map(|f| dir.join(f)).collect()
}

pub fn save_file_dialog(title: &str, default_name: &str, filters: &[Filter]) -> Option<PathBuf> {
    let filter = filter_string(filters);
    let title_w = wide(title);
    let mut buf = vec![0u16; 4096];
    for (i, c) in default_name.encode_utf16().take(4000).enumerate() {
        buf[i] = c;
    }
    let def_ext = filters.first().and_then(|f| f.1.first()).map(|e| wide(e));
    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = main_hwnd();
    ofn.lpstrFilter = filter.as_ptr();
    ofn.nFilterIndex = 1;
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrTitle = title_w.as_ptr();
    ofn.lpstrDefExt = def_ext.as_ref().map(|v| v.as_ptr()).unwrap_or(null());
    ofn.Flags = OFN_EXPLORER | OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR;
    if unsafe { GetSaveFileNameW(&mut ofn) } == 0 {
        return None;
    }
    let s = from_wide(&buf);
    if s.is_empty() {
        None
    } else {
        Some(PathBuf::from(s))
    }
}

pub fn message_box(title: &str, text: &str) {
    unsafe {
        MessageBoxW(main_hwnd(), wide(text).as_ptr(), wide(title).as_ptr(), 0x10);
    }
}

pub fn user_lang_is_russian() -> bool {
    unsafe { (GetUserDefaultUILanguage() & 0x3FF) == 0x19 }
}

/// Подключение к консоли родительского процесса (для CLI в GUI-сборке).
pub fn attach_console() {
    unsafe {
        AttachConsole(0xFFFF_FFFF);
    }
}
