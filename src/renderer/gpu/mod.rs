//! Аппаратный 3D-рендер через OpenGL 2.1 (видеокарта).
//!
//! Без внешних библиотек: функции OpenGL загружаются динамически
//! (Windows — `opengl32.dll` + WGL, macOS — `OpenGL.framework` + CGL).
//! Рендер идёт во внеэкранный буфер (FBO) с двумя цветовыми вложениями —
//! изображение и идентификаторы объектов, — после чего цвет, идентификаторы
//! и глубина считываются в [`Framebuffer`]. Благодаря этому интерфейс,
//! выбор мышью и «точка под курсором» работают одинаково с программным
//! рендером, а приложение может переключать CPU/GPU на лету.
//!
//! Внешний вид повторяет программный рендер: то же освещение (в вершинном
//! шейдере), те же цвета рёбер, сглаженные линии и смещение глубины линий.
//! Если видеокарта или драйвер недоступны, [`GpuRenderer::new`] возвращает
//! ошибку, и приложение остаётся на программном рендере.

#![allow(non_snake_case, clippy::missing_safety_doc)]

use super::canvas::Framebuffer;
use super::software::{self, Camera, DisplayMesh, Line3, RenderItem, RenderSettings, RenderStats};
use std::collections::HashMap;
use std::ffi::{c_char, c_void};

#[cfg(any(target_os = "windows", feature = "check-windows"))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod wgl;

#[cfg(any(target_os = "macos", feature = "check-macos"))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod cgl;

// ---------------------------------------------------------------------------
// Типы и константы OpenGL
// ---------------------------------------------------------------------------

type GLenum = u32;
type GLuint = u32;
type GLint = i32;
type GLsizei = i32;
type GLfloat = f32;
type GLboolean = u8;
type GLbitfield = u32;
type GLsizeiptr = isize;

const GL_NO_ERROR: GLenum = 0;
const GL_VENDOR: GLenum = 0x1F00;
const GL_RENDERER: GLenum = 0x1F01;
const GL_VERSION: GLenum = 0x1F02;
const GL_DEPTH_BUFFER_BIT: GLbitfield = 0x0100;
const GL_COLOR_BUFFER_BIT: GLbitfield = 0x4000;
const GL_DEPTH_TEST: GLenum = 0x0B71;
const GL_CULL_FACE: GLenum = 0x0B44;
const GL_BLEND: GLenum = 0x0BE2;
const GL_DITHER: GLenum = 0x0BD0;
const GL_SCISSOR_TEST: GLenum = 0x0C11;
const GL_LESS: GLenum = 0x0201;
const GL_LEQUAL: GLenum = 0x0203;
const GL_BACK: GLenum = 0x0405;
const GL_CCW: GLenum = 0x0901;
const GL_SRC_ALPHA: GLenum = 0x0302;
const GL_ONE_MINUS_SRC_ALPHA: GLenum = 0x0303;
const GL_TRIANGLES: GLenum = 0x0004;
const GL_TRIANGLE_STRIP: GLenum = 0x0005;
const GL_UNSIGNED_BYTE: GLenum = 0x1401;
const GL_UNSIGNED_INT: GLenum = 0x1405;
const GL_FLOAT: GLenum = 0x1406;
const GL_RGBA: GLenum = 0x1908;
const GL_BGRA: GLenum = 0x80E1;
const GL_DEPTH_COMPONENT: GLenum = 0x1902;
const GL_PACK_ALIGNMENT: GLenum = 0x0D05;
const GL_ARRAY_BUFFER: GLenum = 0x8892;
const GL_ELEMENT_ARRAY_BUFFER: GLenum = 0x8893;
const GL_STATIC_DRAW: GLenum = 0x88E4;
const GL_STREAM_DRAW: GLenum = 0x88E0;
const GL_FRAGMENT_SHADER: GLenum = 0x8B30;
const GL_VERTEX_SHADER: GLenum = 0x8B31;
const GL_COMPILE_STATUS: GLenum = 0x8B81;
const GL_LINK_STATUS: GLenum = 0x8B82;
const GL_FRAMEBUFFER: GLenum = 0x8D40;
const GL_RENDERBUFFER: GLenum = 0x8D41;
const GL_FRAMEBUFFER_COMPLETE: GLenum = 0x8CD5;
const GL_COLOR_ATTACHMENT0: GLenum = 0x8CE0;
const GL_COLOR_ATTACHMENT1: GLenum = 0x8CE1;
const GL_DEPTH_ATTACHMENT: GLenum = 0x8D00;
const GL_RGBA8: GLenum = 0x8058;
const GL_DEPTH_COMPONENT24: GLenum = 0x81A6;
const GL_MAX_RENDERBUFFER_SIZE: GLenum = 0x84E8;

/// Таблица функций OpenGL, загружаемых во время выполнения.
macro_rules! gl_api {
    ($( $name:ident ( $($arg:ty),* ) $(-> $ret:ty)? = $sym:literal $(| $alt:literal)* ;)*) => {
        pub(crate) struct Gl {
            $( $name: unsafe extern "system" fn($($arg),*) $(-> $ret)?, )*
        }
        impl Gl {
            /// `get` получает имя функции, оканчивающееся нулевым байтом.
            unsafe fn load(get: &dyn Fn(&[u8]) -> *const c_void) -> Result<Gl, String> {
                Ok(Gl {
                    $( $name: {
                        #[allow(unused_mut)]
                        let mut p = get(concat!($sym, "\0").as_bytes());
                        $( if p.is_null() { p = get(concat!($alt, "\0").as_bytes()); } )*
                        if p.is_null() {
                            return Err(format!("OpenGL: нет функции {}", $sym));
                        }
                        std::mem::transmute::<*const c_void, unsafe extern "system" fn($($arg),*) $(-> $ret)?>(p)
                    }, )*
                })
            }
        }
    };
}

gl_api! {
    GetError() -> GLenum = "glGetError";
    GetIntegerv(GLenum, *mut GLint) = "glGetIntegerv";
    Viewport(GLint, GLint, GLsizei, GLsizei) = "glViewport";
    ClearColor(GLfloat, GLfloat, GLfloat, GLfloat) = "glClearColor";
    Clear(GLbitfield) = "glClear";
    Enable(GLenum) = "glEnable";
    Disable(GLenum) = "glDisable";
    DepthFunc(GLenum) = "glDepthFunc";
    DepthMask(GLboolean) = "glDepthMask";
    BlendFunc(GLenum, GLenum) = "glBlendFunc";
    CullFace(GLenum) = "glCullFace";
    FrontFace(GLenum) = "glFrontFace";
    ReadPixels(GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, *mut c_void) = "glReadPixels";
    ReadBuffer(GLenum) = "glReadBuffer";
    PixelStorei(GLenum, GLint) = "glPixelStorei";
    DrawElements(GLenum, GLsizei, GLenum, *const c_void) = "glDrawElements";
    DrawArrays(GLenum, GLint, GLsizei) = "glDrawArrays";
    GenBuffers(GLsizei, *mut GLuint) = "glGenBuffers" | "glGenBuffersARB";
    DeleteBuffers(GLsizei, *const GLuint) = "glDeleteBuffers" | "glDeleteBuffersARB";
    BindBuffer(GLenum, GLuint) = "glBindBuffer" | "glBindBufferARB";
    BufferData(GLenum, GLsizeiptr, *const c_void, GLenum) = "glBufferData" | "glBufferDataARB";
    CreateShader(GLenum) -> GLuint = "glCreateShader";
    ShaderSource(GLuint, GLsizei, *const *const c_char, *const GLint) = "glShaderSource";
    CompileShader(GLuint) = "glCompileShader";
    GetShaderiv(GLuint, GLenum, *mut GLint) = "glGetShaderiv";
    GetShaderInfoLog(GLuint, GLsizei, *mut GLsizei, *mut c_char) = "glGetShaderInfoLog";
    DeleteShader(GLuint) = "glDeleteShader";
    CreateProgram() -> GLuint = "glCreateProgram";
    AttachShader(GLuint, GLuint) = "glAttachShader";
    BindAttribLocation(GLuint, GLuint, *const c_char) = "glBindAttribLocation";
    LinkProgram(GLuint) = "glLinkProgram";
    GetProgramiv(GLuint, GLenum, *mut GLint) = "glGetProgramiv";
    GetProgramInfoLog(GLuint, GLsizei, *mut GLsizei, *mut c_char) = "glGetProgramInfoLog";
    UseProgram(GLuint) = "glUseProgram";
    DeleteProgram(GLuint) = "glDeleteProgram";
    GetUniformLocation(GLuint, *const c_char) -> GLint = "glGetUniformLocation";
    UniformMatrix4fv(GLint, GLsizei, GLboolean, *const GLfloat) = "glUniformMatrix4fv";
    Uniform4f(GLint, GLfloat, GLfloat, GLfloat, GLfloat) = "glUniform4f";
    Uniform3f(GLint, GLfloat, GLfloat, GLfloat) = "glUniform3f";
    Uniform2f(GLint, GLfloat, GLfloat) = "glUniform2f";
    Uniform1f(GLint, GLfloat) = "glUniform1f";
    EnableVertexAttribArray(GLuint) = "glEnableVertexAttribArray";
    DisableVertexAttribArray(GLuint) = "glDisableVertexAttribArray";
    VertexAttribPointer(GLuint, GLint, GLenum, GLboolean, GLsizei, *const c_void) = "glVertexAttribPointer";
    VertexAttrib4f(GLuint, GLfloat, GLfloat, GLfloat, GLfloat) = "glVertexAttrib4f";
    DrawBuffers(GLsizei, *const GLenum) = "glDrawBuffers" | "glDrawBuffersARB";
    GenFramebuffers(GLsizei, *mut GLuint) = "glGenFramebuffers" | "glGenFramebuffersEXT";
    DeleteFramebuffers(GLsizei, *const GLuint) = "glDeleteFramebuffers" | "glDeleteFramebuffersEXT";
    BindFramebuffer(GLenum, GLuint) = "glBindFramebuffer" | "glBindFramebufferEXT";
    CheckFramebufferStatus(GLenum) -> GLenum = "glCheckFramebufferStatus" | "glCheckFramebufferStatusEXT";
    FramebufferRenderbuffer(GLenum, GLenum, GLenum, GLuint) = "glFramebufferRenderbuffer" | "glFramebufferRenderbufferEXT";
    GenRenderbuffers(GLsizei, *mut GLuint) = "glGenRenderbuffers" | "glGenRenderbuffersEXT";
    DeleteRenderbuffers(GLsizei, *const GLuint) = "glDeleteRenderbuffers" | "glDeleteRenderbuffersEXT";
    BindRenderbuffer(GLenum, GLuint) = "glBindRenderbuffer" | "glBindRenderbufferEXT";
    RenderbufferStorage(GLenum, GLenum, GLsizei, GLsizei) = "glRenderbufferStorage" | "glRenderbufferStorageEXT";
    Finish() = "glFinish";
}

// ---------------------------------------------------------------------------
// Платформенный контекст
// ---------------------------------------------------------------------------

/// Контекст OpenGL, созданный платформенным кодом.
pub(crate) trait GlContext {
    fn make_current(&self) -> bool;
    fn get_proc(&self, name: &[u8]) -> *const c_void;
}

/// Диагностика инициализации OpenGL в stderr (переменная `ALPHACAD_GL_TRACE=1`).
pub(crate) fn trace(msg: &str) {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    if *ON.get_or_init(|| std::env::var("ALPHACAD_GL_TRACE").map(|v| v == "1").unwrap_or(false)) {
        eprintln!("[gl] {msg}");
    }
}

#[allow(unreachable_code)]
fn create_context() -> Result<Box<dyn GlContext>, String> {
    #[cfg(target_os = "windows")]
    {
        return wgl::WglContext::create().map(|c| Box::new(c) as Box<dyn GlContext>);
    }
    #[cfg(target_os = "macos")]
    {
        return cgl::CglContext::create().map(|c| Box::new(c) as Box<dyn GlContext>);
    }
    Err("аппаратный рендер на этой платформе не поддерживается".into())
}

// ---------------------------------------------------------------------------
// Шейдеры
// ---------------------------------------------------------------------------

const TRI_VS: &str = r#"#version 120
attribute vec3 a_pos;
attribute vec3 a_nrm;
attribute vec4 a_col;
uniform mat4 u_mvp;
uniform mat4 u_model;
uniform mat4 u_nm;
uniform vec3 u_eye;
uniform vec3 u_back;
uniform vec3 u_l1;
uniform vec3 u_l2;
uniform float u_ortho;
uniform vec4 u_base;
uniform float u_usecol;
varying vec3 v_cf;
varying vec3 v_cb;
vec2 light(vec3 n, vec3 v) {
    float d1 = max(dot(n, u_l1), 0.0);
    float d2 = max(dot(n, u_l2), 0.0);
    vec3 hv = normalize(u_l1 + v);
    float s = max(dot(n, hv), 0.0);
    float s2 = s * s;
    float s4 = s2 * s2;
    float s8 = s4 * s4;
    return vec2(0.30 + 0.58 * d1 + 0.22 * d2 + 0.10 * max(n.z, 0.0), 0.22 * s8 * s8);
}
void main() {
    vec3 wp = (u_model * vec4(a_pos, 1.0)).xyz;
    vec3 n = normalize(mat3(u_nm) * a_nrm);
    vec3 v = u_ortho > 0.5 ? u_back : normalize(u_eye - wp);
    vec2 lf = light(n, v);
    vec2 lb = light(-n, v);
    vec3 base = u_usecol > 0.5 ? a_col.rgb : u_base.rgb;
    v_cf = base * lf.x + vec3(lf.y);
    v_cb = base * (lb.x * 0.85) + vec3(lb.y);
    gl_Position = u_mvp * vec4(a_pos, 1.0);
}
"#;

const TRI_FS: &str = r#"#version 120
varying vec3 v_cf;
varying vec3 v_cb;
uniform float u_alpha;
uniform vec4 u_id;
void main() {
    vec3 c = gl_FrontFacing ? v_cf : v_cb;
    gl_FragData[0] = vec4(clamp(c, 0.0, 1.0), u_alpha);
    gl_FragData[1] = u_id;
}
"#;

/// Линия-четырёхугольник: обе конечные точки в атрибутах, расширение
/// до нужной толщины в пикселях и отсечение по ближней плоскости — в шейдере.
const LINE_VS: &str = r#"#version 120
attribute vec3 a_a;
attribute vec3 a_b;
attribute float a_corner;
attribute vec4 a_color;
attribute vec4 a_wd;
uniform mat4 u_mvp;
uniform vec2 u_vp;
uniform float u_bias_c;
uniform float u_bias_w;
varying vec4 v_color;
varying float v_d;
varying float v_hw;
void main() {
    vec4 ca = u_mvp * vec4(a_a, 1.0);
    vec4 cb = u_mvp * vec4(a_b, 1.0);
    float da = ca.z + ca.w;
    float db = cb.z + cb.w;
    v_color = a_color;
    v_hw = max(a_wd.x, 0.6) * 0.5;
    v_d = 0.0;
    if (da < 0.0 && db < 0.0) {
        gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
        return;
    }
    if (da < 0.0) {
        ca = mix(ca, cb, da / (da - db));
    } else if (db < 0.0) {
        cb = mix(cb, ca, db / (db - da));
    }
    vec2 sa = ca.xy / ca.w * 0.5 * u_vp;
    vec2 sb = cb.xy / cb.w * 0.5 * u_vp;
    vec2 dir = sb - sa;
    float len = length(dir);
    dir = len > 1e-5 ? dir / len : vec2(1.0, 0.0);
    vec2 nrm = vec2(-dir.y, dir.x);
    float e = step(1.5, a_corner);
    float side = (a_corner - 2.0 * e) * 2.0 - 1.0;
    float ext = v_hw + 1.0;
    vec4 c = e > 0.5 ? cb : ca;
    vec2 off = nrm * (side * ext) + dir * ((e * 2.0 - 1.0) * 0.5);
    c.xy += off * 2.0 / u_vp * c.w;
    if (a_wd.y > 0.5) {
        c.z += u_bias_c - u_bias_w * c.w;
    } else {
        c.z = -c.w;
    }
    v_d = side * ext;
    gl_Position = c;
}
"#;

const LINE_FS: &str = r#"#version 120
varying vec4 v_color;
varying float v_d;
varying float v_hw;
void main() {
    float cov = clamp(v_hw + 0.5 - abs(v_d), 0.0, 1.0);
    gl_FragData[0] = vec4(v_color.rgb, v_color.a * cov);
}
"#;

const BG_VS: &str = r#"#version 120
attribute vec2 a_pos;
attribute vec4 a_col;
varying vec4 v_col;
void main() {
    v_col = a_col;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
"#;

const BG_FS: &str = r#"#version 120
varying vec4 v_col;
void main() {
    gl_FragData[0] = vec4(v_col.rgb, 1.0);
    gl_FragData[1] = vec4(0.0);
}
"#;

struct Program {
    id: GLuint,
    uniforms: HashMap<&'static str, GLint>,
}

impl Program {
    fn loc(&self, name: &str) -> GLint {
        self.uniforms.get(name).copied().unwrap_or(-1)
    }
}

unsafe fn compile(gl: &Gl, kind: GLenum, src: &str) -> Result<GLuint, String> {
    let sh = (gl.CreateShader)(kind);
    let ptr = src.as_ptr() as *const c_char;
    let len = src.len() as GLint;
    (gl.ShaderSource)(sh, 1, &ptr, &len);
    (gl.CompileShader)(sh);
    let mut ok = 0;
    (gl.GetShaderiv)(sh, GL_COMPILE_STATUS, &mut ok);
    if ok == 0 {
        let mut buf = vec![0u8; 4096];
        let mut n = 0;
        (gl.GetShaderInfoLog)(sh, buf.len() as GLsizei, &mut n, buf.as_mut_ptr() as *mut c_char);
        buf.truncate(n.max(0) as usize);
        (gl.DeleteShader)(sh);
        return Err(format!("шейдер: {}", String::from_utf8_lossy(&buf)));
    }
    Ok(sh)
}

unsafe fn program(gl: &Gl, vs: &str, fs: &str, attribs: &[&str], uniforms: &[&'static str]) -> Result<Program, String> {
    let v = compile(gl, GL_VERTEX_SHADER, vs)?;
    let f = compile(gl, GL_FRAGMENT_SHADER, fs)?;
    let p = (gl.CreateProgram)();
    (gl.AttachShader)(p, v);
    (gl.AttachShader)(p, f);
    for (i, a) in attribs.iter().enumerate() {
        let c = format!("{a}\0");
        (gl.BindAttribLocation)(p, i as GLuint, c.as_ptr() as *const c_char);
    }
    (gl.LinkProgram)(p);
    (gl.DeleteShader)(v);
    (gl.DeleteShader)(f);
    let mut ok = 0;
    (gl.GetProgramiv)(p, GL_LINK_STATUS, &mut ok);
    if ok == 0 {
        let mut buf = vec![0u8; 4096];
        let mut n = 0;
        (gl.GetProgramInfoLog)(p, buf.len() as GLsizei, &mut n, buf.as_mut_ptr() as *mut c_char);
        buf.truncate(n.max(0) as usize);
        (gl.DeleteProgram)(p);
        return Err(format!("программа: {}", String::from_utf8_lossy(&buf)));
    }
    let mut map = HashMap::new();
    for u in uniforms {
        let c = format!("{u}\0");
        map.insert(*u, (gl.GetUniformLocation)(p, c.as_ptr() as *const c_char));
    }
    Ok(Program { id: p, uniforms: map })
}

// ---------------------------------------------------------------------------
// Буферы мешей
// ---------------------------------------------------------------------------

struct EdgeBuf {
    vbo: GLuint,
    edges: usize,
}

struct MeshBuf {
    pos: GLuint,
    nrm: GLuint,
    col: Option<GLuint>,
    ibo: GLuint,
    count: usize,
    feature: Option<EdgeBuf>,
    all: Option<EdgeBuf>,
    last_used: u64,
}

/// Предел рёбер для кэша в видеопамяти (28 байт × 4 вершины на ребро).
const MAX_GPU_EDGES: usize = 3_000_000;

/// Вершины рёбер: a.xyz, b.xyz, угол (0..3).
fn edge_vertices(edges: &[[crate::math::Vec3; 2]]) -> Vec<f32> {
    let mut v = Vec::with_capacity(edges.len() * 28);
    for e in edges {
        for c in 0..4 {
            v.extend_from_slice(&[e[0].x, e[0].y, e[0].z, e[1].x, e[1].y, e[1].z, c as f32]);
        }
    }
    v
}

fn color_f(c: u32) -> [f32; 4] {
    [((c >> 16) & 255) as f32 / 255.0, ((c >> 8) & 255) as f32 / 255.0, (c & 255) as f32 / 255.0, (c >> 24) as f32 / 255.0]
}

// ---------------------------------------------------------------------------
// Рендерер
// ---------------------------------------------------------------------------

/// Аппаратный рендерер (OpenGL). Создаётся и используется в одном потоке.
pub struct GpuRenderer {
    ctx: Box<dyn GlContext>,
    gl: Gl,
    tri: Program,
    line: Program,
    bg: Program,
    fbo: GLuint,
    rb: [GLuint; 3],
    size: (usize, usize),
    max_size: usize,
    meshes: HashMap<u64, MeshBuf>,
    quad_ibo: GLuint,
    quad_edges: usize,
    stream: GLuint,
    frame: u64,
    readback: Vec<u32>,
    depth_tmp: Vec<f32>,
    /// Описание устройства: «производитель — модель (версия GL)».
    pub info: String,
}

impl GpuRenderer {
    /// Создание контекста и ресурсов. Ошибка — GPU недоступен (используйте CPU).
    pub fn new() -> Result<GpuRenderer, String> {
        trace("create context");
        let ctx = create_context()?;
        if !ctx.make_current() {
            return Err("не удалось активировать контекст OpenGL".into());
        }
        trace("context current");
        unsafe {
            // Сначала версия и название драйвера — чтобы старый драйвер давал
            // понятное сообщение, а не «нет функции …».
            let gs = ctx.get_proc(b"glGetString\0");
            if gs.is_null() {
                return Err("OpenGL: нет функции glGetString".into());
            }
            let get_string: unsafe extern "system" fn(GLenum) -> *const u8 = std::mem::transmute(gs);
            let s = |e| {
                let p = get_string(e);
                if p.is_null() {
                    String::new()
                } else {
                    std::ffi::CStr::from_ptr(p as *const c_char).to_string_lossy().into_owned()
                }
            };
            let (vendor, renderer, version) = (s(GL_VENDOR), s(GL_RENDERER), s(GL_VERSION));
            let major: u32 = version.split(|c: char| !c.is_ascii_digit()).next().and_then(|v| v.parse().ok()).unwrap_or(0);
            trace(&format!("driver: {vendor} | {renderer} | {version}"));
            if renderer.contains("GDI Generic") {
                return Err("нет драйвера видеокарты (GDI Generic)".into());
            }
            if major < 2 {
                return Err(format!("требуется OpenGL 2.1+, драйвер: {version} ({renderer})"));
            }
            let gl = Gl::load(&|n| ctx.get_proc(n))?;
            trace("functions loaded");
            let tri = program(
                &gl,
                TRI_VS,
                TRI_FS,
                &["a_pos", "a_nrm", "a_col"],
                &["u_mvp", "u_model", "u_nm", "u_eye", "u_back", "u_l1", "u_l2", "u_ortho", "u_base", "u_usecol", "u_alpha", "u_id"],
            )?;
            let line = program(&gl, LINE_VS, LINE_FS, &["a_a", "a_b", "a_corner", "a_color", "a_wd"], &["u_mvp", "u_vp", "u_bias_c", "u_bias_w"])?;
            let bg = program(&gl, BG_VS, BG_FS, &["a_pos", "a_col"], &[])?;
            trace("shaders compiled");
            let mut fbo = 0;
            (gl.GenFramebuffers)(1, &mut fbo);
            let mut rb = [0u32; 3];
            (gl.GenRenderbuffers)(3, rb.as_mut_ptr());
            let mut bufs = [0u32; 2];
            (gl.GenBuffers)(2, bufs.as_mut_ptr());
            let mut max_rb = 0;
            (gl.GetIntegerv)(GL_MAX_RENDERBUFFER_SIZE, &mut max_rb);
            let info = format!("{vendor} — {renderer} (OpenGL {})", version.split_whitespace().next().unwrap_or(&version));
            let mut r = GpuRenderer {
                ctx,
                gl,
                tri,
                line,
                bg,
                fbo,
                rb,
                size: (0, 0),
                max_size: if max_rb > 0 { max_rb as usize } else { 4096 },
                meshes: HashMap::new(),
                quad_ibo: bufs[0],
                quad_edges: 0,
                stream: bufs[1],
                frame: 0,
                readback: Vec::new(),
                depth_tmp: Vec::new(),
                info,
            };
            // Проверочный кадр: FBO должен собраться.
            r.ensure_fbo(16, 16)?;
            Ok(r)
        }
    }

    unsafe fn ensure_fbo(&mut self, w: usize, h: usize) -> Result<(), String> {
        if self.size == (w, h) {
            return Ok(());
        }
        if w > self.max_size || h > self.max_size {
            return Err(format!("размер {w}×{h} больше предела видеокарты {}", self.max_size));
        }
        let gl = &self.gl;
        (gl.BindFramebuffer)(GL_FRAMEBUFFER, self.fbo);
        let formats = [GL_RGBA8, GL_RGBA8, GL_DEPTH_COMPONENT24];
        let attach = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_DEPTH_ATTACHMENT];
        for k in 0..3 {
            (gl.BindRenderbuffer)(GL_RENDERBUFFER, self.rb[k]);
            (gl.RenderbufferStorage)(GL_RENDERBUFFER, formats[k], w as GLsizei, h as GLsizei);
            (gl.FramebufferRenderbuffer)(GL_FRAMEBUFFER, attach[k], GL_RENDERBUFFER, self.rb[k]);
        }
        let st = (gl.CheckFramebufferStatus)(GL_FRAMEBUFFER);
        if st != GL_FRAMEBUFFER_COMPLETE {
            self.size = (0, 0);
            return Err(format!("FBO не собран (0x{st:X})"));
        }
        self.size = (w, h);
        trace(&format!("FBO {w}x{h} ready"));
        Ok(())
    }

    unsafe fn upload_buffer(&self, target: GLenum, bytes: &[u8]) -> GLuint {
        let mut b = 0;
        (self.gl.GenBuffers)(1, &mut b);
        (self.gl.BindBuffer)(target, b);
        (self.gl.BufferData)(target, bytes.len() as GLsizeiptr, bytes.as_ptr() as *const c_void, GL_STATIC_DRAW);
        b
    }

    unsafe fn mesh_buf(&mut self, dm: &DisplayMesh) -> u64 {
        let key = dm.uid;
        if key != 0 && self.meshes.contains_key(&key) {
            return key;
        }
        let pos: Vec<f32> = dm.positions.iter().flat_map(|p| [p.x, p.y, p.z]).collect();
        let nrm: Vec<f32> = dm.normals.iter().flat_map(|p| [p.x, p.y, p.z]).collect();
        let col: Option<Vec<u8>> = dm.colors.as_ref().map(|c| c.iter().flat_map(|c| [c.r, c.g, c.b, 255]).collect());
        let mb = MeshBuf {
            pos: self.upload_buffer(GL_ARRAY_BUFFER, as_bytes(&pos)),
            nrm: self.upload_buffer(GL_ARRAY_BUFFER, as_bytes(&nrm)),
            col: col.map(|c| self.upload_buffer(GL_ARRAY_BUFFER, &c)),
            ibo: self.upload_buffer(GL_ELEMENT_ARRAY_BUFFER, as_bytes(&dm.indices)),
            count: dm.indices.len(),
            feature: None,
            all: None,
            last_used: self.frame,
        };
        // Меши без uid (0) кэшируются под временным ключом и удаляются после кадра.
        let key = if key == 0 { u64::MAX - self.meshes.len() as u64 } else { key };
        self.meshes.insert(key, mb);
        key
    }

    unsafe fn edge_buf(&mut self, key: u64, dm: &DisplayMesh, all: bool) -> Option<(GLuint, usize)> {
        let have = self.meshes.get(&key).and_then(|m| if all { m.all.as_ref() } else { m.feature.as_ref() }).map(|e| (e.vbo, e.edges));
        if have.is_some() {
            return have;
        }
        let edges: &[[crate::math::Vec3; 2]] = if all { dm.all_edges() } else { &dm.feature_edges };
        if edges.is_empty() || edges.len() > MAX_GPU_EDGES {
            return None;
        }
        let v = edge_vertices(edges);
        let vbo = self.upload_buffer(GL_ARRAY_BUFFER, as_bytes(&v));
        let eb = EdgeBuf { vbo, edges: edges.len() };
        let m = self.meshes.get_mut(&key)?;
        if all {
            m.all = Some(eb);
        } else {
            m.feature = Some(eb);
        }
        Some((vbo, edges.len()))
    }

    /// Общий индексный буфер четырёхугольников рёбер (6 индексов на ребро).
    unsafe fn ensure_quads(&mut self, edges: usize) {
        if edges <= self.quad_edges {
            (self.gl.BindBuffer)(GL_ELEMENT_ARRAY_BUFFER, self.quad_ibo);
            return;
        }
        let n = edges.next_power_of_two().max(1024);
        let mut idx: Vec<u32> = Vec::with_capacity(n * 6);
        for k in 0..n as u32 {
            let b = k * 4;
            idx.extend_from_slice(&[b, b + 1, b + 2, b + 2, b + 1, b + 3]);
        }
        (self.gl.BindBuffer)(GL_ELEMENT_ARRAY_BUFFER, self.quad_ibo);
        let bytes = as_bytes(&idx);
        (self.gl.BufferData)(GL_ELEMENT_ARRAY_BUFFER, bytes.len() as GLsizeiptr, bytes.as_ptr() as *const c_void, GL_STATIC_DRAW);
        self.quad_edges = n;
    }

    unsafe fn delete_mesh(gl: &Gl, m: &MeshBuf) {
        let mut ids = vec![m.pos, m.nrm, m.ibo];
        if let Some(c) = m.col {
            ids.push(c);
        }
        if let Some(e) = &m.feature {
            ids.push(e.vbo);
        }
        if let Some(e) = &m.all {
            ids.push(e.vbo);
        }
        (gl.DeleteBuffers)(ids.len() as GLsizei, ids.as_ptr());
    }

    fn disable_attribs(&self) {
        unsafe {
            for i in 0..5 {
                (self.gl.DisableVertexAttribArray)(i);
            }
        }
    }

    /// Отрисовка кадра в `fb` (цвет, идентификаторы, глубина-«близость»).
    pub fn render(&mut self, fb: &mut Framebuffer, cam: &Camera, items: &[RenderItem], overlay: &[Line3], s: &RenderSettings) -> Result<RenderStats, String> {
        let t0 = std::time::Instant::now();
        if !self.ctx.make_current() {
            return Err("контекст OpenGL потерян".into());
        }
        let (w, h) = (fb.width.max(1), fb.height.max(1));
        let mut stats = RenderStats::default();
        unsafe {
            self.ensure_fbo(w, h)?;
            self.frame += 1;
            let tr = self.frame <= 2;
            if tr {
                trace(&format!("frame {}: {} items, {} overlay lines", self.frame, items.len(), overlay.len()));
            }
            let gl = &self.gl;
            (gl.BindFramebuffer)(GL_FRAMEBUFFER, self.fbo);
            (gl.Viewport)(0, 0, w as GLsizei, h as GLsizei);
            (gl.Disable)(GL_DITHER);
            (gl.Disable)(GL_SCISSOR_TEST);
            (gl.FrontFace)(GL_CCW);
            (gl.CullFace)(GL_BACK);
            let both = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1];
            (gl.DrawBuffers)(2, both.as_ptr());
            (gl.ClearColor)(0.0, 0.0, 0.0, 0.0);
            (gl.DepthMask)(1);
            (gl.Clear)(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);

            // Фон — вертикальный градиент.
            (gl.Disable)(GL_DEPTH_TEST);
            (gl.Disable)(GL_BLEND);
            (gl.Disable)(GL_CULL_FACE);
            (gl.UseProgram)(self.bg.id);
            let (t, b) = (color_f(s.bg_top), color_f(s.bg_bottom));
            #[rustfmt::skip]
            let quad: [f32; 24] = [
                -1.0, 1.0, t[0], t[1], t[2], 1.0,
                1.0, 1.0, t[0], t[1], t[2], 1.0,
                -1.0, -1.0, b[0], b[1], b[2], 1.0,
                1.0, -1.0, b[0], b[1], b[2], 1.0,
            ];
            self.disable_attribs();
            let gl = &self.gl;
            (gl.BindBuffer)(GL_ARRAY_BUFFER, self.stream);
            let qb = as_bytes(&quad);
            (gl.BufferData)(GL_ARRAY_BUFFER, qb.len() as GLsizeiptr, qb.as_ptr() as *const c_void, GL_STREAM_DRAW);
            (gl.EnableVertexAttribArray)(0);
            (gl.EnableVertexAttribArray)(1);
            (gl.VertexAttribPointer)(0, 2, GL_FLOAT, 0, 24, std::ptr::null());
            (gl.VertexAttribPointer)(1, 4, GL_FLOAT, 0, 24, 8 as *const c_void);
            (gl.DrawArrays)(GL_TRIANGLE_STRIP, 0, 4);
            if tr {
                (gl.Finish)();
                trace("background drawn");
            }
        }

        let aspect = w as f32 / h as f32;
        let proj = cam.proj(aspect);
        let vp = cam.view_proj(aspect);
        let eye = cam.eye();
        let back = cam.back_dir();
        let (l1, l2) = software::lights(cam);

        // --- Треугольники: сначала непрозрачные, затем «призраки».
        unsafe {
            (self.gl.UseProgram)(self.tri.id);
            let tp = &self.tri;
            let gl = &self.gl;
            (gl.Uniform3f)(tp.loc("u_eye"), eye.x, eye.y, eye.z);
            (gl.Uniform3f)(tp.loc("u_back"), back.x, back.y, back.z);
            (gl.Uniform3f)(tp.loc("u_l1"), l1.x, l1.y, l1.z);
            (gl.Uniform3f)(tp.loc("u_l2"), l2.x, l2.y, l2.z);
            (gl.Uniform1f)(tp.loc("u_ortho"), if cam.ortho { 1.0 } else { 0.0 });
            (gl.Enable)(GL_DEPTH_TEST);
            (gl.DepthFunc)(GL_LESS);
        }
        for pass_ghost in [false, true] {
            unsafe {
                let gl = &self.gl;
                if pass_ghost {
                    let one = [GL_COLOR_ATTACHMENT0];
                    (gl.DrawBuffers)(1, one.as_ptr());
                    (gl.Enable)(GL_BLEND);
                    (gl.BlendFunc)(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
                    (gl.DepthMask)(0);
                } else {
                    let both = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1];
                    (gl.DrawBuffers)(2, both.as_ptr());
                    (gl.Disable)(GL_BLEND);
                    (gl.DepthMask)(1);
                }
            }
            for it in items {
                if it.ghost != pass_ghost {
                    continue;
                }
                if !pass_ghost {
                    stats.triangles += it.mesh.triangle_count();
                }
                if it.wire && !it.ghost {
                    continue;
                }
                if it.mesh.indices.is_empty() {
                    continue;
                }
                unsafe {
                    let key = self.mesh_buf(it.mesh);
                    let frame = self.frame;
                    let Some(mb) = self.meshes.get_mut(&key) else { continue };
                    mb.last_used = frame;
                    let (pos, nrm, col, ibo, count) = (mb.pos, mb.nrm, mb.col, mb.ibo, mb.count);
                    self.disable_attribs();
                    let gl = &self.gl;
                    let tp = &self.tri;
                    let mvp = vp * it.model;
                    let nm = it.model.normal_matrix();
                    (gl.UniformMatrix4fv)(tp.loc("u_mvp"), 1, 0, mvp.m.as_ptr());
                    (gl.UniformMatrix4fv)(tp.loc("u_model"), 1, 0, it.model.m.as_ptr());
                    (gl.UniformMatrix4fv)(tp.loc("u_nm"), 1, 0, nm.m.as_ptr());
                    let base = software::base_color(it, s.select_color);
                    (gl.Uniform4f)(tp.loc("u_base"), base.r as f32 / 255.0, base.g as f32 / 255.0, base.b as f32 / 255.0, 1.0);
                    let use_col = col.is_some() && it.highlight == software::Highlight::None;
                    (gl.Uniform1f)(tp.loc("u_usecol"), if use_col { 1.0 } else { 0.0 });
                    (gl.Uniform1f)(tp.loc("u_alpha"), if it.ghost { 0x5A as f32 / 255.0 } else { 1.0 });
                    let id = it.id;
                    (gl.Uniform4f)(tp.loc("u_id"), (id & 255) as f32 / 255.0, ((id >> 8) & 255) as f32 / 255.0, ((id >> 16) & 255) as f32 / 255.0, 1.0);
                    if it.mesh.closed && !it.ghost {
                        (gl.Enable)(GL_CULL_FACE);
                    } else {
                        (gl.Disable)(GL_CULL_FACE);
                    }
                    (gl.BindBuffer)(GL_ARRAY_BUFFER, pos);
                    (gl.EnableVertexAttribArray)(0);
                    (gl.VertexAttribPointer)(0, 3, GL_FLOAT, 0, 12, std::ptr::null());
                    (gl.BindBuffer)(GL_ARRAY_BUFFER, nrm);
                    (gl.EnableVertexAttribArray)(1);
                    (gl.VertexAttribPointer)(1, 3, GL_FLOAT, 0, 12, std::ptr::null());
                    if let (true, Some(c)) = (use_col, col) {
                        (gl.BindBuffer)(GL_ARRAY_BUFFER, c);
                        (gl.EnableVertexAttribArray)(2);
                        (gl.VertexAttribPointer)(2, 4, GL_UNSIGNED_BYTE, 1, 4, std::ptr::null());
                    } else {
                        (gl.VertexAttrib4f)(2, 1.0, 1.0, 1.0, 1.0);
                    }
                    (gl.BindBuffer)(GL_ELEMENT_ARRAY_BUFFER, ibo);
                    (gl.DrawElements)(GL_TRIANGLES, count as GLsizei, GL_UNSIGNED_INT, std::ptr::null());
                    stats.drawn_triangles += count / 3;
                }
            }
        }

        // --- Линии: рёбра объектов и наложения (без записи глубины, со смешиванием).
        let (bias_c, bias_w) = if cam.ortho {
            (0.0, 2.0 * 2.5e-3 * cam.distance / (cam.far - cam.near).max(1e-6))
        } else {
            (2.5e-3 * proj.m[14], 0.0)
        };
        unsafe {
            let gl = &self.gl;
            if self.frame <= 2 {
                (gl.Finish)();
                trace("meshes drawn");
            }
            let one = [GL_COLOR_ATTACHMENT0];
            (gl.DrawBuffers)(1, one.as_ptr());
            (gl.Disable)(GL_CULL_FACE);
            (gl.Enable)(GL_BLEND);
            (gl.BlendFunc)(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
            (gl.DepthMask)(0);
            (gl.DepthFunc)(GL_LEQUAL);
            (gl.UseProgram)(self.line.id);
            let lp = &self.line;
            (gl.Uniform2f)(lp.loc("u_vp"), w as f32, h as f32);
            (gl.Uniform1f)(lp.loc("u_bias_c"), bias_c);
            (gl.Uniform1f)(lp.loc("u_bias_w"), bias_w);
        }
        for it in items {
            let Some((color, width, all)) = software::edge_style(it, s) else { continue };
            unsafe {
                let key = self.mesh_buf(it.mesh);
                let eb = self.edge_buf(key, it.mesh, all).or_else(|| if all { self.edge_buf(key, it.mesh, false) } else { None });
                let Some((vbo, n)) = eb else { continue };
                stats.lines += n;
                self.ensure_quads(n);
                self.disable_attribs();
                let gl = &self.gl;
                let mvp = vp * it.model;
                (gl.UniformMatrix4fv)(self.line.loc("u_mvp"), 1, 0, mvp.m.as_ptr());
                let c = color_f(color);
                (gl.VertexAttrib4f)(3, c[0], c[1], c[2], c[3]);
                (gl.VertexAttrib4f)(4, width, 1.0, 0.0, 0.0);
                (gl.BindBuffer)(GL_ARRAY_BUFFER, vbo);
                (gl.EnableVertexAttribArray)(0);
                (gl.EnableVertexAttribArray)(1);
                (gl.EnableVertexAttribArray)(2);
                (gl.VertexAttribPointer)(0, 3, GL_FLOAT, 0, 28, std::ptr::null());
                (gl.VertexAttribPointer)(1, 3, GL_FLOAT, 0, 28, 12 as *const c_void);
                (gl.VertexAttribPointer)(2, 1, GL_FLOAT, 0, 28, 24 as *const c_void);
                (gl.DrawElements)(GL_TRIANGLES, (n * 6) as GLsizei, GL_UNSIGNED_INT, std::ptr::null());
            }
        }
        if !overlay.is_empty() {
            let ls = s.line_scale;
            let mut v: Vec<f32> = Vec::with_capacity(overlay.len() * 4 * 13);
            for l in overlay {
                let c = color_f(l.color);
                for k in 0..4 {
                    v.extend_from_slice(&[l.a.x, l.a.y, l.a.z, l.b.x, l.b.y, l.b.z, k as f32, c[0], c[1], c[2], c[3], l.width * ls, if l.depth_test { 1.0 } else { 0.0 }]);
                }
            }
            stats.lines += overlay.len();
            unsafe {
                self.ensure_quads(overlay.len());
                self.disable_attribs();
                let gl = &self.gl;
                (gl.UniformMatrix4fv)(self.line.loc("u_mvp"), 1, 0, vp.m.as_ptr());
                (gl.BindBuffer)(GL_ARRAY_BUFFER, self.stream);
                let vb = as_bytes(&v);
                (gl.BufferData)(GL_ARRAY_BUFFER, vb.len() as GLsizeiptr, vb.as_ptr() as *const c_void, GL_STREAM_DRAW);
                let st = 13 * 4;
                for a in 0..5 {
                    (gl.EnableVertexAttribArray)(a);
                }
                (gl.VertexAttribPointer)(0, 3, GL_FLOAT, 0, st, std::ptr::null());
                (gl.VertexAttribPointer)(1, 3, GL_FLOAT, 0, st, 12 as *const c_void);
                (gl.VertexAttribPointer)(2, 1, GL_FLOAT, 0, st, 24 as *const c_void);
                (gl.VertexAttribPointer)(3, 4, GL_FLOAT, 0, st, 28 as *const c_void);
                (gl.VertexAttribPointer)(4, 2, GL_FLOAT, 0, st, 44 as *const c_void);
                (gl.DrawElements)(GL_TRIANGLES, (overlay.len() * 6) as GLsizei, GL_UNSIGNED_INT, std::ptr::null());
            }
        }

        // --- Считывание: цвет, идентификаторы, глубина (с переворотом по вертикали).
        let tr = self.frame <= 2;
        unsafe {
            if tr {
                (self.gl.Finish)();
                trace("geometry drawn");
            }
            self.disable_attribs();
            let gl = &self.gl;
            (gl.DepthMask)(1);
            (gl.PixelStorei)(GL_PACK_ALIGNMENT, 4);
            let n = w * h;
            self.readback.resize(n, 0);
            self.depth_tmp.resize(n, 0.0);
            (gl.BindFramebuffer)(GL_FRAMEBUFFER, self.fbo);
            let one = [GL_COLOR_ATTACHMENT0];
            (gl.DrawBuffers)(1, one.as_ptr());
            read_attachment(gl, 0, w, h, GL_BGRA, &mut self.readback);
            if tr {
                trace("color read");
            }
            for y in 0..h {
                let src = &self.readback[(h - 1 - y) * w..(h - y) * w];
                let dst = &mut fb.color[y * w..(y + 1) * w];
                for (d, s) in dst.iter_mut().zip(src) {
                    *d = s & 0x00FF_FFFF;
                }
            }
            read_attachment(gl, 1, w, h, GL_RGBA, &mut self.readback);
            if tr {
                trace("ids read");
            }
            for y in 0..h {
                let src = &self.readback[(h - 1 - y) * w..(h - y) * w];
                let dst = &mut fb.ids[y * w..(y + 1) * w];
                for (d, s) in dst.iter_mut().zip(src) {
                    *d = u32::from_le_bytes(s.to_ne_bytes()) & 0x00FF_FFFF;
                }
            }
            (gl.ReadPixels)(0, 0, w as GLsizei, h as GLsizei, GL_DEPTH_COMPONENT, GL_FLOAT, self.depth_tmp.as_mut_ptr() as *mut c_void);
            if tr {
                trace("depth read");
            }
            // Глубина окна → «близость» программного рендера (1/w или 1 − z_ndc).
            let m = &proj.m;
            for y in 0..h {
                let src = &self.depth_tmp[(h - 1 - y) * w..(h - y) * w];
                let dst = &mut fb.depth[y * w..(y + 1) * w];
                for (d, &zw) in dst.iter_mut().zip(src) {
                    *d = depth_to_q(zw, m, cam.ortho);
                }
            }
            let err = (gl.GetError)();
            // Удаление давно не используемых буферов.
            let frame = self.frame;
            let stale: Vec<u64> = self.meshes.iter().filter(|(k, m)| m.last_used + 60 < frame || **k > u64::MAX - 100_000).map(|(k, _)| *k).collect();
            for k in stale {
                if let Some(m) = self.meshes.remove(&k) {
                    Self::delete_mesh(&self.gl, &m);
                }
            }
            if err != GL_NO_ERROR {
                return Err(format!("ошибка OpenGL 0x{err:X}"));
            }
        }
        stats.millis = t0.elapsed().as_secs_f32() * 1000.0;
        Ok(stats)
    }
}

impl Drop for GpuRenderer {
    fn drop(&mut self) {
        if !self.ctx.make_current() {
            return;
        }
        unsafe {
            let gl = &self.gl;
            for m in self.meshes.values() {
                Self::delete_mesh(gl, m);
            }
            let bufs = [self.quad_ibo, self.stream];
            (gl.DeleteBuffers)(2, bufs.as_ptr());
            (gl.DeleteRenderbuffers)(3, self.rb.as_ptr());
            (gl.DeleteFramebuffers)(1, &self.fbo);
            (gl.DeleteProgram)(self.tri.id);
            (gl.DeleteProgram)(self.line.id);
            (gl.DeleteProgram)(self.bg.id);
            (gl.Finish)();
        }
    }
}

/// Перевод значения буфера глубины OpenGL (0..1) в «близость» программного рендера.
fn depth_to_q(zw: f32, m: &[f32; 16], ortho: bool) -> f32 {
    if zw >= 1.0 {
        return 0.0;
    }
    let zn = zw * 2.0 - 1.0;
    if ortho {
        return 1.0 - zn;
    }
    // z_ndc = (m10·z + m14) / (m11·z + m15)  →  z вида  →  w = m11·z + m15.
    let den = zn * m[11] - m[10];
    if den.abs() < 1e-20 {
        return 0.0;
    }
    let z = (m[14] - zn * m[15]) / den;
    let wv = m[11] * z + m[15];
    if wv > 0.0 {
        1.0 / wv
    } else {
        0.0
    }
}

unsafe fn read_attachment(gl: &Gl, k: u32, w: usize, h: usize, fmt: GLenum, out: &mut [u32]) {
    (gl.ReadBuffer)(GL_COLOR_ATTACHMENT0 + k);
    (gl.ReadPixels)(0, 0, w as GLsizei, h as GLsizei, fmt, GL_UNSIGNED_BYTE, out.as_mut_ptr() as *mut c_void);
}

fn as_bytes<T: Copy>(v: &[T]) -> &[u8] {
    // SAFETY: используется только для f32/u32 — типов без заполнителей.
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_conversion_matches_projection() {
        let cam = Camera { near: 1.0, far: 100.0, ..Default::default() };
        let p = cam.proj(1.5);
        // Точка на глубине 10 перед камерой.
        let zc = p.m[10] * -10.0 + p.m[14];
        let wc = p.m[11] * -10.0 + p.m[15];
        let zw = (zc / wc) * 0.5 + 0.5;
        let q = depth_to_q(zw, &p.m, false);
        assert!((q - 0.1).abs() < 1e-4, "q = {q}");
        assert_eq!(depth_to_q(1.0, &p.m, false), 0.0);
        let mut co = cam;
        co.ortho = true;
        let po = co.proj(1.5);
        assert!((depth_to_q(0.25, &po.m, true) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn edge_vertex_layout() {
        use crate::math::Vec3;
        let v = edge_vertices(&[[Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0)]]);
        assert_eq!(v.len(), 28);
        assert_eq!(&v[21..28], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 3.0]);
    }
}
