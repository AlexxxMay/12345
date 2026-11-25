//! Командная строка.
//!
//! ```text
//! alphacad [файлы…]                         — запуск редактора
//! alphacad info <файл>                      — сведения о модели
//! alphacad convert <вход> <выход> [--ascii] — конвертация форматов
//! alphacad snapshot <выход.png> [файлы…] [--size WxH] [--lang ru|en] [--theme light|dark]
//!          [--font standard|engineering] [--render cpu|gpu] [--require-gpu] [--script a,b,…]
//!                                           — снимок интерфейса без окна
//! ```

use crate::app::{Action, App};
use crate::formats;
use crate::platform::{Event, Handler};
use crate::ui::{MouseButton};
use std::path::{Path, PathBuf};

pub enum Command {
    Gui(Vec<PathBuf>),
    Info(PathBuf),
    Convert { input: PathBuf, output: PathBuf, ascii: bool },
    Snapshot { output: PathBuf, files: Vec<PathBuf>, size: (usize, usize), lang: Option<bool>, script: Vec<String>, opts: SnapshotOpts },
    Help,
    Version,
}

pub fn parse(args: &[String]) -> Result<Command, String> {
    let Some(first) = args.first() else { return Ok(Command::Gui(Vec::new())) };
    match first.as_str() {
        "-h" | "--help" | "help" => Ok(Command::Help),
        "-V" | "--version" | "version" => Ok(Command::Version),
        "info" => {
            let p = args.get(1).ok_or("info: укажите файл")?;
            Ok(Command::Info(PathBuf::from(p)))
        }
        "convert" => {
            let mut pos = Vec::new();
            let mut ascii = false;
            for a in &args[1..] {
                if a == "--ascii" {
                    ascii = true;
                } else {
                    pos.push(PathBuf::from(a));
                }
            }
            if pos.len() != 2 {
                return Err("convert: нужно <вход> <выход>".into());
            }
            let output = pos.pop().unwrap();
            let input = pos.pop().unwrap();
            Ok(Command::Convert { input, output, ascii })
        }
        "snapshot" => {
            let mut output = None;
            let mut files = Vec::new();
            let mut size = (1440, 900);
            let mut lang = None;
            let mut script = Vec::new();
            let mut opts = SnapshotOpts::default();
            let mut i = 1;
            while i < args.len() {
                let a = &args[i];
                match a.as_str() {
                    "--size" => {
                        i += 1;
                        let v = args.get(i).ok_or("--size WxH")?;
                        let (w, h) = v.split_once('x').ok_or("--size WxH")?;
                        size = (w.parse().map_err(|_| "--size")?, h.parse().map_err(|_| "--size")?);
                    }
                    "--lang" => {
                        i += 1;
                        lang = Some(args.get(i).map(|s| s == "en").unwrap_or(false));
                    }
                    "--theme" => {
                        i += 1;
                        opts.dark = Some(args.get(i).map(|s| s == "dark").unwrap_or(false));
                    }
                    "--font" => {
                        i += 1;
                        opts.engineering_font = Some(args.get(i).map(|s| s == "engineering" || s == "eng").unwrap_or(false));
                    }
                    "--render" => {
                        i += 1;
                        opts.gpu = Some(args.get(i).map(|s| s == "gpu").unwrap_or(false));
                    }
                    "--require-gpu" => {
                        opts.gpu = Some(true);
                        opts.require_gpu = true;
                    }
                    "--script" => {
                        i += 1;
                        script = args.get(i).map(|s| s.split(',').map(|x| x.trim().to_string()).collect()).unwrap_or_default();
                    }
                    _ => {
                        if output.is_none() {
                            output = Some(PathBuf::from(a));
                        } else {
                            files.push(PathBuf::from(a));
                        }
                    }
                }
                i += 1;
            }
            Ok(Command::Snapshot { output: output.ok_or("snapshot: укажите выходной PNG")?, files, size, lang, script, opts })
        }
        _ => Ok(Command::Gui(args.iter().filter(|a| !a.starts_with("-psn_")).map(PathBuf::from).collect())),
    }
}

pub const HELP: &str = "AlphaCad — лёгкий 3D CAD-редактор (STL, STEP, 3MF, OBJ, PLY)

Использование:
  alphacad [файлы…]                          запуск редактора
  alphacad info <файл>                       сведения о модели
  alphacad convert <вход> <выход> [--ascii]  конвертация (формат по расширению)
  alphacad snapshot <out.png> [файлы…] [--size WxH] [--lang ru|en]
           [--theme light|dark] [--font standard|engineering]
           [--render cpu|gpu] [--require-gpu] снимок интерфейса без окна
  alphacad --version";

/// Дополнительные параметры снимка.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnapshotOpts {
    /// Тёмная тема интерфейса.
    pub dark: Option<bool>,
    /// Рендер на видеокарте.
    pub gpu: Option<bool>,
    /// Ошибка, если видеокарта недоступна (для проверки в CI).
    pub require_gpu: bool,
    /// Инженерный шрифт интерфейса.
    pub engineering_font: Option<bool>,
}

/// Выполнение консольной команды. Возвращает код выхода.
pub fn run_console(cmd: Command) -> i32 {
    crate::platform::headless::activate();
    match cmd {
        Command::Help => {
            println!("{HELP}");
            0
        }
        Command::Version => {
            println!("AlphaCad {}", crate::VERSION);
            0
        }
        Command::Info(p) => match formats::load_mesh(&p) {
            Ok(m) => {
                println!("{}", formats::ModelInfo::from_mesh(&m).summary());
                0
            }
            Err(e) => {
                eprintln!("ошибка: {e}");
                1
            }
        },
        Command::Convert { input, output, ascii } => match convert(&input, &output, ascii) {
            Ok(()) => {
                println!("{} → {}", input.display(), output.display());
                0
            }
            Err(e) => {
                eprintln!("ошибка: {e}");
                1
            }
        },
        Command::Snapshot { output, files, size, lang, script, opts } => match snapshot(&output, files, size, lang, &script, &opts) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("ошибка: {e}");
                1
            }
        },
        Command::Gui(_) => 2,
    }
}

pub fn convert(input: &Path, output: &Path, ascii: bool) -> Result<(), String> {
    let mesh = formats::load_mesh(input)?;
    if ascii && formats::format_of_path(output) == Some(formats::ModelFormat::Stl) {
        return std::fs::write(output, formats::stl::to_ascii(&mesh)).map_err(|e| e.to_string());
    }
    formats::save_mesh(&mesh, output)
}

/// Снимок интерфейса: создаёт приложение без окна, проигрывает сценарий и сохраняет PNG.
///
/// Шаги сценария: `box`, `cylinder`, `sphere`, `torus`, `ok` (Enter), `esc`, `fit`,
/// `wire`, `help`, `settings`, `sketch`, `info`, `click:X:Y`, `rclick:X:Y`, `move:X:Y`,
/// `drag:X1:Y1:X2:Y2`, `key:c`, `action:revolve|sweep|loft|shell|solidify|pattern|twist|taper|bend|subdivide|smooth|coil|finish|select-last`,
/// `tool:line|rect|circle|arc|polygon|ellipse|slot|spline|fillet|offset`, `demo:revolve|sweep|loft|shapes`.
pub fn snapshot(output: &Path, files: Vec<PathBuf>, size: (usize, usize), lang: Option<bool>, script: &[String], opts: &SnapshotOpts) -> Result<(), String> {
    let mut app = App::new(files);
    if let Some(en) = lang {
        app.st.cfg.lang_en = en;
        crate::app::set_lang_en(en);
    }
    if let Some(dark) = opts.dark {
        app.st.cfg.dark_theme = dark;
        crate::ui::theme::set_dark(dark);
    }
    if let Some(gpu) = opts.gpu {
        app.st.cfg.gpu = gpu;
    }
    if let Some(f) = opts.engineering_font {
        app.st.cfg.engineering_font = f;
    }
    app.event(Event::Resize { width: size.0, height: size.1, scale: 1.0 });
    app.frame();
    app.st.run_action(Action::Fit);
    settle(&mut app);
    for step in script {
        let parts: Vec<&str> = step.split(':').collect();
        let num = |i: usize| parts.get(i).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
        match parts[0] {
            "box" => app.st.run_action(Action::CreatePrimitive(crate::app::PrimKind::Box)),
            "cylinder" => app.st.run_action(Action::CreatePrimitive(crate::app::PrimKind::Cylinder)),
            "sphere" => app.st.run_action(Action::CreatePrimitive(crate::app::PrimKind::Sphere)),
            "torus" => app.st.run_action(Action::CreatePrimitive(crate::app::PrimKind::Torus)),
            "ok" => key(&mut app, crate::ui::Key::Enter),
            "esc" => key(&mut app, crate::ui::Key::Escape),
            "fit" => app.st.run_action(Action::Fit),
            "wire" => app.st.run_action(Action::CycleMode),
            "help" => app.st.run_action(Action::Help),
            "settings" => app.st.run_action(Action::Settings),
            "sketch" => app.st.run_action(Action::NewSketch),
            "extrude" => app.st.run_action(Action::Extrude),
            "info" => app.st.run_action(Action::ShowInfo),
            "iso" => app.st.run_action(Action::ViewHome),
            "move" => {
                app.event(Event::MouseMove { x: num(1), y: num(2) });
                app.frame();
            }
            "click" | "rclick" => {
                let b = if parts[0] == "click" { MouseButton::Left } else { MouseButton::Right };
                app.event(Event::MouseMove { x: num(1), y: num(2) });
                app.frame();
                app.event(Event::MouseButton { button: b, down: true });
                app.frame();
                app.event(Event::MouseButton { button: b, down: false });
                app.frame();
            }
            "drag" => {
                app.event(Event::MouseMove { x: num(1), y: num(2) });
                app.frame();
                app.event(Event::MouseButton { button: MouseButton::Left, down: true });
                app.frame();
                for k in 1..=6 {
                    let f = k as f32 / 6.0;
                    app.event(Event::MouseMove { x: num(1) + (num(3) - num(1)) * f, y: num(2) + (num(4) - num(2)) * f });
                    app.frame();
                }
                app.event(Event::MouseButton { button: MouseButton::Left, down: false });
                app.frame();
            }
            "action" => {
                let a = match parts.get(1).copied().unwrap_or("") {
                    "revolve" => Action::Revolve,
                    "sweep" => Action::Sweep,
                    "loft" => Action::Loft,
                    "shell" => Action::Shell,
                    "solidify" => Action::Solidify,
                    "pattern" => Action::Pattern,
                    "twist" => Action::Deform(0),
                    "taper" => Action::Deform(1),
                    "bend" => Action::Deform(2),
                    "subdivide" => Action::Subdivide,
                    "smooth" => Action::Smooth,
                    "coil" => Action::Coil,
                    "finish" => Action::FinishSketch,
                    "select-last" => {
                        app.st.scene.selected = app.st.scene.bodies.iter().rev().find(|b| !b.mesh.is_empty()).map(|b| b.id);
                        settle(&mut app);
                        continue;
                    }
                    other => return Err(format!("неизвестное действие: {other}")),
                };
                app.st.run_action(a);
            }
            "tool" => {
                use crate::app::Tool;
                let tl = match parts.get(1).copied().unwrap_or("") {
                    "line" => Tool::Line,
                    "rect" => Tool::Rectangle,
                    "circle" => Tool::Circle,
                    "arc" => Tool::Arc,
                    "polygon" => Tool::Polygon,
                    "ellipse" => Tool::Ellipse,
                    "slot" => Tool::Slot,
                    "spline" => Tool::Spline,
                    "fillet" => Tool::SketchFillet,
                    "offset" => Tool::SketchOffset,
                    other => return Err(format!("неизвестный инструмент: {other}")),
                };
                app.st.run_action(Action::SetTool(tl));
            }
            "demo" => demo_scene(&mut app, parts.get(1).copied().unwrap_or(""))?,
            "key" => {
                if let Some(c) = parts.get(1).and_then(|s| s.chars().next()) {
                    key(&mut app, crate::ui::Key::Char(c));
                }
            }
            "" => {}
            other => return Err(format!("неизвестный шаг сценария: {other}")),
        }
        settle(&mut app);
    }
    settle(&mut app);
    if app.st.cfg.gpu {
        match (&app.st.gpu, &app.st.gpu_error) {
            (Some(g), _) => println!("renderer: GPU {} ({:.1} ms)", g.info, app.st.last_render_ms),
            (None, e) => {
                let e = e.clone().unwrap_or_default();
                println!("renderer: CPU (GPU unavailable: {e})");
                if opts.require_gpu {
                    return Err(format!("GPU недоступен: {e}"));
                }
            }
        }
    } else {
        println!("renderer: CPU ({:.1} ms)", app.st.last_render_ms);
    }
    let fb = app.framebuffer();
    let png = crate::renderer::canvas::encode_png(fb.width, fb.height, &fb.to_rgb8());
    std::fs::write(output, png).map_err(|e| e.to_string())
}

/// Готовые эскизы для проверки операций моделирования в снимках.
fn demo_scene(app: &mut App, kind: &str) -> Result<(), String> {
    use crate::math::Vec3;
    use crate::sketch::{Sketch, SketchEntity, SketchPlane};
    let v = |x: f32, y: f32| Vec3::new(x, y, 0.0);
    let mut add = |name: &str, sk: Sketch| {
        let mut sk = sk;
        sk.name = name.to_string();
        let mut b = crate::scene::Body::new(name, crate::mesh::Mesh::new());
        b.sketch = Some(sk);
        app.st.scene.add_body(b)
    };
    match kind {
        // Профиль бокала на XZ и ось — отрезок по оси Z.
        "revolve" => {
            let mut sk = Sketch::new(SketchPlane::Xz);
            let pts = [(0.0, 0.0), (14.0, 0.0), (14.0, 2.0), (3.0, 3.0), (2.0, 18.0), (12.0, 26.0), (13.0, 40.0), (11.0, 40.0), (10.0, 28.0), (0.0, 22.0)];
            for i in 0..pts.len() {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                sk.entities.push(SketchEntity::Line { a: v(a.0, a.1), b: v(b.0, b.1) });
            }
            add("Профиль", sk);
        }
        // Профиль — окружность на YZ, траектория — сплайн на XY.
        "sweep" => {
            let mut p = Sketch::new(SketchPlane::Yz);
            p.entities.push(SketchEntity::Circle { center: v(0.0, 0.0), radius: 3.0 });
            add("Профиль", p);
            let mut path = Sketch::new(SketchPlane::Xy);
            path.entities.push(SketchEntity::Spline { points: vec![v(0.0, 0.0), v(20.0, 12.0), v(40.0, -12.0), v(60.0, 0.0)], closed: false });
            add("Траектория", path);
        }
        // Окружность внизу, шестиугольник на высоте 30.
        "loft" => {
            let mut a = Sketch::new(SketchPlane::Xy);
            a.entities.push(SketchEntity::Circle { center: v(0.0, 0.0), radius: 15.0 });
            add("Сечение 1", a);
            let mut b = Sketch::new(SketchPlane::offset(SketchPlane::Xy, 30.0));
            b.entities.push(SketchEntity::Polygon { center: v(0.0, 0.0), radius: 8.0, sides: 6, rotation: 0.0 });
            add("Сечение 2", b);
        }
        // Все новые элементы эскиза.
        "shapes" => {
            let mut sk = Sketch::new(SketchPlane::Xy);
            sk.entities.push(SketchEntity::Polygon { center: v(-30.0, 15.0), radius: 10.0, sides: 6, rotation: 0.0 });
            sk.entities.push(SketchEntity::Ellipse { center: v(0.0, 15.0), rx: 12.0, ry: 6.0, rotation: 0.3 });
            sk.entities.push(SketchEntity::Slot { a: v(20.0, 15.0), b: v(40.0, 15.0), radius: 5.0 });
            sk.entities.push(SketchEntity::Spline { points: vec![v(-40.0, -15.0), v(-25.0, -5.0), v(-10.0, -25.0), v(0.0, -10.0)], closed: false });
            sk.entities.push(SketchEntity::arc_3pt(crate::math::Vec2::new(10.0, -20.0), crate::math::Vec2::new(30.0, -20.0), crate::math::Vec2::new(20.0, -10.0)).unwrap());
            sk.entities.push(SketchEntity::Rect { min: v(32.0, -25.0), max: v(45.0, -10.0) });
            sk.fillet_corner(crate::math::Vec2::new(45.0, -10.0), 4.0, 0.01)?;
            sk.offset_at(crate::math::Vec2::new(-20.0, 15.0), 1.0, 2.0)?;
            add("Эскиз", sk);
        }
        other => return Err(format!("неизвестная демо-сцена: {other}")),
    }
    app.st.invalidate_view();
    Ok(())
}

fn key(app: &mut App, k: crate::ui::Key) {
    app.event(Event::Key { key: k, mods: Default::default(), down: true });
    app.frame();
    app.event(Event::Key { key: k, mods: Default::default(), down: false });
}

/// Прогон кадров, пока идут анимации (не более 60).
fn settle(app: &mut App) {
    for _ in 0..60 {
        let out = app.frame();
        match out.wake_ms {
            Some(ms) if ms < 100 => std::thread::sleep(std::time::Duration::from_millis(ms.min(16))),
            _ => break,
        }
    }
    app.frame();
}
