//! Проверка драйвера OpenGL в отдельном процессе.
//!
//! Ошибку драйвера (неподдерживаемая функция, нет FBO) [`GpuRenderer::new`]
//! возвращает как `Err`, но неисправный драйвер может и аварийно завершить
//! процесс — внутри процесса это не перехватить. Поэтому перед первым включением
//! видеокарты программа запускает саму себя с ключом `--gpu-probe`: дочерний
//! процесс создаёт контекст и рисует пробный кадр. Если он упал, завис или
//! получил неверную картинку — остаёмся на процессоре, а пользователь видит
//! причину в настройках.

use super::GpuRenderer;
use crate::math::{Color, Mat4, Vec3};
use crate::renderer::canvas::Framebuffer;
use crate::renderer::software::{Camera, DisplayMesh, Highlight, Line3, RenderItem, RenderSettings};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Аргумент командной строки дочернего процесса.
pub const PROBE_ARG: &str = "--gpu-probe";
const TIMEOUT: Duration = Duration::from_secs(20);

/// Результат проверки — один раз за сеанс.
static RESULT: Mutex<Option<Result<(), String>>> = Mutex::new(None);

/// Проверка драйвера (кэшируется). `Ok` — видеокарту можно использовать.
pub fn check() -> Result<(), String> {
    let mut r = RESULT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = r.as_ref() {
        return v.clone();
    }
    let v = run();
    super::trace(&format!("probe: {v:?}"));
    *r = Some(v.clone());
    v
}

fn run() -> Result<(), String> {
    use std::process::{Command, Stdio};
    if std::env::var("ALPHACAD_GL_NO_PROBE").map(|v| v == "1").unwrap_or(false) {
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| format!("проверка драйвера: {e}"))?;
    let out = std::env::temp_dir().join(format!("alphacad-gpu-probe-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let mut child = Command::new(exe)
        .arg(PROBE_ARG)
        .arg(&out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(if super::trace_on() { Stdio::inherit() } else { Stdio::null() })
        .spawn()
        .map_err(|e| format!("проверка драйвера: {e}"))?;
    let t0 = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if t0.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&out);
                return Err("драйвер OpenGL не отвечает".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(15)),
            Err(e) => return Err(format!("проверка драйвера: {e}")),
        }
    };
    let text = std::fs::read_to_string(&out).unwrap_or_default();
    let _ = std::fs::remove_file(&out);
    interpret(&text, status.success(), status.code())
}

/// Разбор отчёта дочернего процесса.
fn interpret(text: &str, success: bool, code: Option<i32>) -> Result<(), String> {
    let last = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
    if let Some(msg) = last.strip_prefix("ERR ") {
        return Err(msg.to_string());
    }
    if success && last.starts_with("OK") {
        return Ok(());
    }
    // Последний пройденный этап помогает понять, где упал драйвер.
    let stage = last.strip_prefix("STAGE ").unwrap_or("запуск");
    Err(match code {
        // Коды Windows (NTSTATUS/HRESULT) отрицательны — их принято писать в hex.
        Some(c) if c < 0 => format!("драйвер OpenGL аварийно завершился (код 0x{:08X}, этап: {stage})", c as u32),
        Some(c) => format!("драйвер OpenGL аварийно завершился (код {c}, этап: {stage})"),
        None => format!("драйвер OpenGL аварийно завершился (этап: {stage})"),
    })
}

/// Тело дочернего процесса: создать рендерер, нарисовать пробный кадр,
/// проверить картинку. Отчёт — построчно в файл (каждый этап сразу на диск,
/// чтобы после падения было видно, где оно произошло).
pub fn child_main(report: Option<&str>) -> i32 {
    use std::io::Write;
    let mut file = report.and_then(|p| std::fs::File::create(p).ok());
    let mut log = |s: &str| {
        if let Some(f) = file.as_mut() {
            let _ = writeln!(f, "{s}");
            let _ = f.flush();
        }
        super::trace(s);
    };
    log("STAGE контекст");
    let mut gpu = match GpuRenderer::new() {
        Ok(g) => g,
        Err(e) => {
            log(&format!("ERR {e}"));
            return 0;
        }
    };
    log("STAGE пробный кадр");
    match test_frame(&mut gpu) {
        Ok(()) => {
            log(&format!("OK {}", gpu.info));
            0
        }
        Err(e) => {
            log(&format!("ERR {e}"));
            0
        }
    }
}

/// Кадр с кубом и линиями. Проверяет фон, заливку, буфер выбора и глубину.
fn test_frame(gpu: &mut GpuRenderer) -> Result<(), String> {
    let (w, h) = (96usize, 72usize);
    let mesh = crate::mesh::primitive_box(10.0, 10.0, 10.0);
    let dm = DisplayMesh::build(&mesh, true, 35.0);
    let items = [RenderItem { id: 7, mesh: &dm, model: Mat4::identity(), color: Color::rgb(200, 60, 40), highlight: Highlight::None, wire: false, ghost: false }];
    let lines = [Line3 { a: Vec3::new(-30.0, 0.0, 0.0), b: Vec3::new(30.0, 0.0, 0.0), color: 0xFF20_40E0, width: 1.5, depth_test: true }];
    let cam = Camera { distance: 60.0, near: 1.0, far: 500.0, ..Camera::default() };
    let settings = RenderSettings { bg_top: 0xFF10_2030, bg_bottom: 0xFF10_2030, ..RenderSettings::default() };
    let mut fb = Framebuffer::new(w, h);
    // Два кадра: второй использует кэш буферов, как при обычной работе.
    for _ in 0..2 {
        gpu.render(&mut fb, &cam, &items, &lines, &settings)?;
    }
    let center = (h / 2) * w + w / 2;
    if fb.ids[center] != 7 {
        return Err(format!("неверный буфер выбора (0x{:X})", fb.ids[center]));
    }
    if fb.ids[0] != 0 || (fb.color[0] & 0x00FF_FFFF) != 0x10_2030 {
        return Err(format!("неверный фон (0x{:06X})", fb.color[0] & 0x00FF_FFFF));
    }
    let c = fb.color[center];
    let (r, g) = ((c >> 16) & 0xFF, (c >> 8) & 0xFF);
    if r <= g {
        return Err(format!("неверный цвет модели (0x{:06X})", c & 0x00FF_FFFF));
    }
    if fb.depth[center] <= 0.0 {
        return Err("неверный буфер глубины".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::interpret;

    #[test]
    fn interpret_reports() {
        assert!(interpret("STAGE контекст\nSTAGE пробный кадр\nOK Mesa", true, Some(0)).is_ok());
        assert_eq!(interpret("STAGE контекст\nERR нет FBO\n", true, Some(0)), Err("нет FBO".into()));
        let e = interpret("STAGE контекст\nSTAGE пробный кадр\n", false, Some(87)).unwrap_err();
        assert!(e.contains("87") && e.contains("пробный кадр"), "{e}");
        assert!(interpret("", false, None).unwrap_err().contains("запуск"));
        assert!(interpret("", false, Some(-2147024809)).unwrap_err().contains("0x80070057"));
    }
}
