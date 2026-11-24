//! Виджеты: кнопки, поля ввода, флажки, выпадающие списки, переключатели.

use super::icons::Icon;
use super::{theme, Cursor, Key, Ui};
use crate::renderer::canvas::{fade, Rect};

/// Запрос всплывающего списка (рисуется в конце кадра поверх всего).
#[derive(Clone, Debug)]
pub struct PopupList {
    pub id: u64,
    pub anchor: Rect,
    pub options: Vec<String>,
    pub selected: usize,
}

thread_local! {
    static POPUP: std::cell::RefCell<(Option<PopupList>, Option<(u64, usize)>, u64, Rect, u64)> =
        std::cell::RefCell::new((None, None, 0, Rect::default(), 0));
}

/// Форматирование числа без лишних нулей.
pub fn fmt_num(v: f32) -> String {
    if !v.is_finite() {
        return "—".into();
    }
    let mut s = format!("{:.3}", v);
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".into();
    }
    s
}

impl<'a> Ui<'a> {
    /// Текстовая кнопка. `primary` — акцентная (синяя).
    pub fn button(&mut self, id: u64, r: Rect, label: &str, primary: bool) -> bool {
        self.button_ex(id, r, label, primary, true)
    }

    pub fn button_ex(&mut self, id: u64, r: Rect, label: &str, primary: bool, enabled: bool) -> bool {
        let (hov, held, clicked) = if enabled { self.interact(id, r) } else { (false, false, false) };
        let rad = self.px(4.0);
        let (bg, fg, border) = if primary {
            let bg = if !enabled {
                theme::TEXT_DISABLED()
            } else if held {
                theme::ACCENT_DARK()
            } else if hov {
                theme::ACCENT_HOVER()
            } else {
                theme::ACCENT()
            };
            (bg, 0xFFFF_FFFF, bg)
        } else {
            let bg = if held { theme::PRESSED() } else if hov { theme::HOVER() } else { theme::FIELD() };
            (bg, if enabled { theme::TEXT() } else { theme::TEXT_DISABLED() }, theme::FIELD_BORDER())
        };
        self.fb.fill_rounded(r, rad, bg);
        self.fb.stroke_rounded(r, rad, 1.0, border);
        self.text_in(Rect::new(r.x + 5.0 * self.s, r.y, (r.w - 10.0 * self.s).max(0.0), r.h), label, 12.5, fg, 1);
        if hov {
            self.st.cursor = Cursor::Hand;
        }
        clicked
    }

    /// Плоская кнопка-иконка с подсказкой.
    pub fn icon_button(&mut self, id: u64, r: Rect, icon: Icon, tooltip: &str, active: bool, enabled: bool) -> bool {
        let (hov, held, clicked) = if enabled { self.interact(id, r) } else { (false, false, false) };
        let rad = self.px(4.0);
        if active {
            self.fb.fill_rounded(r, rad, theme::ACCENT_SOFT());
            self.fb.stroke_rounded(r, rad, 1.0, theme::FOCUS_RING());
        } else if held {
            self.fb.fill_rounded(r, rad, theme::PRESSED());
        } else if hov {
            self.fb.fill_rounded(r, rad, theme::HOVER());
        }
        let pad = (r.w.min(r.h) * 0.16).round();
        let ir = r.shrink(pad);
        let col = if enabled { theme::TEXT() } else { theme::TEXT_DISABLED() };
        self.icon(icon, ir, col);
        if !enabled {
            // Приглушение цветной иконки.
            self.fb.fill_rect(r, theme::DISABLED_VEIL());
        }
        if !tooltip.is_empty() {
            self.set_tooltip(id, tooltip);
        }
        if hov {
            self.st.cursor = Cursor::Hand;
        }
        clicked
    }

    /// Кнопка инструмента: большая иконка + (опционально) подпись снизу.
    pub fn tool_button(&mut self, id: u64, r: Rect, icon: Icon, label: &str, tooltip: &str, active: bool, enabled: bool) -> bool {
        let (hov, held, clicked) = if enabled { self.interact(id, r) } else { (false, false, false) };
        let rad = self.px(4.0);
        if active {
            self.fb.fill_rounded(r, rad, theme::ACCENT_SOFT());
        } else if held {
            self.fb.fill_rounded(r, rad, theme::PRESSED());
        } else if hov {
            self.fb.fill_rounded(r, rad, theme::HOVER());
        }
        let isz = self.px(30.0);
        let ir = Rect::new(r.x + (r.w - isz) * 0.5, r.y + self.px(3.0), isz, isz);
        self.icon(icon, ir, if enabled { theme::TEXT() } else { theme::TEXT_DISABLED() });
        if !label.is_empty() {
            let lr = Rect::new(r.x, ir.bottom(), r.w, r.bottom() - ir.bottom());
            self.text_in(lr, label, 10.5, if enabled { theme::TEXT() } else { theme::TEXT_DISABLED() }, 1);
        }
        if !enabled {
            self.fb.fill_rect(ir, theme::DISABLED_VEIL());
        }
        if !tooltip.is_empty() {
            self.set_tooltip(id, tooltip);
        }
        if hov {
            self.st.cursor = Cursor::Hand;
        }
        clicked
    }

    /// Флажок.
    pub fn checkbox(&mut self, id: u64, r: Rect, label: &str, v: &mut bool) -> bool {
        let (hov, _held, clicked) = self.interact(id, r);
        let bs = self.px(15.0);
        let b = Rect::new(r.x, r.y + (r.h - bs) * 0.5, bs, bs);
        let rad = self.px(3.0);
        if *v {
            self.fb.fill_rounded(b, rad, theme::ACCENT());
            let s = self.s;
            self.fb.polyline(
                &[(b.x + 3.5 * s, b.y + 7.5 * s), (b.x + 6.5 * s, b.y + 10.5 * s), (b.x + 11.5 * s, b.y + 4.5 * s)],
                1.8 * s,
                0xFFFF_FFFF,
                false,
            );
        } else {
            self.fb.fill_rounded(b, rad, if hov { theme::HOVER() } else { theme::FIELD() });
            self.fb.stroke_rounded(b, rad, 1.0, theme::FIELD_BORDER());
        }
        let lr = Rect::new(b.right() + self.px(7.0), r.y, r.right() - b.right() - self.px(7.0), r.h);
        self.text_in(lr, label, 12.5, theme::TEXT(), 0);
        if clicked {
            *v = !*v;
        }
        if hov {
            self.st.cursor = Cursor::Hand;
        }
        clicked
    }

    /// Сегментированный переключатель (выбор одного варианта).
    pub fn segmented(&mut self, id: u64, r: Rect, options: &[&str], sel: &mut usize) -> bool {
        let n = options.len().max(1);
        let w = r.w / n as f32;
        let rad = self.px(4.0);
        self.fb.fill_rounded(r, rad, theme::PANEL_HEADER());
        self.fb.stroke_rounded(r, rad, 1.0, theme::FIELD_BORDER());
        let mut changed = false;
        for (i, o) in options.iter().enumerate() {
            let cr = Rect::new(r.x + w * i as f32, r.y, w, r.h);
            let cid = super::id_idx(id, i);
            let (hov, _, clicked) = self.interact(cid, cr);
            if i == *sel {
                self.fb.fill_rounded(cr.shrink(self.px(2.0)), rad, theme::ACCENT());
                self.text_in(cr.shrink(3.0 * self.s), o, 12.0, 0xFFFF_FFFF, 1);
            } else {
                if hov {
                    self.fb.fill_rounded(cr.shrink(self.px(2.0)), rad, theme::HOVER());
                }
                self.text_in(cr.shrink(3.0 * self.s), o, 12.0, theme::TEXT(), 1);
            }
            if clicked && *sel != i {
                *sel = i;
                changed = true;
            }
            if hov {
                self.st.cursor = Cursor::Hand;
            }
        }
        changed
    }

    /// Однострочное текстовое поле. Возвращает `true`, когда значение подтверждено.
    pub fn text_field(&mut self, id: u64, r: Rect, value: &mut String) -> bool {
        let shown = value.clone();
        match self.edit_core(id, r, &shown, "", false) {
            Some(s) => {
                *value = s;
                true
            }
            None => false,
        }
    }

    /// Числовое поле с поддержкой выражений и колесом мыши.
    pub fn number_field(&mut self, id: u64, r: Rect, v: &mut f32, unit: &str, step: f32, min: f32, max: f32) -> bool {
        let shown = fmt_num(*v);
        let mut changed = false;
        if let Some(s) = self.edit_core(id, r, &shown, unit, true) {
            if let Some(x) = super::eval_expr(&s) {
                let nv = (x as f32).clamp(min, max);
                if nv != *v {
                    *v = nv;
                    changed = true;
                }
            }
        }
        if self.st.focus != id && self.hovered(r) && self.input.wheel != 0.0 && step > 0.0 {
            let k = if self.input.mods.shift { 10.0 } else { 1.0 };
            let nv = (*v + self.input.wheel.signum() * step * k).clamp(min, max);
            let nv = (nv / step).round() * step;
            if nv != *v {
                *v = nv;
                changed = true;
            }
        }
        changed
    }

    /// Общая логика редактирования. Возвращает новый текст при подтверждении.
    fn edit_core(&mut self, id: u64, r: Rect, shown: &str, unit: &str, numeric: bool) -> Option<String> {
        let (hov, _held, clicked) = self.interact(id, r);
        let focused = self.st.focus == id;
        let rad = self.px(3.0);
        let mut result = None;

        if hov {
            self.st.cursor = Cursor::Text;
        }
        if clicked && !focused {
            self.st.focus = id;
            self.st.edit_buf = shown.to_string();
            self.st.edit_cursor = self.st.edit_buf.chars().count();
            self.st.edit_select_all = true;
            self.st.keyboard_taken = true;
        }
        // Подтверждение по внешнему клику.
        if self.st.committed == Some(id) {
            self.st.committed = None;
            if self.st.focus == id {
                self.st.focus = 0;
            }
            result = Some(self.st.edit_buf.clone());
        }
        let focused = self.st.focus == id;
        if focused {
            POPUP.with(|p| {
                let mut p = p.borrow_mut();
                p.3 = r;
                p.4 = self.st.frame;
            });
            self.st.keyboard_taken = true;
            let input = self.input;
            let mut chars: Vec<char> = self.st.edit_buf.chars().collect();
            let mut cur = self.st.edit_cursor.min(chars.len());
            for c in input.text.chars() {
                if numeric && !(c.is_ascii_digit() || "+-*/^().,e pi°".contains(c)) {
                    continue;
                }
                if self.st.edit_select_all {
                    chars.clear();
                    cur = 0;
                    self.st.edit_select_all = false;
                }
                chars.insert(cur, c);
                cur += 1;
            }
            for (k, m) in &input.keys {
                match k {
                    Key::Backspace => {
                        if self.st.edit_select_all {
                            chars.clear();
                            cur = 0;
                            self.st.edit_select_all = false;
                        } else if cur > 0 {
                            cur -= 1;
                            chars.remove(cur);
                        }
                    }
                    Key::Delete => {
                        if self.st.edit_select_all {
                            chars.clear();
                            cur = 0;
                            self.st.edit_select_all = false;
                        } else if cur < chars.len() {
                            chars.remove(cur);
                        }
                    }
                    Key::Left => {
                        self.st.edit_select_all = false;
                        cur = cur.saturating_sub(1);
                    }
                    Key::Right => {
                        self.st.edit_select_all = false;
                        cur = (cur + 1).min(chars.len());
                    }
                    Key::Home => {
                        self.st.edit_select_all = false;
                        cur = 0;
                    }
                    Key::End => {
                        self.st.edit_select_all = false;
                        cur = chars.len();
                    }
                    Key::Char('a') if m.primary() => self.st.edit_select_all = true,
                    Key::Enter | Key::Tab => {
                        result = Some(chars.iter().collect());
                        self.st.focus = 0;
                    }
                    Key::Escape => {
                        self.st.focus = 0;
                    }
                    _ => {}
                }
            }
            self.st.edit_buf = chars.iter().collect();
            self.st.edit_cursor = cur;
        }
        let focused = self.st.focus == id;

        // Рисование.
        self.fb.fill_rounded(r, rad, theme::FIELD());
        self.fb.stroke_rounded(r, rad, if focused { 1.6 } else { 1.0 }, if focused { theme::ACCENT() } else if hov { theme::FIELD_BORDER_HOVER() } else { theme::FIELD_BORDER() });
        let pad = self.px(6.0);
        let inner = Rect::new(r.x + pad, r.y, r.w - pad * 2.0, r.h);
        let size = 12.5;
        if focused {
            let txt = self.st.edit_buf.clone();
            let lh = self.font.line_height(self.s * size);
            let ty = r.y + (r.h - lh) * 0.5;
            let old = self.fb.clip;
            self.fb.set_clip(inner.intersect(&old));
            if self.st.edit_select_all && !txt.is_empty() {
                let tw = self.text_width(&txt, size);
                self.fb.fill_rect(Rect::new(inner.x, ty, tw, lh), theme::ACCENT_SOFT());
            }
            self.text(inner.x, ty, &txt, size, theme::TEXT());
            let before: String = txt.chars().take(self.st.edit_cursor).collect();
            let cx = inner.x + self.text_width(&before, size);
            self.fb.fill_rect(Rect::new(cx.round(), ty + 1.0, self.s.max(1.0), lh - 2.0), theme::TEXT());
            self.fb.clip = old;
        } else {
            let label = if unit.is_empty() { shown.to_string() } else { format!("{shown} {unit}") };
            self.text_in(inner, &label, size, theme::TEXT(), 0);
        }
        result
    }

    /// Выпадающий список. Возвращает `true`, если выбор изменился.
    pub fn combo(&mut self, id: u64, r: Rect, options: &[String], sel: &mut usize) -> bool {
        let mut changed = false;
        POPUP.with(|p| {
            let mut p = p.borrow_mut();
            if let Some((pid, idx)) = p.1 {
                if pid == id {
                    p.1 = None;
                    if idx != *sel && idx < options.len() {
                        *sel = idx;
                        changed = true;
                    }
                }
            }
        });
        let (hov, held, clicked) = self.interact(id, r);
        let rad = self.px(3.0);
        self.fb.fill_rounded(r, rad, if held { theme::PRESSED() } else if hov { theme::HOVER() } else { theme::FIELD() });
        self.fb.stroke_rounded(r, rad, 1.0, theme::FIELD_BORDER());
        let pad = self.px(7.0);
        let label = options.get(*sel).cloned().unwrap_or_default();
        self.text_in(Rect::new(r.x + pad, r.y, r.w - pad * 2.0 - self.px(12.0), r.h), &label, 12.5, theme::TEXT(), 0);
        let ar = Rect::new(r.right() - self.px(20.0), r.y, self.px(14.0), r.h);
        self.icon(Icon::ChevronDown, Rect::new(ar.x, r.y + (r.h - self.px(12.0)) * 0.5, self.px(12.0), self.px(12.0)), theme::TEXT_DIM());
        if clicked {
            POPUP.with(|p| {
                let mut p = p.borrow_mut();
                if p.0.as_ref().map(|q| q.id) == Some(id) {
                    p.0 = None;
                } else {
                    p.0 = Some(PopupList { id, anchor: r, options: options.to_vec(), selected: *sel });
                }
            });
        }
        if hov {
            self.st.cursor = Cursor::Hand;
        }
        changed
    }

    /// Отрисовка открытого выпадающего списка (вызывать в конце кадра).
    pub fn draw_popups(&mut self) {
        // Снятие фокуса с поля при клике вне его.
        let (frame_seen, frect) = POPUP.with(|p| {
            let p = p.borrow();
            (p.4, p.3)
        });
        if self.st.focus != 0 {
            if frame_seen + 1 < self.st.frame {
                // Поле с фокусом больше не отображается.
                self.st.focus = 0;
            } else if self.input.pressed.iter().any(|&b| b) && !frect.contains(self.input.mouse.0, self.input.mouse.1) {
                self.st.committed = Some(self.st.focus);
                self.st.wake_ms = Some(0);
            }
        }
        let popup = POPUP.with(|p| p.borrow().0.clone());
        let Some(pl) = popup else { return };
        let old_layer = self.st.layer;
        self.st.layer = 3;
        let ih = self.px(26.0);
        let h = ih * pl.options.len() as f32 + self.px(8.0);
        let mut y = pl.anchor.bottom() + self.px(2.0);
        if y + h > self.fb.height as f32 {
            y = (pl.anchor.y - h - self.px(2.0)).max(0.0);
        }
        let w = pl.anchor.w.max(pl.options.iter().map(|o| self.text_width(o, 12.5)).fold(0.0, f32::max) + self.px(24.0));
        let r = Rect::new(pl.anchor.x, y, w, h);
        self.fb.reset_clip();
        self.panel(r, 5.0);
        let mut chosen = None;
        for (i, o) in pl.options.iter().enumerate() {
            let ir = Rect::new(r.x + self.px(4.0), r.y + self.px(4.0) + ih * i as f32, r.w - self.px(8.0), ih);
            let iid = super::id_idx(pl.id ^ 0x55, i);
            let (hov, _, clicked) = self.interact(iid, ir);
            if hov || i == pl.selected {
                self.fb.fill_rounded(ir, self.px(3.0), if hov { theme::HOVER() } else { theme::ACCENT_SOFT() });
            }
            self.text_in(Rect::new(ir.x + self.px(8.0), ir.y, ir.w - self.px(16.0), ir.h), o, 12.5, theme::TEXT(), 0);
            if clicked {
                chosen = Some(i);
            }
        }
        let clicked_outside = self.input.pressed[0] && !r.contains(self.input.mouse.0, self.input.mouse.1) && !pl.anchor.contains(self.input.mouse.0, self.input.mouse.1);
        POPUP.with(|p| {
            let mut p = p.borrow_mut();
            if let Some(i) = chosen {
                p.1 = Some((pl.id, i));
                p.0 = None;
            } else if clicked_outside || self.input.keys.iter().any(|(k, _)| *k == Key::Escape) {
                p.0 = None;
            }
        });
        if chosen.is_some() {
            self.st.wake_ms = Some(0);
        }
        self.st.layer = old_layer;
    }

    /// Открыт ли какой-либо выпадающий список.
    pub fn popup_open(&self) -> bool {
        POPUP.with(|p| p.borrow().0.is_some())
    }

    /// Подпись слева от поля: возвращает прямоугольник поля.
    pub fn labeled(&mut self, r: Rect, label: &str, label_w: f32) -> Rect {
        let lw = self.px(label_w);
        self.text_in(Rect::new(r.x, r.y, lw - self.px(6.0), r.h), label, 12.0, theme::TEXT_DIM(), 0);
        Rect::new(r.x + lw, r.y, r.w - lw, r.h)
    }

    /// Заголовок раздела панели.
    pub fn section(&mut self, r: Rect, title: &str) {
        self.text_in(r, &title.to_uppercase(), 10.5, theme::TEXT_DIM(), 0);
        let y = r.bottom() - 1.0;
        self.fb.fill_rect(Rect::new(r.x, y, r.w, 1.0), fade(theme::BORDER(), 0.7));
    }

    /// Палитра цветов. Возвращает выбранный цвет при клике.
    pub fn color_palette(&mut self, id: u64, r: Rect, colors: &[u32], current: u32) -> Option<u32> {
        let n = colors.len().max(1);
        let sz = (r.w / n as f32).min(r.h);
        let mut out = None;
        for (i, &c) in colors.iter().enumerate() {
            let cr = Rect::new(r.x + sz * i as f32, r.y, sz, sz).shrink(self.px(2.0));
            let cid = super::id_idx(id, i);
            let (hov, _, clicked) = self.interact(cid, cr);
            self.fb.fill_rounded(cr, self.px(3.0), c | 0xFF00_0000);
            let sel = (c & 0xFF_FFFF) == (current & 0xFF_FFFF);
            if sel {
                self.fb.stroke_rounded(cr.shrink(-self.px(1.5)), self.px(4.0), self.px(2.0), theme::ACCENT());
            } else {
                self.fb.stroke_rounded(cr, self.px(3.0), 1.0, if hov { theme::TEXT_DIM() } else { theme::SWATCH_BORDER() });
            }
            if clicked {
                out = Some(c);
            }
            if hov {
                self.st.cursor = Cursor::Hand;
            }
        }
        out
    }

    /// Строка меню. Возвращает `true` при клике.
    pub fn menu_item(&mut self, id: u64, r: Rect, icon: Option<Icon>, label: &str, shortcut: &str, enabled: bool) -> bool {
        let (hov, _, clicked) = if enabled { self.interact(id, r) } else { (false, false, false) };
        if hov {
            self.fb.fill_rounded(r, self.px(3.0), theme::HOVER());
        }
        let col = if enabled { theme::TEXT() } else { theme::TEXT_DISABLED() };
        let isz = self.px(18.0);
        if let Some(ic) = icon {
            self.icon(ic, Rect::new(r.x + self.px(6.0), r.y + (r.h - isz) * 0.5, isz, isz), col);
        }
        let tx = r.x + self.px(32.0);
        self.text_in(Rect::new(tx, r.y, r.w - self.px(40.0), r.h), label, 12.5, col, 0);
        if !shortcut.is_empty() {
            self.text_in(Rect::new(r.x, r.y, r.w - self.px(10.0), r.h), shortcut, 11.5, theme::TEXT_DIM(), 2);
        }
        if hov {
            self.st.cursor = Cursor::Hand;
        }
        clicked
    }

    /// Разделитель в меню.
    pub fn menu_separator(&mut self, r: Rect) {
        let y = (r.y + r.h * 0.5).round();
        self.fb.fill_rect(Rect::new(r.x + self.px(8.0), y, r.w - self.px(16.0), 1.0), theme::TOOLBAR_BORDER());
    }
}
