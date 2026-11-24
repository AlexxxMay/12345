//! Контекст OpenGL на macOS (CGL, внеэкранный) — `OpenGL.framework`
//! загружается через `dlopen`, окно для контекста не требуется.
//!
//! OpenGL в macOS объявлен устаревшим, но поддерживается в macOS 12–15 как на
//! Intel, так и на Apple Silicon (через слой Metal). Используется профиль Legacy
//! (OpenGL 2.1 + GLSL 1.20 + FBO).

use super::GlContext;
use std::ffi::{c_char, c_void};

extern "C" {
    fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

type CglChoosePixelFormat = unsafe extern "C" fn(*const i32, *mut *mut c_void, *mut i32) -> i32;
type CglCreateContext = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> i32;
type CglSetCurrentContext = unsafe extern "C" fn(*mut c_void) -> i32;
type CglDestroyPixelFormat = unsafe extern "C" fn(*mut c_void) -> i32;
type CglDestroyContext = unsafe extern "C" fn(*mut c_void) -> i32;

const KCGL_PFA_ACCELERATED: i32 = 73;
const KCGL_PFA_COLOR_SIZE: i32 = 8;
const KCGL_PFA_ALPHA_SIZE: i32 = 11;
const KCGL_PFA_DEPTH_SIZE: i32 = 12;
const KCGL_PFA_ALLOW_OFFLINE_RENDERERS: i32 = 96;
const KCGL_PFA_OPENGL_PROFILE: i32 = 99;
const KCGL_OGLP_VERSION_LEGACY: i32 = 0x1000;
const KCGL_PFA_RENDERER_ID: i32 = 70;
const KCGL_RENDERER_GENERIC_FLOAT_ID: i32 = 0x0002_0400;

pub struct CglContext {
    lib: *mut c_void,
    ctx: *mut c_void,
    set_current: CglSetCurrentContext,
    destroy: CglDestroyContext,
}

impl CglContext {
    pub fn create() -> Result<CglContext, String> {
        unsafe {
            let lib = dlopen(c"/System/Library/Frameworks/OpenGL.framework/OpenGL".as_ptr(), 2 /* RTLD_NOW */);
            if lib.is_null() {
                return Err("OpenGL.framework недоступен".into());
            }
            let sym = |n: &std::ffi::CStr| dlsym(lib, n.as_ptr());
            let (ch, cr, sc, dp, dc) = (sym(c"CGLChoosePixelFormat"), sym(c"CGLCreateContext"), sym(c"CGLSetCurrentContext"), sym(c"CGLDestroyPixelFormat"), sym(c"CGLDestroyContext"));
            if ch.is_null() || cr.is_null() || sc.is_null() || dp.is_null() || dc.is_null() {
                return Err("OpenGL.framework: нет функций CGL".into());
            }
            let choose: CglChoosePixelFormat = std::mem::transmute(ch);
            let create: CglCreateContext = std::mem::transmute(cr);
            let set_current: CglSetCurrentContext = std::mem::transmute(sc);
            let destroy_pf: CglDestroyPixelFormat = std::mem::transmute(dp);
            let destroy: CglDestroyContext = std::mem::transmute(dc);

            let accelerated = [
                KCGL_PFA_ACCELERATED,
                KCGL_PFA_ALLOW_OFFLINE_RENDERERS,
                KCGL_PFA_OPENGL_PROFILE,
                KCGL_OGLP_VERSION_LEGACY,
                KCGL_PFA_COLOR_SIZE,
                24,
                KCGL_PFA_ALPHA_SIZE,
                8,
                KCGL_PFA_DEPTH_SIZE,
                24,
                0,
            ];
            // Программный растеризатор Apple — только по явному разрешению
            // (переменная ALPHACAD_GL_SOFTWARE=1, для проверки на машинах без GPU).
            let software = [
                KCGL_PFA_RENDERER_ID,
                KCGL_RENDERER_GENERIC_FLOAT_ID,
                KCGL_PFA_OPENGL_PROFILE,
                KCGL_OGLP_VERSION_LEGACY,
                KCGL_PFA_COLOR_SIZE,
                24,
                KCGL_PFA_ALPHA_SIZE,
                8,
                KCGL_PFA_DEPTH_SIZE,
                24,
                0,
            ];
            let allow_sw = std::env::var("ALPHACAD_GL_SOFTWARE").map(|v| v == "1").unwrap_or(false);
            let mut pf: *mut c_void = std::ptr::null_mut();
            let mut n = 0;
            let mut err = choose(accelerated.as_ptr(), &mut pf, &mut n);
            if (err != 0 || pf.is_null()) && allow_sw {
                err = choose(software.as_ptr(), &mut pf, &mut n);
            }
            if err != 0 || pf.is_null() {
                return Err(format!("нет аппаратного OpenGL (CGL {err})"));
            }
            let mut ctx: *mut c_void = std::ptr::null_mut();
            let err = create(pf, std::ptr::null_mut(), &mut ctx);
            destroy_pf(pf);
            if err != 0 || ctx.is_null() {
                return Err(format!("CGLCreateContext: {err}"));
            }
            Ok(CglContext { lib, ctx, set_current, destroy })
        }
    }
}

impl GlContext for CglContext {
    fn make_current(&self) -> bool {
        unsafe { (self.set_current)(self.ctx) == 0 }
    }

    fn get_proc(&self, name: &[u8]) -> *const c_void {
        unsafe { dlsym(self.lib, name.as_ptr() as *const c_char) as *const c_void }
    }
}

impl Drop for CglContext {
    fn drop(&mut self) {
        unsafe {
            (self.set_current)(std::ptr::null_mut());
            (self.destroy)(self.ctx);
        }
    }
}
