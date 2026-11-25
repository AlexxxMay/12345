//! Диалоги команд (боковая панель с предпросмотром) и модальные окна.

use super::actions::Action;
use super::panels::{edit_primitive, prim_param_count};
use super::view::fmt_count;
use super::{t, State};
use crate::csg::BoolOp;
use crate::math::Mat4;
use crate::mesh::{Mesh, Primitive};
use crate::renderer::canvas::Rect;
use crate::ui::icons::Icon;
use crate::ui::{fmt_num, id_of, theme, Ui};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Открытый диалог.
#[derive(Clone, Debug)]
pub enum Dialog {
    Primitive { prim: Primitive },
    SketchPlane,
    Extrude { sketch: u64, distance: f32, symmetric: bool, op: usize, target: u64 },
    Combine { target: u64, tool: u64, op: usize, keep: bool },
    Mirror { plane: usize, copy: bool },
    Reduce { id: u64, percent: f32 },
    Info { id: u64 },
    Settings,
    Help,
    ConfirmQuit,
    ConfirmDiscard { then: Action },
    Message { title: String, text: String },
}

enum Outcome {
    Keep,
    Close,
}

impl Dialog {
    /// Диалог отображается в боковой панели (не модальный).
    pub fn is_side_panel(&self) -> bool {
        matches!(self, Dialog::Primitive { .. } | Dialog::SketchPlane | Dialog::Extrude { .. } | Dialog::Combine { .. } | Dialog::Mirror { .. } | Dialog::Reduce { .. })
    }

    /// Ключ предпросмотра (изменение — пересчёт меша предпросмотра).
    pub fn preview_key(&self, st: &State) -> Option<u64> {
        let mut h = DefaultHasher::new();
        match self {
            Dialog::Primitive { prim } => (format!("p{prim:?}"), st.scene.revision).hash(&mut h),
            Dialog::Extrude { sketch, distance, symmetric, .. } => {
                (sketch, distance.to_bits(), symmetric).hash(&mut h);
                st.scene.revision.hash(&mut h);
            }
            Dialog::Mirror { plane, .. } => {
                ("m", plane, st.scene.selected, st.scene.revision).hash(&mut h);
            }
            _ => return None,
        }
        Some(h.finish() | 1)
    }

    /// Меш предпросмотра и его трансформация.
    pub fn build_preview(&self, st: &State) -> Option<(Mesh, Mat4)> {
        match self {
            Dialog::Primitive { prim } => {
                let m = prim.build();
                let tf = st.placement_for(&m);
                Some((m, tf))
            }
            Dialog::Extrude { sketch, distance, symmetric, .. } => {
                let (m, tf) = extrude_mesh(st, *sketch, *distance, *symmetric)?;
                Some((m, tf))
            }
            Dialog::Mirror { plane, .. } => {
                let b = st.scene.selected_body()?;
                if b.mesh.is_empty() {
                    return None;
                }
                Some((mirrored_world(&b.mesh, b.transform, *plane), Mat4::identity()))
            }
            _ => None,
        }
    }
}

/// Меш выдавливания эскиза (в локальных координатах тела-эскиза) и трансформация тела.
pub fn extrude_mesh(st: &State, sketch_id: u64, distance: f32, symmetric: bool) -> Option<(Mesh, Mat4)> {
    let b = st.scene.body(sketch_id)?;
    let sk = b.sketch.as_ref()?;
    if distance.abs() < 1e-6 {
        return None;
    }
    let mut m = sk.extrude(distance)?;
    if symmetric {
        let (_, _, _, n) = sk.plane.frame();
        m.translate(n * (-distance * 0.5));
    }
    Some((m, b.transform))
}

/// Зеркальная копия меша в мировых координатах. `plane`: 0 — YZ (по X), 1 — XZ (по Y), 2 — XY (по Z).
pub fn mirrored_world(mesh: &Mesh, tf: Mat4, plane: usize) -> Mesh {
    let mut m = mesh.clone();
    m.transform(tf);
    for p in &mut m.positions {
        match plane {
            0 => p.x = -p.x,
            1 => p.y = -p.y,
            _ => p.z = -p.z,
        }
    }
    m.flip_winding();
    m.compute_smooth_normals();
    m
}

impl State {
    fn solid_ids(&self) -> Vec<(u64, String)> {
        self.scene.bodies.iter().filter(|b| !b.mesh.is_empty()).map(|b| (b.id, b.name.clone())).collect()
    }

    fn body_combo(ui: &mut Ui, id: u64, r: Rect, list: &[(u64, String)], cur: &mut u64) -> bool {
        let names: Vec<String> = list.iter().map(|(_, n)| n.clone()).collect();
        let mut idx = list.iter().position(|(i, _)| i == cur).unwrap_or(0);
        let ch = ui.combo(id, r, &names, &mut idx);
        if let Some((i, _)) = list.get(idx) {
            *cur = *i;
        }
        ch
    }

    pub(super) fn draw_side_dialog(&mut self, ui: &mut Ui) {
        let Some(mut d) = self.dialog.take() else { return };
        let s = ui.s;
        let area = self.layout.right;
        let row = 30.0 * s;
        let (title, icon, rows): (String, Icon, f32) = match &d {
            Dialog::Primitive { prim } => (State::primitive_name(prim).to_string(), prim_icon(prim), prim_param_count(prim) as f32 + 1.0),
            Dialog::SketchPlane => (t("Создать эскиз", "Create sketch").into(), Icon::Sketch, 5.0),
            Dialog::Extrude { op, .. } => (t("Выдавить", "Extrude").into(), Icon::Extrude, if *op == 0 { 5.0 } else { 6.0 }),
            Dialog::Combine { .. } => (t("Объединить", "Combine").into(), Icon::Union, 5.5),
            Dialog::Mirror { .. } => (t("Зеркало", "Mirror").into(), Icon::Mirror, 3.5),
            Dialog::Reduce { .. } => (t("Упростить сетку", "Reduce mesh").into(), Icon::Reduce, 4.0),
            _ => (String::new(), Icon::Info, 2.0),
        };
        let h = 40.0 * s + rows * row + 56.0 * s;
        let r = Rect::new(area.x, area.y, area.w, h.min(area.h.max(200.0 * s)));
        ui.panel(r, 6.0);
        // Заголовок (тёмный, как в командных диалогах).
        let hr = Rect::new(r.x, r.y, r.w, 36.0 * s);
        ui.fb.fill_rounded(Rect::new(hr.x, hr.y, hr.w, hr.h), 6.0 * s, theme::APPBAR());
        ui.fb.fill_rect(Rect::new(hr.x, hr.bottom() - 6.0 * s, hr.w, 6.0 * s), theme::APPBAR());
        let ib = Rect::new(hr.x + 8.0 * s, hr.y + 6.0 * s, 24.0 * s, 24.0 * s);
        ui.fb.fill_rounded(ib, 4.0 * s, theme::INFO_BOX());
        ui.icon(icon, ib.shrink(2.0 * s), theme::TEXT());
        ui.text_in(Rect::new(hr.x + 40.0 * s, hr.y, hr.w - 50.0 * s, hr.h), &title.to_uppercase(), 12.5, theme::APPBAR_TEXT(), 0);

        let x0 = r.x + 12.0 * s;
        let w0 = r.w - 24.0 * s;
        let mut y = hr.bottom() + 10.0 * s;
        let mut ok_label = t("ОК", "OK");
        let mut ok_enabled = true;
        let mut show_ok = true;
        let solids = self.solid_ids();

        match &mut d {
            Dialog::Primitive { prim } => {
                ui.text_in(Rect::new(x0, y, w0, 20.0 * s), t("Тело встанет на плоскость XY рядом с моделью", "Placed on the XY plane next to the model"), 11.5, theme::TEXT_DIM(), 0);
                y += row;
                edit_primitive(ui, prim, x0, &mut y, w0, row, "dlg_prim");
            }
            Dialog::SketchPlane => {
                show_ok = false;
                ui.text_in(Rect::new(x0, y, w0, 20.0 * s), t("Выберите плоскость эскиза:", "Choose the sketch plane:"), 12.0, theme::TEXT(), 0);
                y += row;
                let planes = [
                    (crate::sketch::SketchPlane::Xy, t("XY — вид сверху", "XY — top")),
                    (crate::sketch::SketchPlane::Xz, t("XZ — вид спереди", "XZ — front")),
                    (crate::sketch::SketchPlane::Yz, t("YZ — вид справа", "YZ — right")),
                ];
                for (i, (pl, name)) in planes.iter().enumerate() {
                    let br = Rect::new(x0, y, w0, 34.0 * s);
                    let id = crate::ui::id_idx(id_of("dlg_plane"), i);
                    let (hov, _, clicked) = ui.interact(id, br);
                    ui.fb.fill_rounded(br, 4.0 * s, if hov { theme::HOVER() } else { theme::FIELD() });
                    ui.fb.stroke_rounded(br, 4.0 * s, 1.0, if hov { theme::ACCENT() } else { theme::FIELD_BORDER() });
                    ui.icon(Icon::Plane, Rect::new(br.x + 8.0 * s, br.y + 5.0 * s, 24.0 * s, 24.0 * s), theme::TEXT());
                    ui.text_in(Rect::new(br.x + 40.0 * s, br.y, br.w - 44.0 * s, br.h), name, 12.5, theme::TEXT(), 0);
                    if clicked {
                        let plane = *pl;
                        self.begin_sketch(plane, None);
                        return;
                    }
                    y += 40.0 * s;
                }
            }
            Dialog::Extrude { sketch, distance, symmetric, op, target } => {
                let sketches: Vec<(u64, String)> = self.scene.bodies.iter().filter(|b| b.sketch.is_some()).map(|b| (b.id, b.name.clone())).collect();
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Профиль", "Profile"), 100.0);
                State::body_combo(ui, id_of("dlg_ex_sketch"), fr, &sketches, sketch);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Расстояние", "Distance"), 100.0);
                ui.number_field(id_of("dlg_ex_dist"), fr, distance, t("мм", "mm"), 1.0, -1e5, 1e5);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Направление", "Direction"), 100.0);
                let mut sym = *symmetric as usize;
                if ui.segmented(id_of("dlg_ex_sym"), fr, &[t("Одно", "One side"), t("Симметр.", "Symmetric")], &mut sym) {
                    *symmetric = sym == 1;
                }
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Операция", "Operation"), 100.0);
                let has_solids = !solids.is_empty();
                let mut o = *op;
                if ui.segmented(id_of("dlg_ex_op"), fr, &[t("Новое", "New"), t("Соед.", "Join"), t("Вырез.", "Cut"), t("Перес.", "Inter.")], &mut o) {
                    *op = if has_solids { o } else { 0 };
                }
                y += row;
                if *op != 0 {
                    if !solids.iter().any(|(i, _)| i == target) {
                        *target = solids.first().map(|(i, _)| *i).unwrap_or(0);
                    }
                    let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Тело", "Body"), 100.0);
                    State::body_combo(ui, id_of("dlg_ex_target"), fr, &solids, target);
                    y += row;
                }
                let ok = extrude_mesh(self, *sketch, *distance, *symmetric).is_some();
                if !ok {
                    ui.text_in(Rect::new(x0, y, w0, 20.0 * s), t("Нет замкнутых профилей в эскизе", "No closed profiles in the sketch"), 11.5, theme::DANGER(), 0);
                }
                ok_enabled = ok;
            }
            Dialog::Combine { target, tool, op, keep } => {
                if !solids.iter().any(|(i, _)| i == target) {
                    *target = solids.first().map(|(i, _)| *i).unwrap_or(0);
                }
                if !solids.iter().any(|(i, _)| i == tool) || tool == target {
                    *tool = solids.iter().map(|(i, _)| *i).find(|i| i != target).unwrap_or(0);
                }
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Целевое тело", "Target body"), 110.0);
                State::body_combo(ui, id_of("dlg_cb_target"), fr, &solids, target);
                y += row;
                let tools: Vec<(u64, String)> = solids.iter().filter(|(i, _)| i != target).cloned().collect();
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Тело-инструмент", "Tool body"), 110.0);
                State::body_combo(ui, id_of("dlg_cb_tool"), fr, &tools, tool);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Операция", "Operation"), 110.0);
                ui.segmented(id_of("dlg_cb_op"), fr, &[t("Соединить", "Join"), t("Вырезать", "Cut"), t("Пересечь", "Intersect")], op);
                y += row;
                ui.checkbox(id_of("dlg_cb_keep"), Rect::new(x0, y, w0, 24.0 * s), t("Сохранить тело-инструмент", "Keep tool body"), keep);
                y += row;
                ok_enabled = *tool != 0 && *target != 0 && tool != target;
                ui.text_in(Rect::new(x0, y, w0, 18.0 * s), t("Тела должны быть замкнутыми", "Bodies should be watertight"), 11.0, theme::TEXT_DIM(), 0);
            }
            Dialog::Mirror { plane, copy } => {
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Плоскость", "Plane"), 100.0);
                ui.segmented(id_of("dlg_mi_plane"), fr, &["YZ", "XZ", "XY"], plane);
                y += row;
                ui.checkbox(id_of("dlg_mi_copy"), Rect::new(x0, y, w0, 24.0 * s), t("Создать копию", "Create a copy"), copy);
                y += row;
                ok_enabled = self.scene.selected_body().map(|b| !b.mesh.is_empty()).unwrap_or(false);
            }
            Dialog::Reduce { id, percent } => {
                let tris = self.scene.body(*id).map(|b| b.mesh.triangle_count()).unwrap_or(0);
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Оставить", "Keep"), 100.0);
                ui.number_field(id_of("dlg_rd_pct"), fr, percent, "%", 5.0, 1.0, 100.0);
                y += row;
                let target = ((tris as f32) * *percent / 100.0) as usize;
                ui.text_in(Rect::new(x0, y, w0, 22.0 * s), &format!("{} → {} {}", fmt_count(tris), fmt_count(target), t("треуг.", "tris")), 12.0, theme::TEXT(), 0);
                y += row;
                ok_label = t("Упростить", "Reduce");
            }
            _ => {}
        }
        let _ = y;

        // Кнопки.
        let mut outcome = Outcome::Keep;
        let by = r.bottom() - 40.0 * s;
        let bw = (w0 - 8.0 * s) * 0.5;
        if show_ok {
            if ui.button_ex(id_of("dlg_ok"), Rect::new(x0, by, bw, 28.0 * s), ok_label, true, ok_enabled) || (ok_enabled && ui.key(crate::ui::Key::Enter)) {
                if self.apply_dialog(&d) {
                    outcome = Outcome::Close;
                }
            }
            if ui.button(id_of("dlg_cancel"), Rect::new(x0 + bw + 8.0 * s, by, bw, 28.0 * s), t("Отмена", "Cancel"), false) {
                outcome = Outcome::Close;
            }
        } else if ui.button(id_of("dlg_cancel"), Rect::new(x0, by, w0, 28.0 * s), t("Отмена", "Cancel"), false) {
            outcome = Outcome::Close;
        }
        match outcome {
            Outcome::Keep => {
                if self.dialog.is_none() {
                    self.dialog = Some(d);
                }
            }
            Outcome::Close => {
                self.preview_cache = None;
                self.invalidate_view();
            }
        }
    }

    /// Применение диалога. Возвращает `true`, если диалог нужно закрыть.
    fn apply_dialog(&mut self, d: &Dialog) -> bool {
        match d {
            Dialog::Primitive { prim } => {
                self.create_primitive(*prim);
                true
            }
            Dialog::Extrude { sketch, distance, symmetric, op, target } => self.do_extrude(*sketch, *distance, *symmetric, *op, *target),
            Dialog::Combine { target, tool, op, keep } => {
                let bop = [BoolOp::Union, BoolOp::Subtract, BoolOp::Intersect][(*op).min(2)];
                self.do_combine(*target, *tool, bop, *keep)
            }
            Dialog::Mirror { plane, copy } => {
                self.do_mirror(*plane, *copy);
                true
            }
            Dialog::Reduce { id, percent } => {
                self.do_reduce(*id, *percent);
                true
            }
            _ => true,
        }
    }

    // -----------------------------------------------------------------------
    // Модальные окна
    // -----------------------------------------------------------------------

    pub(super) fn draw_modal_dialog(&mut self, ui: &mut Ui) {
        let Some(d) = self.dialog.take() else { return };
        if d.is_side_panel() {
            self.dialog = Some(d);
            return;
        }
        let s = ui.s;
        let (fw, fh) = (ui.fb.width as f32, ui.fb.height as f32);
        let old_layer = ui.st.layer;
        ui.st.layer = 4;
        ui.fb.reset_clip();
        let full = Rect::new(0.0, 0.0, fw, fh);
        ui.region(full);
        ui.fb.fill_rect(full, theme::MODAL_DIM());
        let (w, h) = match &d {
            Dialog::Settings => (500.0, 595.0),
            Dialog::Help => (640.0, 490.0),
            Dialog::Info { .. } => (420.0, 340.0),
            Dialog::Message { text, .. } => (440.0, 150.0 + 18.0 * (text.len() as f32 / 50.0).ceil()),
            _ => (440.0, 170.0),
        };
        let (w, h) = ((w * s).min(fw - 20.0), (h * s).min(fh - 20.0));
        let r = Rect::new((fw - w) * 0.5, (fh - h) * 0.5, w, h);
        ui.panel(r, 8.0);
        let title = match &d {
            Dialog::Settings => t("Настройки", "Settings").to_string(),
            Dialog::Help => t("Справка", "Help").to_string(),
            Dialog::Info { .. } => t("Анализ сетки", "Mesh analysis").to_string(),
            Dialog::ConfirmQuit | Dialog::ConfirmDiscard { .. } => t("Несохранённые изменения", "Unsaved changes").to_string(),
            Dialog::Message { title, .. } => title.clone(),
            _ => String::new(),
        };
        ui.text(r.x + 18.0 * s, r.y + 14.0 * s, &title, 15.0, theme::TEXT());
        if ui.icon_button(id_of("modal_close"), Rect::new(r.right() - 34.0 * s, r.y + 10.0 * s, 24.0 * s, 24.0 * s), Icon::Close, "", false, true) || ui.input.key_pressed(crate::ui::Key::Escape) {
            ui.st.layer = old_layer;
            if matches!(d, Dialog::Settings) {
                self.cfg.save();
            }
            return;
        }
        let x0 = r.x + 18.0 * s;
        let w0 = r.w - 36.0 * s;
        let mut y = r.y + 50.0 * s;
        let mut keep = true;
        let row = 32.0 * s;
        match &d {
            Dialog::Settings => {
                let lw = 170.0;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), "Язык / Language", lw);
                let mut lang = self.cfg.lang_en as usize;
                if ui.segmented(id_of("set_lang"), fr, &["Русский", "English"], &mut lang) {
                    self.cfg.lang_en = lang == 1;
                    super::set_lang_en(self.cfg.lang_en);
                }
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Цвет интерфейса", "Interface color"), lw);
                let mut th = self.cfg.dark_theme as usize;
                if ui.segmented(id_of("set_theme"), fr, &[t("Светлый", "Light"), t("Тёмный", "Dark")], &mut th) {
                    self.cfg.dark_theme = th == 1;
                    theme::set_dark(self.cfg.dark_theme);
                    self.invalidate_view();
                }
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Шрифт", "Font"), lw);
                let mut fi = self.cfg.engineering_font as usize;
                let note_x = fr.x;
                if ui.segmented(id_of("set_font"), fr, &[t("Стандартный", "Standard"), t("Инженерный", "Engineering")], &mut fi) {
                    self.cfg.engineering_font = fi == 1;
                    self.invalidate_view();
                }
                y += 24.0 * s;
                let font_note = if self.cfg.engineering_font && !self.font_source.starts_with("ISOCPEUR") {
                    format!("{} {}", t("ISOCPEUR не найден в системе — используется аналог", "ISOCPEUR not found — using the look-alike"), self.font_source)
                } else {
                    self.font_source.clone()
                };
                ui.text_in(Rect::new(note_x, y, x0 + w0 - note_x, 18.0 * s), &font_note, 11.0, theme::TEXT_DIM(), 0);
                y += 26.0 * s;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Масштаб интерфейса", "Interface scale"), lw);
                let scales = [0.9f32, 1.0, 1.25, 1.5];
                let mut si = scales.iter().position(|v| (v - self.cfg.ui_scale).abs() < 0.01).unwrap_or(1);
                if ui.segmented(id_of("set_scale"), fr, &["90%", "100%", "125%", "150%"], &mut si) {
                    self.cfg.ui_scale = scales[si];
                    self.invalidate_view();
                }
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Разрешение 3D", "3D resolution"), lw);
                let rss = [1.0f32, 0.75, 0.5];
                let mut ri = rss.iter().position(|v| (v - self.cfg.render_scale).abs() < 0.01).unwrap_or(0);
                if ui.segmented(id_of("set_rs"), fr, &["100%", "75%", "50%"], &mut ri) {
                    self.cfg.render_scale = rss[ri];
                    self.invalidate_view();
                }
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Расчёт графики", "Graphics rendering"), lw);
                let mut dev = self.cfg.gpu as usize;
                if ui.segmented(id_of("set_gpu"), fr, &[t("Процессор", "CPU"), t("Видеокарта", "GPU")], &mut dev) {
                    self.cfg.gpu = dev == 1;
                    if self.cfg.gpu {
                        // Повторная попытка инициализации при каждом включении.
                        self.gpu_error = None;
                    } else {
                        self.gpu = None;
                    }
                    self.invalidate_view();
                }
                y += 24.0 * s;
                let status = if !self.cfg.gpu {
                    t("Многопоточный программный рендер — работает на любом компьютере.", "Multithreaded software renderer — works on any computer.").to_string()
                } else if let Some(g) = &self.gpu {
                    format!("{} {}", t("Видеокарта:", "GPU:"), g.info)
                } else if let Some(e) = &self.gpu_error {
                    format!("{} {e}. {}", t("Видеокарта недоступна:", "GPU unavailable:"), t("Используется процессор.", "Using the CPU."))
                } else {
                    t("Видеокарта будет задействована при следующей отрисовке.", "The GPU will be used on the next redraw.").to_string()
                };
                let col = if self.cfg.gpu && self.gpu_error.is_some() { theme::DANGER() } else { theme::TEXT_DIM() };
                // Длинные сообщения (причина отказа драйвера) — в две строки.
                let mut lines: Vec<String> = vec![String::new()];
                for word in status.split(' ') {
                    let n_lines = lines.len();
                    let cur = lines.last_mut().unwrap();
                    let cand = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
                    if !cur.is_empty() && ui.text_width(&cand, 11.0) > w0 && n_lines < 2 {
                        let next = word.to_string();
                        lines.push(next);
                    } else {
                        *cur = cand;
                    }
                }
                for l in &lines {
                    ui.text_in(Rect::new(x0, y, w0, 18.0 * s), l, 11.0, col, 0);
                    y += 16.0 * s;
                }
                y += 10.0 * s;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Потоков рендера", "Render threads"), lw);
                let mut th = self.cfg.threads as f32;
                let fw2 = fr.w * 0.4;
                if ui.number_field(id_of("set_threads"), Rect::new(fr.x, fr.y, fw2, fr.h), &mut th, "", 1.0, 0.0, 64.0) {
                    self.cfg.threads = th as usize;
                    self.invalidate_view();
                }
                ui.text_in(Rect::new(fr.x + fw2 + 8.0 * s, fr.y, fr.w - fw2 - 8.0 * s, fr.h), &format!("0 = {} ({})", t("авто", "auto"), crate::renderer::software::default_threads()), 11.5, theme::TEXT_DIM(), 0);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Угол рёбер, °", "Edge angle, °"), lw);
                let mut cr = self.cfg.crease_deg;
                if ui.number_field(id_of("set_crease"), Rect::new(fr.x, fr.y, fr.w * 0.4, fr.h), &mut cr, "°", 5.0, 1.0, 89.0) {
                    self.cfg.crease_deg = cr;
                    self.invalidate_view();
                }
                y += row + 4.0 * s;
                let mut fast = self.cfg.fast_nav;
                if ui.checkbox(id_of("set_fast"), Rect::new(x0, y, w0, 24.0 * s), t("Быстрая навигация (меньше разрешение при вращении)", "Fast navigation (lower resolution while orbiting)"), &mut fast) {
                    self.cfg.fast_nav = fast;
                }
                y += 28.0 * s;
                let mut dark = self.cfg.dark_bg;
                if ui.checkbox(id_of("set_dark"), Rect::new(x0, y, w0, 24.0 * s), t("Тёмный фон 3D-вида", "Dark 3D background"), &mut dark) {
                    self.cfg.dark_bg = dark;
                    self.invalidate_view();
                }
                y += 28.0 * s;
                let mut snap = self.cfg.snap;
                if ui.checkbox(id_of("set_snap"), Rect::new(x0, y, w0, 24.0 * s), t("Привязка (сетка, шаг манипуляторов)", "Snapping (grid, manipulator steps)"), &mut snap) {
                    self.cfg.snap = snap;
                }
                y += 28.0 * s;
                let mut grid = self.cfg.show_grid;
                if ui.checkbox(id_of("set_grid"), Rect::new(x0, y, w0, 24.0 * s), t("Показывать сетку", "Show grid"), &mut grid) {
                    self.cfg.show_grid = grid;
                    self.invalidate_view();
                }
                if ui.button(id_of("set_done"), Rect::new(r.right() - 130.0 * s, r.bottom() - 44.0 * s, 112.0 * s, 28.0 * s), t("Готово", "Done"), true) {
                    self.cfg.save();
                    keep = false;
                }
            }
            Dialog::Help => {
                let col_w = w0 * 0.5;
                let items: [(&str, &str); 20] = [
                    ("Ctrl+N / O / S", t("Новый / открыть / сохранить", "New / open / save")),
                    ("Ctrl+I", t("Импорт модели в проект", "Import a model")),
                    ("Ctrl+E", t("Экспорт STL", "Export STL")),
                    ("Ctrl+Z / Ctrl+Y", t("Отменить / повторить", "Undo / redo")),
                    ("Ctrl+D, Del", t("Дублировать, удалить", "Duplicate, delete")),
                    ("F, Home", t("Вписать, начальный вид", "Fit, home view")),
                    ("1 / 3 / 7 / 0", t("Спереди / справа / сверху / изо", "Front / right / top / iso")),
                    ("W, G, P", t("Стиль, сетка, перспектива", "Style, grid, perspective")),
                    ("V, M, R, S", t("Выбор, перемещ., поворот, масштаб", "Select, move, rotate, scale")),
                    ("E, I", t("Выдавить, измерить", "Extrude, measure")),
                    (t("ЛКМ (тянуть)", "LMB drag"), t("Вращение вида", "Orbit")),
                    (t("ПКМ / СКМ", "RMB / MMB"), t("Сдвиг вида", "Pan")),
                    (t("Shift+ЛКМ", "Shift+LMB"), t("Сдвиг вида", "Pan")),
                    (t("Колесо", "Wheel"), t("Масштаб к курсору", "Zoom to cursor")),
                    (t("Эскиз: L, R, C", "Sketch: L, R, C"), t("Линия, прямоуг., окружность", "Line, rectangle, circle")),
                    (t("Эскиз: Enter", "Sketch: Enter"), t("Завершить эскиз", "Finish sketch")),
                    ("Shift", t("Без привязки", "Disable snapping")),
                    ("Esc", t("Отмена / снять выбор", "Cancel / deselect")),
                    (t("Поля", "Fields"), t("Выражения: 10+5*2, pi*r", "Expressions: 10+5*2, pi*r")),
                    ("F1", t("Эта справка", "This help")),
                ];
                for (i, (k, v)) in items.iter().enumerate() {
                    let cx = x0 + if i < 10 { 0.0 } else { col_w };
                    let cy = y + (i % 10) as f32 * 26.0 * s;
                    let kw = ui.text_width(k, 11.5) + 12.0 * s;
                    let kr = Rect::new(cx, cy + 2.0 * s, kw, 20.0 * s);
                    ui.fb.fill_rounded(kr, 3.0 * s, theme::PANEL_HEADER());
                    ui.text_in(kr, k, 11.5, theme::TEXT(), 1);
                    ui.text_in(Rect::new(cx + kw + 8.0 * s, cy, col_w - kw - 14.0 * s, 24.0 * s), v, 11.5, theme::TEXT_DIM(), 0);
                }
                y += 10.0 * 26.0 * s + 14.0 * s;
                ui.fb.fill_rect(Rect::new(x0, y, w0, 1.0), theme::TOOLBAR_BORDER());
                y += 12.0 * s;
                let about = [
                    format!("AlphaCad {} — {}", crate::VERSION, t("лёгкий 3D CAD-редактор на Rust", "a lightweight 3D CAD editor in Rust")),
                    t("Форматы: STL (двоичный/ASCII), STEP AP203/AP214, 3MF, OBJ, PLY, проект .acad", "Formats: STL (binary/ASCII), STEP AP203/AP214, 3MF, OBJ, PLY, .acad project").to_string(),
                    t("Рендер на процессоре (многопоточный) или на видеокарте (OpenGL).", "Rendering on the CPU (multithreaded) or on the GPU (OpenGL).").to_string(),
                    t("Независимая реализация (clean room); не связана с Autodesk.", "Independent clean-room implementation; not affiliated with Autodesk.").to_string(),
                ];
                // Логотип слева от описания.
                let ls = 76.0 * s;
                crate::ui::icons::draw_logo(ui.fb, Rect::new(x0, y + 2.0 * s, ls, ls));
                let tx = x0 + ls + 16.0 * s;
                for (i, l) in about.iter().enumerate() {
                    ui.text(tx, y, l, if i == 0 { 12.5 } else { 11.5 }, if i == 0 { theme::TEXT() } else { theme::TEXT_DIM() });
                    y += 20.0 * s;
                }
                // Подпись автора — в самом низу окна справки.
                let sig = "Designed by Alex Abmaev";
                let sy = r.bottom() - 30.0 * s;
                ui.fb.fill_rect(Rect::new(x0, sy - 10.0 * s, w0, 1.0), theme::TOOLBAR_BORDER());
                let sw = ui.text_width(sig, 12.0);
                ui.text(r.x + (r.w - sw) * 0.5, sy, sig, 12.0, theme::TEXT_DIM());
            }
            Dialog::Info { id } => {
                if let Some(idx) = self.scene.index_of(*id) {
                    let b = &self.scene.bodies[idx];
                    let mut m = b.mesh.clone();
                    m.transform(b.transform);
                    let size = m.aabb().size();
                    let mm = t("мм", "mm");
                    let lines = [
                        (t("Тело", "Body"), b.name.clone()),
                        (t("Треугольников", "Triangles"), fmt_count(b.mesh.triangle_count())),
                        (t("Вершин", "Vertices"), fmt_count(b.mesh.vertex_count())),
                        (t("Габариты", "Size"), format!("{} × {} × {} {mm}", fmt_num(size.x), fmt_num(size.y), fmt_num(size.z))),
                        (t("Объём", "Volume"), format!("{} {mm}³", fmt_num(m.volume().abs()))),
                        (t("Площадь", "Area"), format!("{} {mm}²", fmt_num(m.surface_area()))),
                        (t("Открытых рёбер", "Open edges"), b.mesh.boundary_edge_count().to_string()),
                        (t("Центр масс", "Center of mass"), {
                            let c = m.center_of_mass();
                            format!("{}, {}, {}", fmt_num(c.x), fmt_num(c.y), fmt_num(c.z))
                        }),
                    ];
                    for (k, v) in lines.iter() {
                        let fr = ui.labeled(Rect::new(x0, y, w0, 24.0 * s), k, 150.0);
                        ui.text_in(fr, v, 12.5, theme::TEXT(), 0);
                        y += 26.0 * s;
                    }
                    let bw = (w0 - 8.0 * s) * 0.5;
                    let by = r.bottom() - 44.0 * s;
                    if ui.button(id_of("info_weld"), Rect::new(x0, by, bw, 28.0 * s), t("Сварить вершины", "Merge vertices"), false) {
                        self.run_action(Action::Weld);
                    }
                    if ui.button(id_of("info_fix"), Rect::new(x0 + bw + 8.0 * s, by, bw, 28.0 * s), t("Исправить ориентацию", "Fix orientation"), false) {
                        self.run_action(Action::FixOrientation);
                    }
                } else {
                    keep = false;
                }
            }
            Dialog::ConfirmQuit | Dialog::ConfirmDiscard { .. } => {
                ui.text(x0, y, t("Сохранить изменения в проекте?", "Save changes to the project?"), 12.5, theme::TEXT());
                let by = r.bottom() - 44.0 * s;
                let bw = (w0 - 16.0 * s) / 3.0;
                let then = match &d {
                    Dialog::ConfirmDiscard { then } => Some(*then),
                    _ => None,
                };
                if ui.button(id_of("cq_save"), Rect::new(x0, by, bw, 28.0 * s), t("Сохранить", "Save"), true) {
                    keep = false;
                    if self.save_project(false) {
                        match then {
                            Some(a) => {
                                self.saved_revision = self.scene.revision;
                                self.run_action(a);
                            }
                            None => {
                                self.cfg.save();
                                self.quit = true;
                            }
                        }
                    }
                }
                if ui.button(id_of("cq_discard"), Rect::new(x0 + bw + 8.0 * s, by, bw, 28.0 * s), t("Не сохранять", "Don't save"), false) {
                    keep = false;
                    match then {
                        Some(a) => {
                            self.saved_revision = self.scene.revision;
                            self.run_action(a);
                        }
                        None => {
                            self.cfg.save();
                            self.quit = true;
                        }
                    }
                }
                if ui.button(id_of("cq_cancel"), Rect::new(x0 + 2.0 * (bw + 8.0 * s), by, bw, 28.0 * s), t("Отмена", "Cancel"), false) {
                    keep = false;
                }
            }
            Dialog::Message { text, .. } => {
                for (i, line) in wrap(ui, text, 12.0, w0).iter().enumerate() {
                    ui.text(x0, y + i as f32 * 18.0 * s, line, 12.0, theme::TEXT());
                }
                if ui.button(id_of("msg_ok"), Rect::new(r.right() - 110.0 * s, r.bottom() - 44.0 * s, 92.0 * s, 28.0 * s), "OK", true) || ui.input.key_pressed(crate::ui::Key::Enter) {
                    keep = false;
                }
            }
            _ => {}
        }
        ui.st.layer = old_layer;
        if keep && self.dialog.is_none() && !self.quit {
            self.dialog = Some(d);
        }
    }
}

/// Перенос строк по ширине.
fn wrap(ui: &Ui, text: &str, size: f32, w: f32) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.lines() {
        let mut line = String::new();
        for word in para.split(' ') {
            let cand = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if ui.text_width(&cand, size) > w && !line.is_empty() {
                out.push(std::mem::take(&mut line));
                line = word.to_string();
            } else {
                line = cand;
            }
        }
        out.push(line);
    }
    out
}

pub fn prim_icon(p: &Primitive) -> Icon {
    match p {
        Primitive::Box { .. } => Icon::Box,
        Primitive::Cylinder { .. } => Icon::Cylinder,
        Primitive::Sphere { .. } => Icon::Sphere,
        Primitive::Cone { .. } => Icon::Cone,
        Primitive::Torus { .. } => Icon::Torus,
        Primitive::Capsule { .. } => Icon::Capsule,
        Primitive::Tube { .. } => Icon::Tube,
        Primitive::Wedge { .. } => Icon::Wedge,
        Primitive::Pyramid { .. } => Icon::Pyramid,
    }
}
