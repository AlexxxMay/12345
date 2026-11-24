//! Приложение ForgeCAD: состояние, конфигурация, локализация и цикл кадра.
//!
//! Раскладка в духе Fusion 360: верхняя панель приложения, лента инструментов
//! с вкладками, плавающий браузер слева, ViewCube справа сверху, панель
//! свойств/команд справа, панель навигации снизу по центру и временная шкала
//! истории внизу окна.

mod actions;
mod dialogs;
mod panels;
mod view;

use crate::math::{Aabb, Color, Mat4, Vec2, Vec3};
use crate::mesh::Primitive;
use crate::platform::{Event, FrameOutput, Handler};
use crate::renderer::canvas::{Framebuffer, Rect};
use crate::renderer::software::{Camera, DisplayMesh, DisplayMode};
use crate::scene::Scene;
use crate::sketch::Sketch;
use crate::ui::font::Font;
use crate::ui::{Input, Ui, UiState};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Instant;

pub use actions::{Action, ExportKind, PrimKind};
pub use dialogs::Dialog;

// ---------------------------------------------------------------------------
// Локализация
// ---------------------------------------------------------------------------

static LANG: AtomicU8 = AtomicU8::new(0);

/// Выбор строки по текущему языку интерфейса (русский / английский).
pub fn t(ru: &'static str, en: &'static str) -> &'static str {
    if LANG.load(Ordering::Relaxed) == 1 {
        en
    } else {
        ru
    }
}

/// Текущий язык — английский?
pub fn lang_en() -> bool {
    LANG.load(Ordering::Relaxed) == 1
}

pub fn set_lang_en(en: bool) {
    LANG.store(if en { 1 } else { 0 }, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// Конфигурация
// ---------------------------------------------------------------------------

/// Пользовательские настройки (сохраняются в `config.ini`).
#[derive(Clone, Debug)]
pub struct Config {
    pub lang_en: bool,
    pub ui_scale: f32,
    /// Пониженное разрешение 3D во время навигации (для слабых ПК).
    pub fast_nav: bool,
    /// Масштаб разрешения 3D-вида (1.0 / 0.75 / 0.5).
    pub render_scale: f32,
    /// Число потоков рендера (0 — авто).
    pub threads: usize,
    pub show_grid: bool,
    pub mode: DisplayMode,
    pub snap: bool,
    pub crease_deg: f32,
    pub recent: Vec<String>,
    pub dark_bg: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            lang_en: !crate::platform::system_prefers_russian(),
            ui_scale: 1.0,
            fast_nav: true,
            render_scale: 1.0,
            threads: 0,
            show_grid: true,
            mode: DisplayMode::ShadedEdges,
            snap: true,
            crease_deg: 35.0,
            recent: Vec::new(),
            dark_bg: false,
        }
    }
}

impl Config {
    fn path() -> PathBuf {
        crate::platform::config_dir().join("config.ini")
    }

    pub fn load() -> Config {
        let mut c = Config::default();
        let Ok(text) = std::fs::read_to_string(Self::path()) else { return c };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "lang" => c.lang_en = v == "en",
                "ui_scale" => c.ui_scale = v.parse().unwrap_or(1.0f32).clamp(0.75, 2.0),
                "fast_nav" => c.fast_nav = v == "1",
                "render_scale" => c.render_scale = v.parse().unwrap_or(1.0f32).clamp(0.25, 1.0),
                "threads" => c.threads = v.parse().unwrap_or(0),
                "grid" => c.show_grid = v == "1",
                "snap" => c.snap = v == "1",
                "dark_bg" => c.dark_bg = v == "1",
                "crease" => c.crease_deg = v.parse().unwrap_or(35.0f32).clamp(1.0, 89.0),
                "mode" => {
                    c.mode = match v {
                        "shaded" => DisplayMode::Shaded,
                        "wire" => DisplayMode::Wireframe,
                        _ => DisplayMode::ShadedEdges,
                    }
                }
                "recent" => {
                    if !v.is_empty() && c.recent.len() < 10 {
                        c.recent.push(v.to_string());
                    }
                }
                _ => {}
            }
        }
        c
    }

    pub fn save(&self) {
        let mut s = String::new();
        s.push_str(&format!("lang={}\n", if self.lang_en { "en" } else { "ru" }));
        s.push_str(&format!("ui_scale={}\n", self.ui_scale));
        s.push_str(&format!("fast_nav={}\n", self.fast_nav as u8));
        s.push_str(&format!("render_scale={}\n", self.render_scale));
        s.push_str(&format!("threads={}\n", self.threads));
        s.push_str(&format!("grid={}\n", self.show_grid as u8));
        s.push_str(&format!("snap={}\n", self.snap as u8));
        s.push_str(&format!("dark_bg={}\n", self.dark_bg as u8));
        s.push_str(&format!("crease={}\n", self.crease_deg));
        s.push_str(&format!(
            "mode={}\n",
            match self.mode {
                DisplayMode::Shaded => "shaded",
                DisplayMode::Wireframe => "wire",
                DisplayMode::ShadedEdges => "edges",
            }
        ));
        for r in &self.recent {
            s.push_str(&format!("recent={r}\n"));
        }
        let _ = std::fs::write(Self::path(), s);
    }

    pub fn add_recent(&mut self, p: &std::path::Path) {
        let s = p.to_string_lossy().to_string();
        self.recent.retain(|r| r != &s);
        self.recent.insert(0, s);
        self.recent.truncate(8);
    }
}

// ---------------------------------------------------------------------------
// Инструменты и состояние
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Select,
    Move,
    Rotate,
    Scale,
    Measure,
    Line,
    Rectangle,
    Circle,
}

impl Tool {
    pub fn is_sketch(&self) -> bool {
        matches!(self, Tool::Line | Tool::Rectangle | Tool::Circle)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    Solid,
    Sketch,
    Mesh,
    Inspect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NavMode {
    Default,
    Orbit,
    Pan,
    Zoom,
}

/// Активное перетаскивание в 3D-виде.
#[derive(Clone, Copy, Debug)]
pub enum Drag {
    /// Кнопка нажата, движение ещё не началось.
    Pending { button: usize, start: (f32, f32) },
    Orbit,
    Pan,
    Zoom { anchor: (f32, f32) },
    Gizmo(view::GizmoDrag),
}

/// Сеанс редактирования эскиза.
#[derive(Clone, Debug)]
pub struct SketchSession {
    /// Редактируемое тело-эскиз (если эскиз уже существовал).
    pub body_id: Option<u64>,
    pub sketch: Sketch,
    /// Первая точка текущей операции (начало линии / угол / центр).
    pub anchor: Option<Vec2>,
    /// Начало цепочки линий (для замыкания).
    pub chain_start: Option<Vec2>,
    /// Точка под курсором (с привязкой).
    pub hover: Option<Vec2>,
    pub hover_snapped_to_point: bool,
    pub prev_camera: Camera,
}

/// Состояние инструмента измерения.
#[derive(Clone, Debug, Default)]
pub struct MeasureState {
    pub a: Option<Vec3>,
    pub b: Option<Vec3>,
}

/// Уведомление в правом нижнем углу.
#[derive(Clone, Debug)]
pub struct Toast {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

/// Открытое меню.
#[derive(Clone, Debug)]
pub struct Menu {
    pub x: f32,
    pub y: f32,
    pub items: Vec<MenuItem>,
    /// Прямоугольник кнопки, открывшей меню (клик по ней не закрывает меню повторно).
    pub opener: Rect,
}

#[derive(Clone, Debug)]
pub struct MenuItem {
    pub icon: Option<crate::ui::icons::Icon>,
    pub label: String,
    pub shortcut: String,
    pub action: Option<Action>,
    pub enabled: bool,
}

impl MenuItem {
    pub fn new(icon: Option<crate::ui::icons::Icon>, label: &str, shortcut: &str, action: Action) -> Self {
        Self { icon, label: label.to_string(), shortcut: shortcut.to_string(), action: Some(action), enabled: true }
    }
    pub fn sep() -> Self {
        Self { icon: None, label: String::new(), shortcut: String::new(), action: None, enabled: false }
    }
    pub fn enabled(mut self, e: bool) -> Self {
        self.enabled = e;
        self
    }
}

/// Запись кэша отображаемых мешей.
struct CacheEntry {
    fp: u64,
    flat: bool,
    crease: u32,
    dm: Arc<DisplayMesh>,
    used: u64,
}

/// Раскладка окна на текущий кадр (в физических пикселях).
#[derive(Clone, Copy, Debug, Default)]
pub struct Layout {
    pub appbar: Rect,
    pub toolbar: Rect,
    pub view: Rect,
    pub timeline: Rect,
    pub browser: Rect,
    pub right: Rect,
    pub cube: Rect,
    pub navbar: Rect,
}

/// Всё состояние приложения, кроме кадрового буфера, шрифта и ввода.
pub struct State {
    pub scene: Scene,
    pub cam: Camera,
    cam_anim: Option<(Camera, Camera, Instant, f32)>,
    pub cfg: Config,
    pub scale: f32,
    pub layout: Layout,
    view_fb: Framebuffer,
    view_key: u64,
    cache: HashMap<u64, CacheEntry>,
    cache_tick: u64,
    pub tool: Tool,
    pub tab: Tab,
    pub nav_mode: NavMode,
    pub dialog: Option<Dialog>,
    pub menu: Option<Menu>,
    pub hover_body: Option<u64>,
    pub drag: Option<Drag>,
    pub gizmo_hover: Option<usize>,
    pub sketch: Option<SketchSession>,
    pub measure: MeasureState,
    pub toasts: Vec<Toast>,
    pub project_path: Option<PathBuf>,
    pub saved_revision: u64,
    pub quit: bool,
    pub pending_open: Vec<PathBuf>,
    pub browser_open: bool,
    pub last_render_ms: f32,
    pub last_tris: usize,
    pub navigating_until: Option<Instant>,
    pub rename: Option<(u64, String)>,
    pub wake_ms: Option<u64>,
    pub status: String,
    pub frame_no: u64,
    pub mouse_cache: (f32, f32),
    preview_cache: Option<(u64, Arc<DisplayMesh>, Mat4)>,
    fill_cache: HashMap<u64, Arc<DisplayMesh>>,
    info_cache: Option<(u64, u64, BodyInfo)>,
}

/// Сведения о теле (кэшируются: объём и площадь дороги для больших мешей).
#[derive(Clone, Debug, Default)]
pub struct BodyInfo {
    pub tris: usize,
    pub verts: usize,
    pub size: Vec3,
    pub volume: f32,
    pub area: f32,
    pub closed: bool,
}

/// Приложение целиком (реализует [`Handler`]).
pub struct App {
    fb: Framebuffer,
    font: Font,
    input: Input,
    ui: UiState,
    pub st: State,
}

impl App {
    pub fn new(files: Vec<PathBuf>) -> App {
        let cfg = Config::load();
        set_lang_en(cfg.lang_en);
        let st = State {
            scene: Scene::new(),
            cam: Camera::default(),
            cam_anim: None,
            cfg,
            scale: 1.0,
            layout: Layout::default(),
            view_fb: Framebuffer::new(1, 1),
            view_key: 0,
            cache: HashMap::new(),
            cache_tick: 0,
            tool: Tool::Select,
            tab: Tab::Solid,
            nav_mode: NavMode::Default,
            dialog: None,
            menu: None,
            hover_body: None,
            drag: None,
            gizmo_hover: None,
            sketch: None,
            measure: MeasureState::default(),
            toasts: Vec::new(),
            project_path: None,
            saved_revision: 0,
            quit: false,
            pending_open: files,
            browser_open: true,
            last_render_ms: 0.0,
            last_tris: 0,
            navigating_until: None,
            rename: None,
            wake_ms: None,
            status: String::new(),
            frame_no: 0,
            mouse_cache: (0.0, 0.0),
            preview_cache: None,
            fill_cache: HashMap::new(),
            info_cache: None,
        };
        App { fb: Framebuffer::new(1280, 800), font: Font::builtin(), input: Input::default(), ui: UiState::default(), st }
    }

    /// Доступ к кадровому буферу (для скриншотов в CLI/тестах).
    pub fn fb(&self) -> &Framebuffer {
        &self.fb
    }
}

impl Handler for App {
    fn event(&mut self, e: Event) {
        match e {
            Event::Resize { width, height, scale } => {
                self.fb.resize(width, height);
                self.st.scale = scale.clamp(0.5, 4.0);
                self.st.view_key = 0;
            }
            Event::MouseMove { x, y } => self.input.on_mouse_move(x, y),
            Event::MouseButton { button, down } => self.input.on_button(button, down),
            Event::Wheel { delta } => self.input.wheel += delta,
            Event::Key { key, mods, down } => {
                self.input.mods = mods;
                if down {
                    self.input.on_key(key, mods);
                }
            }
            Event::Text(s) => self.input.on_text(&s),
            Event::Modifiers(m) => self.input.mods = m,
            Event::DropFiles(files) => self.st.pending_open.extend(files),
            Event::MouseLeave => {
                self.input.mouse_inside = false;
                self.input.mouse = (-1000.0, -1000.0);
            }
            Event::CloseRequested => self.st.request_quit(),
        }
    }

    fn frame(&mut self) -> FrameOutput {
        let s = self.st.scale * self.st.cfg.ui_scale;
        self.st.wake_ms = None;
        let mut ui = Ui::new(&mut self.fb, &self.font, &self.input, &mut self.ui, s);
        self.st.frame(&mut ui);
        let cursor = ui.st.cursor;
        let mut wake = ui.st.wake_ms;
        ui.finish();
        if let Some(w2) = self.ui.wake_ms {
            wake = Some(wake.map_or(w2, |w| w.min(w2)));
        }
        if let Some(w2) = self.st.wake_ms {
            wake = Some(wake.map_or(w2, |w| w.min(w2)));
        }
        self.input.end_frame();
        FrameOutput { title: self.st.title(), cursor, quit: self.st.quit, wake_ms: wake }
    }

    fn framebuffer(&self) -> &Framebuffer {
        &self.fb
    }
}

// ---------------------------------------------------------------------------
// Вспомогательные функции трансформаций
// ---------------------------------------------------------------------------

/// Разложение матрицы на перенос, углы Эйлера (градусы, XYZ) и масштаб.
pub fn decompose(m: &Mat4) -> (Vec3, Vec3, Vec3) {
    let t = m.translation_part();
    let c0 = Vec3::new(m.get(0, 0), m.get(0, 1), m.get(0, 2));
    let c1 = Vec3::new(m.get(1, 0), m.get(1, 1), m.get(1, 2));
    let c2 = Vec3::new(m.get(2, 0), m.get(2, 1), m.get(2, 2));
    let mut s = Vec3::new(c0.len(), c1.len(), c2.len());
    if m.determinant() < 0.0 {
        s.x = -s.x;
    }
    let sx = if s.x.abs() > 1e-9 { s.x } else { 1.0 };
    let sy = if s.y.abs() > 1e-9 { s.y } else { 1.0 };
    let sz = if s.z.abs() > 1e-9 { s.z } else { 1.0 };
    // R(row, col): столбцы нормализованы.
    let r = |row: usize, col: usize| -> f32 {
        let c = match col {
            0 => c0 * (1.0 / sx),
            1 => c1 * (1.0 / sy),
            _ => c2 * (1.0 / sz),
        };
        c.get(row)
    };
    // R = Rx(a) · Ry(b) · Rz(c)
    let b = r(0, 2).clamp(-1.0, 1.0).asin();
    let (a, c) = if b.cos().abs() > 1e-5 {
        ((-r(1, 2)).atan2(r(2, 2)), (-r(0, 1)).atan2(r(0, 0)))
    } else {
        (r(2, 1).atan2(r(1, 1)), 0.0)
    };
    let deg = |x: f32| {
        let d = x.to_degrees();
        if d.abs() < 1e-4 {
            0.0
        } else {
            d
        }
    };
    (t, Vec3::new(deg(a), deg(b), deg(c)), s)
}

/// Сборка матрицы: перенос · поворот (XYZ, градусы) · масштаб.
pub fn compose(t: Vec3, e: Vec3, s: Vec3) -> Mat4 {
    Mat4::translation(t) * Mat4::rotation_euler_deg(e) * Mat4::scale(s)
}

/// Палитра материалов.
pub const PALETTE: [u32; 10] = [
    0xB8C0CC, 0x8FA7C4, 0x5C88C9, 0x4FA36B, 0xE0B04A, 0xE07B39, 0xC9504E, 0x9B6FC4, 0x3D4450, 0xF2F2F2,
];

pub fn color_u32(c: Color) -> u32 {
    ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32
}

pub fn u32_color(c: u32) -> Color {
    Color::rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

// ---------------------------------------------------------------------------
// Кадр
// ---------------------------------------------------------------------------

impl State {
    pub fn px(&self, v: f32) -> f32 {
        (v * self.scale * self.cfg.ui_scale).round()
    }

    pub fn title(&self) -> String {
        let name = self
            .project_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| t("Без имени", "Untitled").to_string());
        let dirty = if self.is_dirty() { " •" } else { "" };
        format!("{name}{dirty} — ForgeCAD {}", crate::VERSION)
    }

    pub fn is_dirty(&self) -> bool {
        self.scene.revision != self.saved_revision && !self.scene.bodies.is_empty()
    }

    pub fn request_quit(&mut self) {
        if self.is_dirty() {
            self.dialog = Some(Dialog::ConfirmQuit);
        } else {
            self.cfg.save();
            self.quit = true;
        }
    }

    pub fn toast(&mut self, text: impl Into<String>, error: bool) {
        let text = text.into();
        if error {
            eprintln!("{text}");
        }
        self.toasts.push(Toast { text, error, at: Instant::now() });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    fn compute_layout(&mut self, w: f32, h: f32) {
        let appbar_h = self.px(38.0);
        let toolbar_h = self.px(92.0);
        let timeline_h = self.px(36.0);
        let appbar = Rect::new(0.0, 0.0, w, appbar_h);
        let toolbar = Rect::new(0.0, appbar_h, w, toolbar_h);
        let vy = appbar_h + toolbar_h;
        let view = Rect::new(0.0, vy, w, (h - vy - timeline_h).max(10.0));
        let timeline = Rect::new(0.0, view.bottom(), w, timeline_h);
        let m = self.px(10.0);
        let bw = self.px(250.0).min(w * 0.3);
        let browser = Rect::new(m, view.y + m, bw, (view.h - self.px(80.0)).max(self.px(60.0)));
        let cs = self.px(108.0);
        let cube = Rect::new(view.right() - cs - m, view.y + m * 0.5, cs, cs);
        let rw = self.px(296.0).min(w * 0.36);
        let right = Rect::new(view.right() - rw - m, cube.bottom() + m * 0.5, rw, (view.bottom() - cube.bottom() - self.px(70.0)).max(self.px(80.0)));
        let nw = self.px(360.0);
        let nh = self.px(40.0);
        let navbar = Rect::new(view.x + (view.w - nw) * 0.5, view.bottom() - nh - m, nw, nh);
        self.layout = Layout { appbar, toolbar, view, timeline, browser, right, cube, navbar };
    }

    /// Построение и отрисовка кадра.
    pub fn frame(&mut self, ui: &mut Ui) {
        self.frame_no += 1;
        let (w, h) = (ui.fb.width as f32, ui.fb.height as f32);
        self.compute_layout(w, h);

        // Файлы из командной строки / перетаскивания.
        if !self.pending_open.is_empty() {
            let files = std::mem::take(&mut self.pending_open);
            self.open_files(files, true);
        }

        // Анимация камеры.
        if let Some((from, to, start, dur)) = self.cam_anim {
            let k = (start.elapsed().as_secs_f32() / dur).min(1.0);
            let e = k * k * (3.0 - 2.0 * k);
            let mut c = to;
            c.target = from.target.lerp(to.target, e);
            c.distance = from.distance + (to.distance - from.distance) * e;
            let mut dy = to.yaw - from.yaw;
            while dy > std::f32::consts::PI {
                dy -= 2.0 * std::f32::consts::PI;
            }
            while dy < -std::f32::consts::PI {
                dy += 2.0 * std::f32::consts::PI;
            }
            c.yaw = from.yaw + dy * e;
            c.pitch = from.pitch + (to.pitch - from.pitch) * e;
            self.cam = c;
            if k >= 1.0 {
                self.cam_anim = None;
            } else {
                self.wake_ms = Some(15);
            }
        }

        self.shortcuts(ui);
        self.viewport_input(ui);
        self.draw_view(ui);
        self.draw_view_overlays(ui);
        self.draw_appbar(ui);
        self.draw_toolbar(ui);
        self.draw_timeline(ui);
        if self.browser_open {
            self.draw_browser(ui);
        } else {
            self.draw_browser_collapsed(ui);
        }
        if self.dialog.as_ref().map(|d| d.is_side_panel()).unwrap_or(false) {
            self.draw_side_dialog(ui);
        } else if self.sketch.is_none() {
            self.draw_properties(ui);
        }
        self.draw_sketch_panel(ui);
        self.draw_navbar(ui);
        self.draw_viewcube(ui);
        self.draw_toasts(ui);
        self.draw_modal_dialog(ui);
        self.draw_menu(ui);
        ui.draw_popups();
        ui.draw_tooltip();
    }

    /// Глобальные горячие клавиши.
    fn shortcuts(&mut self, ui: &mut Ui) {
        if ui.st.keyboard_taken {
            return;
        }
        let mut act: Option<Action> = None;
        if ui.shortcut('n') {
            act = Some(Action::New);
        } else if ui.shortcut('o') {
            act = Some(Action::Open);
        } else if ui.shortcut('i') {
            act = Some(Action::Import);
        } else if ui.shortcut('s') {
            act = Some(Action::Save);
        } else if ui.shortcut_shift('s') {
            act = Some(Action::SaveAs);
        } else if ui.shortcut('e') {
            act = Some(Action::Export(actions::ExportKind::StlBinary));
        } else if ui.shortcut('z') {
            act = Some(Action::Undo);
        } else if ui.shortcut('y') || ui.shortcut_shift('z') {
            act = Some(Action::Redo);
        } else if ui.shortcut('d') {
            act = Some(Action::Duplicate);
        } else if ui.shortcut('q') {
            act = Some(Action::Quit);
        } else if ui.key(crate::ui::Key::Delete) || (cfg!(target_os = "macos") && ui.key(crate::ui::Key::Backspace)) {
            if self.sketch.is_none() {
                act = Some(Action::Delete);
            }
        } else if ui.key(crate::ui::Key::Escape) {
            act = Some(Action::Escape);
        } else if ui.key(crate::ui::Key::Enter) && self.sketch.is_some() && self.dialog.is_none() {
            act = Some(Action::FinishSketch);
        } else if ui.key(crate::ui::Key::F(1)) {
            act = Some(Action::Help);
        } else if ui.key(crate::ui::Key::Home) {
            act = Some(Action::ViewHome);
        } else if self.dialog.is_none() {
            let sk = self.sketch.is_some();
            for (c, a) in [
                ('f', Action::Fit),
                ('g', Action::ToggleGrid),
                ('w', Action::CycleMode),
                ('p', Action::ToggleOrtho),
                ('i', Action::SetTool(Tool::Measure)),
                ('v', Action::SetTool(Tool::Select)),
                ('m', Action::SetTool(Tool::Move)),
                ('e', Action::Extrude),
                ('1', Action::SetView(view::ViewDir::Front)),
                ('3', Action::SetView(view::ViewDir::Right)),
                ('7', Action::SetView(view::ViewDir::Top)),
                ('0', Action::ViewHome),
            ] {
                if ui.plain_key(c) {
                    act = Some(a);
                }
            }
            if sk {
                if ui.plain_key('l') {
                    act = Some(Action::SetTool(Tool::Line));
                } else if ui.plain_key('r') {
                    act = Some(Action::SetTool(Tool::Rectangle));
                } else if ui.plain_key('c') {
                    act = Some(Action::SetTool(Tool::Circle));
                }
            } else if ui.plain_key('r') {
                act = Some(Action::SetTool(Tool::Rotate));
            } else if ui.plain_key('s') {
                act = Some(Action::SetTool(Tool::Scale));
            }
        }
        if let Some(a) = act {
            self.run_action(a);
        }
    }

    /// Плавный переход камеры.
    pub fn animate_camera(&mut self, to: Camera) {
        self.cam_anim = Some((self.cam, to, Instant::now(), 0.35));
        self.wake_ms = Some(0);
    }

    /// Общие габариты видимых тел (включая эскизы).
    pub fn scene_bounds(&self) -> Aabb {
        let mut b = Aabb::empty();
        for body in &self.scene.bodies {
            if !body.visible {
                continue;
            }
            b.union(&body.world_aabb());
            if let Some(sk) = &body.sketch {
                for p in sk.points() {
                    b.include(body.transform.transform_point(sk.plane.to_world(Vec2::new(p.x, p.y))));
                }
            }
        }
        b
    }

    /// Отображаемый меш тела из кэша (строится при изменении).
    pub fn display_mesh(&mut self, body_idx: usize) -> Option<Arc<DisplayMesh>> {
        let body = self.scene.bodies.get(body_idx)?;
        if body.mesh.is_empty() {
            return None;
        }
        let fp = crate::renderer::software::mesh_fingerprint(&body.mesh);
        let flat = body.material.shading == 0;
        let crease = self.cfg.crease_deg as u32;
        self.cache_tick += 1;
        let tick = self.cache_tick;
        if let Some(e) = self.cache.get_mut(&body.id) {
            if e.fp == fp && e.flat == flat && e.crease == crease {
                e.used = tick;
                return Some(e.dm.clone());
            }
        }
        let dm = Arc::new(DisplayMesh::build(&body.mesh, flat, crease as f32));
        self.cache.insert(body.id, CacheEntry { fp, flat, crease, dm: dm.clone(), used: tick });
        // Очистка записей удалённых тел.
        if self.cache.len() > self.scene.bodies.len() + 8 {
            let ids: std::collections::HashSet<u64> = self.scene.bodies.iter().map(|b| b.id).collect();
            self.cache.retain(|k, _| ids.contains(k));
        }
        Some(dm)
    }

    /// Пометить 3D-вид для перерисовки.
    pub fn invalidate_view(&mut self) {
        self.view_key = 0;
    }

    /// Сброс сцены в новый документ.
    pub fn reset_document(&mut self) {
        self.scene = Scene::new();
        self.project_path = None;
        self.saved_revision = self.scene.revision;
        self.sketch = None;
        self.dialog = None;
        self.tool = Tool::Select;
        self.measure = MeasureState::default();
        self.cache.clear();
        self.cam = Camera::default();
        self.invalidate_view();
    }

    /// Имя нового тела без повторов.
    pub fn unique_name(&self, base: &str) -> String {
        let mut n = 1;
        loop {
            let name = format!("{base} {n}");
            if !self.scene.bodies.iter().any(|b| b.name == name) {
                return name;
            }
            n += 1;
        }
    }

    /// Название примитива на текущем языке.
    pub fn primitive_name(p: &Primitive) -> &'static str {
        match p {
            Primitive::Box { .. } => t("Параллелепипед", "Box"),
            Primitive::Cylinder { .. } => t("Цилиндр", "Cylinder"),
            Primitive::Sphere { .. } => t("Сфера", "Sphere"),
            Primitive::Cone { .. } => t("Конус", "Cone"),
            Primitive::Torus { .. } => t("Тор", "Torus"),
            Primitive::Capsule { .. } => t("Капсула", "Capsule"),
            Primitive::Tube { .. } => t("Труба", "Pipe"),
            Primitive::Wedge { .. } => t("Клин", "Wedge"),
            Primitive::Pyramid { .. } => t("Пирамида", "Pyramid"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompose_roundtrip() {
        let t = Vec3::new(1.0, -2.0, 3.0);
        let e = Vec3::new(10.0, 20.0, 30.0);
        let s = Vec3::new(2.0, 1.5, 0.5);
        let m = compose(t, e, s);
        let (t2, e2, s2) = decompose(&m);
        assert!(t2.distance(t) < 1e-4);
        assert!(e2.distance(e) < 1e-2, "{e2:?}");
        assert!(s2.distance(s) < 1e-4);
    }
}
