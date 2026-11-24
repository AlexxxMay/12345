//! Командная строка.
//!
//! ```text
//! alphacad [файлы…]                         — запуск редактора
//! alphacad info <файл>                      — сведения о модели
//! alphacad convert <вход> <выход> [--ascii] — конвертация форматов
//! alphacad snapshot <выход.png> [файлы…] [--size WxH] [--lang ru|en] [--theme light|dark]
//!          [--render cpu|gpu] [--require-gpu] [--script a,b,…]
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
           [--theme light|dark] [--render cpu|gpu] [--require-gpu]
                                             снимок интерфейса без окна
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
/// `drag:X1:Y1:X2:Y2`, `key:c`.
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
