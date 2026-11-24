//! Контекст OpenGL на Windows (WGL) — скрытое окно + `opengl32.dll`,
//! загружаемая динамически (без зависимости при линковке).

#![allow(clashing_extern_declarations, non_camel_case_types, clippy::upper_case_acronyms)]

use super::GlContext;
use std::ffi::c_void;

type HWND = *mut c_void;
type HDC = *mut c_void;
type HGLRC = *mut c_void;
type HMODULE = *mut c_void;

#[repr(C)]
struct WNDCLASSW {
    style: u32,
    lpfn_wnd_proc: Option<unsafe extern "system" fn(HWND, u32, usize, isize) -> isize>,
    cb_cls_extra: i32,
    cb_wnd_extra: i32,
    h_instance: *mut c_void,
    h_icon: *mut c_void,
    h_cursor: *mut c_void,
    hbr_background: *mut c_void,
    lpsz_menu_name: *const u16,
    lpsz_class_name: *const u16,
}

#[repr(C)]
#[derive(Default)]
struct PIXELFORMATDESCRIPTOR {
    n_size: u16,
    n_version: u16,
    dw_flags: u32,
    i_pixel_type: u8,
    c_color_bits: u8,
    c_red_bits: u8,
    c_red_shift: u8,
    c_green_bits: u8,
    c_green_shift: u8,
    c_blue_bits: u8,
    c_blue_shift: u8,
    c_alpha_bits: u8,
    c_alpha_shift: u8,
    c_accum_bits: u8,
    c_accum_red_bits: u8,
    c_accum_green_bits: u8,
    c_accum_blue_bits: u8,
    c_accum_alpha_bits: u8,
    c_depth_bits: u8,
    c_stencil_bits: u8,
    c_aux_buffers: u8,
    i_layer_type: u8,
    b_reserved: u8,
    dw_layer_mask: u32,
    dw_visible_mask: u32,
    dw_damage_mask: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> HMODULE;
    fn GetProcAddress(module: HMODULE, name: *const u8) -> *mut c_void;
    fn GetModuleHandleW(name: *const u16) -> HMODULE;
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassW(wc: *const WNDCLASSW) -> u16;
    fn CreateWindowExW(ex: u32, class: *const u16, title: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: HWND, menu: *mut c_void, inst: *mut c_void, param: *mut c_void) -> HWND;
    fn DestroyWindow(hwnd: HWND) -> i32;
    fn GetDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, dc: HDC) -> i32;
    fn DefWindowProcW(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize;
}

#[link(name = "gdi32")]
extern "system" {
    fn ChoosePixelFormat(dc: HDC, pfd: *const PIXELFORMATDESCRIPTOR) -> i32;
    fn SetPixelFormat(dc: HDC, format: i32, pfd: *const PIXELFORMATDESCRIPTOR) -> i32;
}

type WglCreateContext = unsafe extern "system" fn(HDC) -> HGLRC;
type WglMakeCurrent = unsafe extern "system" fn(HDC, HGLRC) -> i32;
type WglDeleteContext = unsafe extern "system" fn(HGLRC) -> i32;
type WglGetProcAddress = unsafe extern "system" fn(*const u8) -> *const c_void;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    DefWindowProcW(hwnd, msg, wp, lp)
}

pub struct WglContext {
    hwnd: HWND,
    dc: HDC,
    rc: HGLRC,
    lib: HMODULE,
    make_current: WglMakeCurrent,
    delete_context: WglDeleteContext,
    get_proc: WglGetProcAddress,
}

impl WglContext {
    pub fn create() -> Result<WglContext, String> {
        unsafe {
            let lib = LoadLibraryW(wide("opengl32.dll").as_ptr());
            if lib.is_null() {
                return Err("opengl32.dll не найдена".into());
            }
            let sym = |n: &[u8]| GetProcAddress(lib, n.as_ptr());
            let (c, m, d, g) = (sym(b"wglCreateContext\0"), sym(b"wglMakeCurrent\0"), sym(b"wglDeleteContext\0"), sym(b"wglGetProcAddress\0"));
            if c.is_null() || m.is_null() || d.is_null() || g.is_null() {
                return Err("opengl32.dll: нет функций WGL".into());
            }
            let create: WglCreateContext = std::mem::transmute(c);
            let make_current: WglMakeCurrent = std::mem::transmute(m);
            let delete_context: WglDeleteContext = std::mem::transmute(d);
            let get_proc: WglGetProcAddress = std::mem::transmute(g);

            let inst = GetModuleHandleW(std::ptr::null());
            let class = wide("AlphaCadGL");
            let wc = WNDCLASSW {
                style: 0x0020, // CS_OWNDC
                lpfn_wnd_proc: Some(wndproc),
                cb_cls_extra: 0,
                cb_wnd_extra: 0,
                h_instance: inst,
                h_icon: std::ptr::null_mut(),
                h_cursor: std::ptr::null_mut(),
                hbr_background: std::ptr::null_mut(),
                lpsz_menu_name: std::ptr::null(),
                lpsz_class_name: class.as_ptr(),
            };
            // Повторная регистрация (второй рендерер) завершается ошибкой — это не страшно.
            RegisterClassW(&wc);
            let title = wide("AlphaCad GL");
            let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), 0x8000_0000 /* WS_POPUP */, 0, 0, 16, 16, std::ptr::null_mut(), std::ptr::null_mut(), inst, std::ptr::null_mut());
            if hwnd.is_null() {
                return Err("не удалось создать окно для OpenGL".into());
            }
            let dc = GetDC(hwnd);
            let pfd = PIXELFORMATDESCRIPTOR {
                n_size: std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as u16,
                n_version: 1,
                dw_flags: 0x04 | 0x20 | 0x01, // DRAW_TO_WINDOW | SUPPORT_OPENGL | DOUBLEBUFFER
                i_pixel_type: 0,
                c_color_bits: 32,
                c_alpha_bits: 8,
                c_depth_bits: 24,
                c_stencil_bits: 8,
                ..Default::default()
            };
            let fmt = ChoosePixelFormat(dc, &pfd);
            if fmt == 0 || SetPixelFormat(dc, fmt, &pfd) == 0 {
                ReleaseDC(hwnd, dc);
                DestroyWindow(hwnd);
                return Err("формат пикселей OpenGL не поддерживается".into());
            }
            let rc = create(dc);
            if rc.is_null() {
                ReleaseDC(hwnd, dc);
                DestroyWindow(hwnd);
                return Err("wglCreateContext: ошибка".into());
            }
            Ok(WglContext { hwnd, dc, rc, lib, make_current, delete_context, get_proc })
        }
    }
}

impl GlContext for WglContext {
    fn make_current(&self) -> bool {
        unsafe { (self.make_current)(self.dc, self.rc) != 0 }
    }

    fn get_proc(&self, name: &[u8]) -> *const c_void {
        unsafe {
            let p = (self.get_proc)(name.as_ptr());
            // wglGetProcAddress не отдаёт функции GL 1.1 и может вернуть 1/2/3/-1.
            let bad = p.is_null() || matches!(p as isize, 1 | 2 | 3 | -1);
            if bad {
                GetProcAddress(self.lib, name.as_ptr()) as *const c_void
            } else {
                p
            }
        }
    }
}

impl Drop for WglContext {
    fn drop(&mut self) {
        unsafe {
            (self.make_current)(std::ptr::null_mut(), std::ptr::null_mut());
            (self.delete_context)(self.rc);
            ReleaseDC(self.hwnd, self.dc);
            DestroyWindow(self.hwnd);
        }
    }
}
