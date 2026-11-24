//! Immediate-mode интерфейс: ввод, слои, виджеты, тема и иконки.
//!
//! Каждый кадр интерфейс полностью перестраивается; состояние виджетов
//! (фокус, активный элемент, редактируемый текст) хранится в [`UiState`].
//! Слои (база, панели, всплывающие меню, модальные окна) не дают кликам
//! «проваливаться» под меню: используется карта слоёв предыдущего кадра.

pub mod font;
pub mod icons;
pub mod widgets;

use crate::renderer::canvas::{Framebuffer, Rect};
use font::Font;
use std::time::Instant;

pub use widgets::*;

/// Клавиши, которые важны для интерфейса и горячих клавиш.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Escape,
    Enter,
    Tab,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Space,
    F(u8),
    /// Буква или цифра (в нижнем регистре, латиница по физической клавише).
    Char(char),
}

/// Модификаторы.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// Command (macOS) / Win.
    pub meta: bool,
}

impl Mods {
    /// Основной модификатор горячих клавиш: Cmd на macOS, Ctrl на остальных.
    pub fn primary(&self) -> bool {
        if cfg!(target_os = "macos") {
            self.meta || self.ctrl
        } else {
            self.ctrl
        }
    }
}

/// Кнопки мыши.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left = 0,
    Right = 1,
    Middle = 2,
}

/// Форма курсора.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Cursor {
    #[default]
    Arrow,
    Hand,
    Text,
    Crosshair,
    Move,
}

/// Состояние ввода за кадр.
#[derive(Clone, Debug, Default)]
pub struct Input {
    pub mouse: (f32, f32),
    pub mouse_prev: (f32, f32),
    pub down: [bool; 3],
    pub pressed: [bool; 3],
    pub released: [bool; 3],
    pub double_click: bool,
    pub wheel: f32,
    pub keys: Vec<(Key, Mods)>,
    pub text: String,
    pub mods: Mods,
    pub mouse_inside: bool,
    last_click: Option<(Instant, (f32, f32))>,
}

impl Input {
    pub fn on_mouse_move(&mut self, x: f32, y: f32) {
        self.mouse = (x, y);
        self.mouse_inside = true;
    }
    pub fn on_button(&mut self, b: MouseButton, down: bool) {
        let i = b as usize;
        if down {
            if !self.down[i] {
                self.pressed[i] = true;
            }
            if b == MouseButton::Left {
                let now = Instant::now();
                if let Some((t, p)) = self.last_click {
                    let d = ((p.0 - self.mouse.0).powi(2) + (p.1 - self.mouse.1).powi(2)).sqrt();
                    if now.duration_since(t).as_millis() < 400 && d < 6.0 {
                        self.double_click = true;
                        self.last_click = None;
                    } else {
                        self.last_click = Some((now, self.mouse));
                    }
                } else {
                    self.last_click = Some((now, self.mouse));
                }
            }
        } else if self.down[i] {
            self.released[i] = true;
        }
        self.down[i] = down;
    }
    pub fn on_key(&mut self, k: Key, mods: Mods) {
        self.mods = mods;
        self.keys.push((k, mods));
    }
    pub fn on_text(&mut self, s: &str) {
        for c in s.chars() {
            if !c.is_control() {
                self.text.push(c);
            }
        }
    }
    /// Сброс событий кадра (после отрисовки).
    pub fn end_frame(&mut self) {
        self.pressed = [false; 3];
        self.released = [false; 3];
        self.double_click = false;
        self.wheel = 0.0;
        self.keys.clear();
        self.text.clear();
        self.mouse_prev = self.mouse;
    }
    pub fn key_pressed(&self, k: Key) -> bool {
        self.keys.iter().any(|(kk, _)| *kk == k)
    }
    pub fn any_down(&self) -> bool {
        self.down.iter().any(|&d| d)
    }
}

/// Сохраняемое между кадрами состояние интерфейса.
#[derive(Debug)]
pub struct UiState {
    /// Активный (зажатый) элемент.
    pub active: u64,
    /// Элемент под курсором в текущем кадре.
    pub hot: u64,
    prev_hot: u64,
    hot_since: Option<Instant>,
    /// Элемент с фокусом ввода текста.
    pub focus: u64,
    pub edit_buf: String,
    pub edit_cursor: usize,
    pub edit_select_all: bool,
    /// Фокус был снят в этом кадре (для применения значения).
    pub committed: Option<u64>,
    /// Слои прямоугольников прошлого кадра: (rect, layer).
    prev_layers: Vec<(Rect, u8)>,
    cur_layers: Vec<(Rect, u8)>,
    /// Текущий слой рисования.
    pub layer: u8,
    pub tooltip: Option<String>,
    pub tooltip_ready: bool,
    pub cursor: Cursor,
    /// Требуется повторная отрисовка через N мс (например, для подсказки).
    pub wake_ms: Option<u64>,
    /// Ввод с клавиатуры поглощён полем.
    pub keyboard_taken: bool,
    /// Курсор над элементом интерфейса (не над 3D-видом).
    pub mouse_over_ui: bool,
    /// Номер кадра (для мигания курсора и т.п.).
    pub frame: u64,
    pub scroll: std::collections::HashMap<u64, f32>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            active: 0,
            hot: 0,
            prev_hot: 0,
            hot_since: None,
            focus: 0,
            edit_buf: String::new(),
            edit_cursor: 0,
            edit_select_all: false,
            committed: None,
            prev_layers: Vec::new(),
            cur_layers: Vec::new(),
            layer: 0,
            tooltip: None,
            tooltip_ready: false,
            cursor: Cursor::Arrow,
            wake_ms: None,
            keyboard_taken: false,
            mouse_over_ui: false,
            frame: 0,
            scroll: Default::default(),
        }
    }
}

/// Тема оформления: светлая (по умолчанию) или тёмная «графит».
///
/// Цвета — функции, а не константы: тема переключается во время работы.
#[allow(non_snake_case)]
pub mod theme {
    use crate::renderer::canvas::rgb;
    use std::sync::atomic::{AtomicBool, Ordering};

    static DARK: AtomicBool = AtomicBool::new(false);

    /// Включить тёмную тему (`true`) или светлую (`false`).
    pub fn set_dark(dark: bool) {
        DARK.store(dark, Ordering::Relaxed);
    }

    /// Активна ли тёмная тема.
    #[inline]
    pub fn is_dark() -> bool {
        DARK.load(Ordering::Relaxed)
    }

    #[inline]
    fn pick(light: u32, dark: u32) -> u32 {
        if is_dark() { dark } else { light }
    }

    #[inline]
    pub fn APPBAR() -> u32 {
        pick(rgb(0x2E, 0x33, 0x3B), rgb(0x1E, 0x1F, 0x22))
    }
    #[inline]
    pub fn APPBAR_TEXT() -> u32 {
        pick(rgb(0xE8, 0xEB, 0xEF), rgb(0xD0, 0xD3, 0xD8))
    }
    #[inline]
    pub fn APPBAR_HOVER() -> u32 {
        pick(rgb(0x45, 0x4C, 0x57), rgb(0x3A, 0x3D, 0x42))
    }
    #[inline]
    pub fn APPBAR_DIM() -> u32 {
        pick(rgb(0x6B, 0x73, 0x80), rgb(0x5E, 0x62, 0x6A))
    }
    #[inline]
    pub fn APPBAR_BORDER() -> u32 {
        pick(rgb(0x5A, 0x62, 0x6E), rgb(0x4A, 0x4E, 0x55))
    }
    #[inline]
    pub fn TOOLBAR() -> u32 {
        pick(rgb(0xF5, 0xF6, 0xF8), rgb(0x2B, 0x2D, 0x30))
    }
    #[inline]
    pub fn TOOLBAR_BORDER() -> u32 {
        pick(rgb(0xD7, 0xDB, 0xE1), rgb(0x1C, 0x1D, 0x20))
    }
    #[inline]
    pub fn PANEL() -> u32 {
        pick(0xF7FF_FFFF, 0xF52B_2D30)
    }
    #[inline]
    pub fn PANEL_SOLID() -> u32 {
        pick(rgb(0xFF, 0xFF, 0xFF), rgb(0x2B, 0x2D, 0x30))
    }
    #[inline]
    pub fn PANEL_HEADER() -> u32 {
        pick(rgb(0xEE, 0xF0, 0xF3), rgb(0x33, 0x35, 0x39))
    }
    #[inline]
    pub fn BORDER() -> u32 {
        pick(rgb(0xC9, 0xCE, 0xD6), rgb(0x45, 0x48, 0x4E))
    }
    #[inline]
    pub fn TEXT() -> u32 {
        pick(rgb(0x24, 0x29, 0x30), rgb(0xD0, 0xD3, 0xD8))
    }
    #[inline]
    pub fn TEXT_DIM() -> u32 {
        pick(rgb(0x6B, 0x72, 0x7E), rgb(0x9A, 0x9F, 0xA8))
    }
    #[inline]
    pub fn TEXT_DISABLED() -> u32 {
        pick(rgb(0xA8, 0xAE, 0xB8), rgb(0x63, 0x67, 0x6E))
    }
    #[inline]
    pub fn HOVER() -> u32 {
        pick(rgb(0xE3, 0xEE, 0xFA), rgb(0x3A, 0x3E, 0x45))
    }
    #[inline]
    pub fn PRESSED() -> u32 {
        pick(rgb(0xC9, 0xDF, 0xF7), rgb(0x2F, 0x4A, 0x66))
    }
    #[inline]
    pub fn ACCENT() -> u32 {
        pick(rgb(0x0A, 0x7B, 0xD8), rgb(0x2D, 0x8C, 0xE6))
    }
    #[inline]
    pub fn ACCENT_DARK() -> u32 {
        pick(rgb(0x07, 0x5F, 0xA8), rgb(0x1F, 0x6F, 0xBF))
    }
    #[inline]
    pub fn ACCENT_SOFT() -> u32 {
        pick(rgb(0xD6, 0xE9, 0xFB), rgb(0x25, 0x3D, 0x56))
    }
    #[inline]
    pub fn ACCENT_HOVER() -> u32 {
        pick(rgb(0x20, 0x8A, 0xE6), rgb(0x45, 0x9C, 0xEE))
    }
    #[inline]
    pub fn FOCUS_RING() -> u32 {
        pick(rgb(0x8C, 0xBF, 0xEA), rgb(0x3D, 0x6E, 0x9E))
    }
    #[inline]
    pub fn SKETCH() -> u32 {
        pick(rgb(0xF0, 0x8A, 0x24), rgb(0xF0, 0x8A, 0x24))
    }
    #[inline]
    pub fn SKETCH_SOFT() -> u32 {
        pick(rgb(0xFD, 0xEB, 0xD8), rgb(0x4A, 0x37, 0x24))
    }
    #[inline]
    pub fn FIELD() -> u32 {
        pick(rgb(0xFF, 0xFF, 0xFF), rgb(0x22, 0x23, 0x26))
    }
    #[inline]
    pub fn FIELD_BORDER() -> u32 {
        pick(rgb(0xBF, 0xC5, 0xCE), rgb(0x4C, 0x50, 0x57))
    }
    #[inline]
    pub fn FIELD_BORDER_HOVER() -> u32 {
        pick(rgb(0x8F, 0x99, 0xA6), rgb(0x6E, 0x73, 0x7C))
    }
    #[inline]
    pub fn DANGER() -> u32 {
        pick(rgb(0xD9, 0x3B, 0x3B), rgb(0xE5, 0x55, 0x55))
    }
    #[inline]
    pub fn OK() -> u32 {
        pick(rgb(0x2E, 0x9E, 0x5B), rgb(0x3F, 0xB5, 0x6E))
    }
    #[inline]
    pub fn TIMELINE() -> u32 {
        pick(rgb(0xEC, 0xEE, 0xF1), rgb(0x26, 0x28, 0x2B))
    }
    #[inline]
    pub fn TIMELINE_ITEM() -> u32 {
        pick(rgb(0xFF, 0xFF, 0xFF), rgb(0x3A, 0x3D, 0x42))
    }
    #[inline]
    pub fn TIMELINE_ITEM_HOVER() -> u32 {
        pick(rgb(0xD3, 0xE5, 0xF8), rgb(0x2F, 0x4A, 0x66))
    }
    #[inline]
    pub fn TIMELINE_UNDONE() -> u32 {
        pick(rgb(0xE2, 0xE5, 0xEA), rgb(0x30, 0x32, 0x36))
    }
    #[inline]
    pub fn TIMELINE_UNDONE_HOVER() -> u32 {
        pick(rgb(0xE9, 0xED, 0xF2), rgb(0x36, 0x39, 0x3D))
    }
    #[inline]
    pub fn TOOLTIP() -> u32 {
        pick(rgb(0x30, 0x35, 0x3D), rgb(0x45, 0x48, 0x4E))
    }
    #[inline]
    pub fn DISABLED_VEIL() -> u32 {
        pick(0x99F5_F6F8, 0x992B_2D30)
    }
    #[inline]
    pub fn INFO_BOX() -> u32 {
        pick(rgb(0xF5, 0xF6, 0xF8), rgb(0x33, 0x35, 0x39))
    }
    #[inline]
    pub fn LABEL_BG() -> u32 {
        pick(0xF0FF_FFFF, 0xF033_3539)
    }
    #[inline]
    pub fn SWATCH_BORDER() -> u32 {
        pick(0x4000_0000, 0x40FF_FFFF)
    }
    #[inline]
    pub fn MODAL_DIM() -> u32 {
        pick(0x5010_1418, 0x7008_0809)
    }
}

/// Контекст кадра интерфейса.
pub struct Ui<'a> {
    pub fb: &'a mut Framebuffer,
    pub font: &'a Font,
    pub input: &'a Input,
    pub st: &'a mut UiState,
    /// Масштаб интерфейса (HiDPI).
    pub s: f32,
}

/// FNV-1a хеш для идентификаторов виджетов.
pub fn id_of(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h | 1
}

/// Комбинирование идентификатора с индексом.
pub fn id_idx(base: u64, i: usize) -> u64 {
    (base ^ ((i as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15))) | 1
}

impl<'a> Ui<'a> {
    pub fn new(fb: &'a mut Framebuffer, font: &'a Font, input: &'a Input, st: &'a mut UiState, s: f32) -> Self {
        st.hot = 0;
        st.tooltip = None;
        st.cursor = Cursor::Arrow;
        st.wake_ms = None;
        st.keyboard_taken = st.focus != 0;
        st.layer = 0;
        st.cur_layers.clear();
        st.frame += 1;
        let mouse_over_ui = st.prev_layers.iter().any(|(r, _)| r.contains(input.mouse.0, input.mouse.1));
        st.mouse_over_ui = mouse_over_ui;
        if !input.any_down() && st.active != 0 && !input.released.iter().any(|&r| r) {
            st.active = 0;
        }
        Self { fb, font, input, st, s }
    }

    /// Завершение кадра: подсказки, смена слоёв, таймеры.
    pub fn finish(self) {
        let st = self.st;
        if st.hot != st.prev_hot {
            st.prev_hot = st.hot;
            st.hot_since = if st.hot != 0 { Some(Instant::now()) } else { None };
            st.tooltip_ready = false;
        }
        if st.tooltip.is_some() {
            if let Some(t) = st.hot_since {
                let el = t.elapsed().as_millis() as u64;
                if el >= 550 {
                    st.tooltip_ready = true;
                } else {
                    st.wake_ms = Some(560 - el);
                }
            }
        }
        if self.input.released[0] {
            st.active = 0;
        }
        std::mem::swap(&mut st.prev_layers, &mut st.cur_layers);
    }

    /// Масштабирование логических пикселей.
    #[inline]
    pub fn px(&self, v: f32) -> f32 {
        (v * self.s).round()
    }

    /// Регистрация области текущего слоя (панель, меню, диалог).
    pub fn region(&mut self, r: Rect) {
        self.st.cur_layers.push((r, self.st.layer));
    }

    /// Верхний слой под курсором по данным прошлого кадра.
    fn top_layer_at(&self, x: f32, y: f32) -> u8 {
        self.st.prev_layers.iter().filter(|(r, _)| r.contains(x, y)).map(|(_, l)| *l).max().unwrap_or(0)
    }

    /// Курсор над прямоугольником и не перекрыт верхним слоем.
    pub fn hovered(&self, r: Rect) -> bool {
        let (mx, my) = self.input.mouse;
        r.contains(mx, my) && self.st.layer >= self.top_layer_at(mx, my)
    }

    /// Курсор над 3D-видом (не над интерфейсом).
    pub fn mouse_in_viewport(&self) -> bool {
        let (mx, my) = self.input.mouse;
        self.top_layer_at(mx, my) == 0 && self.st.active == 0
    }

    /// Базовая логика кнопки: (наведена, нажата-удерживается, кликнута).
    pub fn interact(&mut self, id: u64, r: Rect) -> (bool, bool, bool) {
        let hov = self.hovered(r) && (self.st.active == 0 || self.st.active == id);
        if hov {
            self.st.hot = id;
        }
        if hov && self.input.pressed[0] {
            self.st.active = id;
            if self.st.focus != 0 && self.st.focus != id {
                self.commit_focus();
            }
        }
        let held = self.st.active == id && self.input.down[0];
        let clicked = self.st.active == id && self.input.released[0] && hov;
        (hov, held, clicked)
    }

    /// Снятие фокуса с текстового поля с применением значения.
    pub fn commit_focus(&mut self) {
        if self.st.focus != 0 {
            self.st.committed = Some(self.st.focus);
        }
    }

    /// Ключ нажат в этом кадре (если ввод не поглощён полем).
    pub fn key(&self, k: Key) -> bool {
        !self.st.keyboard_taken && self.input.keys.iter().any(|(kk, _)| *kk == k)
    }

    /// Горячая клавиша с основным модификатором (Ctrl/Cmd).
    pub fn shortcut(&self, c: char) -> bool {
        !self.st.keyboard_taken && self.input.keys.iter().any(|(k, m)| *k == Key::Char(c) && m.primary() && !m.shift)
    }

    /// Горячая клавиша с Ctrl/Cmd+Shift.
    pub fn shortcut_shift(&self, c: char) -> bool {
        !self.st.keyboard_taken && self.input.keys.iter().any(|(k, m)| *k == Key::Char(c) && m.primary() && m.shift)
    }

    /// Одиночная клавиша без модификаторов.
    pub fn plain_key(&self, c: char) -> bool {
        !self.st.keyboard_taken && self.input.keys.iter().any(|(k, m)| *k == Key::Char(c) && !m.primary() && !m.alt)
    }

    // --- Рисование -------------------------------------------------------

    pub fn text(&mut self, x: f32, y: f32, text: &str, size: f32, color: u32) -> f32 {
        self.font.draw(self.fb, x.round(), y.round(), text, self.s * size, color)
    }

    pub fn text_width(&self, text: &str, size: f32) -> f32 {
        self.font.measure(text, self.s * size)
    }

    /// Текст, выровненный по вертикали в прямоугольнике. `align`: 0 — влево, 1 — центр, 2 — вправо.
    pub fn text_in(&mut self, r: Rect, text: &str, size: f32, color: u32, align: u8) {
        let mut px = self.s * size;
        // Не умещается — сначала уменьшаем шрифт (до 80 %), затем обрезаем с многоточием.
        let full = self.font.measure(text, px);
        if full > r.w && full > 0.0 {
            px = (px * r.w / full).max(px * 0.8);
        }
        let shown = self.font.ellipsize(text, px, r.w.max(0.0));
        let tw = self.font.measure(&shown, px);
        let lh = self.font.line_height(px);
        let x = match align {
            1 => r.x + (r.w - tw) * 0.5,
            2 => r.right() - tw,
            _ => r.x,
        };
        let y = r.y + (r.h - lh) * 0.5;
        let old = self.fb.clip;
        self.fb.set_clip(r.intersect(&old));
        self.font.draw(self.fb, x.round(), y.round(), &shown, px, color);
        self.fb.clip = old;
    }

    /// Рисование иконки.
    pub fn icon(&mut self, icon: icons::Icon, r: Rect, color: u32) {
        icons::draw(self.fb, icon, r, color);
    }

    /// Панель с тенью; регистрирует область слоя.
    pub fn panel(&mut self, r: Rect, radius: f32) {
        self.region(r);
        self.fb.shadow(r, self.px(radius), self.px(6.0), 70);
        self.fb.fill_rounded(r, self.px(radius), theme::PANEL());
        self.fb.stroke_rounded(r, self.px(radius), 1.0, theme::BORDER());
    }

    /// Установка подсказки для наведённого элемента.
    pub fn set_tooltip(&mut self, id: u64, text: &str) {
        if self.st.hot == id && self.st.active == 0 {
            self.st.tooltip = Some(text.to_string());
        }
    }

    /// Отрисовка подсказки (вызывается последней).
    pub fn draw_tooltip(&mut self) {
        if !self.st.tooltip_ready {
            return;
        }
        let Some(tip) = self.st.tooltip.clone() else { return };
        let size = 12.0;
        let lines: Vec<&str> = tip.lines().collect();
        let w = lines.iter().map(|l| self.text_width(l, size)).fold(0.0, f32::max) + self.px(16.0);
        let lh = self.font.line_height(self.s * size);
        let h = lh * lines.len() as f32 + self.px(10.0);
        let (mx, my) = self.input.mouse;
        let mut x = mx + self.px(12.0);
        let mut y = my + self.px(20.0);
        if x + w > self.fb.width as f32 - 4.0 {
            x = self.fb.width as f32 - w - 4.0;
        }
        if y + h > self.fb.height as f32 - 4.0 {
            y = my - h - self.px(8.0);
        }
        let r = Rect::new(x, y, w, h);
        self.fb.reset_clip();
        self.fb.shadow(r, self.px(4.0), self.px(4.0), 60);
        self.fb.fill_rounded(r, self.px(4.0), theme::TOOLTIP());
        for (i, l) in lines.iter().enumerate() {
            let c = if i == 0 { 0xFFFF_FFFF } else { 0xFFC8_CED6 };
            self.text(x + self.px(8.0), y + self.px(5.0) + lh * i as f32, l, size, c);
        }
    }
}

/// Вычисление простого арифметического выражения (как в полях Fusion 360):
/// `+ - * / ^`, скобки, унарный минус, запятая как десятичный разделитель.
pub fn eval_expr(s: &str) -> Option<f64> {
    let s: Vec<char> = s.replace(',', ".").chars().filter(|c| !c.is_whitespace()).collect();
    fn expr(s: &[char], i: &mut usize) -> Option<f64> {
        let mut v = term(s, i)?;
        while *i < s.len() {
            match s[*i] {
                '+' => {
                    *i += 1;
                    v += term(s, i)?;
                }
                '-' => {
                    *i += 1;
                    v -= term(s, i)?;
                }
                _ => break,
            }
        }
        Some(v)
    }
    fn term(s: &[char], i: &mut usize) -> Option<f64> {
        let mut v = power(s, i)?;
        while *i < s.len() {
            match s[*i] {
                '*' => {
                    *i += 1;
                    v *= power(s, i)?;
                }
                '/' => {
                    *i += 1;
                    let d = power(s, i)?;
                    if d == 0.0 {
                        return None;
                    }
                    v /= d;
                }
                _ => break,
            }
        }
        Some(v)
    }
    fn power(s: &[char], i: &mut usize) -> Option<f64> {
        let b = unary(s, i)?;
        if *i < s.len() && s[*i] == '^' {
            *i += 1;
            let e = power(s, i)?;
            return Some(b.powf(e));
        }
        Some(b)
    }
    fn unary(s: &[char], i: &mut usize) -> Option<f64> {
        if *i < s.len() && s[*i] == '-' {
            *i += 1;
            return Some(-unary(s, i)?);
        }
        if *i < s.len() && s[*i] == '+' {
            *i += 1;
            return unary(s, i);
        }
        atom(s, i)
    }
    fn atom(s: &[char], i: &mut usize) -> Option<f64> {
        if *i < s.len() && s[*i] == '(' {
            *i += 1;
            let v = expr(s, i)?;
            if *i < s.len() && s[*i] == ')' {
                *i += 1;
            }
            return Some(v);
        }
        // Константа pi.
        if s.len() >= *i + 2 && s[*i].to_ascii_lowercase() == 'p' && s[*i + 1].to_ascii_lowercase() == 'i' {
            *i += 2;
            return Some(std::f64::consts::PI);
        }
        let st = *i;
        while *i < s.len() && (s[*i].is_ascii_digit() || s[*i] == '.' || s[*i] == 'e' && *i > st) {
            *i += 1;
        }
        // Единицы измерения игнорируем (мм, mm, °, deg).
        let num: String = s[st..*i].iter().collect();
        let v = num.parse::<f64>().ok()?;
        while *i < s.len() && (s[*i].is_alphabetic() || s[*i] == '°') {
            *i += 1;
        }
        Some(v)
    }
    let mut i = 0;
    let v = expr(&s, &mut i)?;
    if i != s.len() || !v.is_finite() {
        return None;
    }
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expressions() {
        assert_eq!(eval_expr("10+5*2"), Some(20.0));
        assert_eq!(eval_expr("(1+2)*3"), Some(9.0));
        assert_eq!(eval_expr("-4/2"), Some(-2.0));
        assert_eq!(eval_expr("2^3"), Some(8.0));
        assert_eq!(eval_expr("12,5 мм"), Some(12.5));
        assert_eq!(eval_expr("45°"), Some(45.0));
        assert!((eval_expr("pi*2").unwrap() - 6.2831).abs() < 1e-3);
        assert_eq!(eval_expr("1/0"), None);
        assert_eq!(eval_expr("abc"), None);
    }
}
