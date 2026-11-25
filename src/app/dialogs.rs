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
    /// Новое тело-примитив. `place` — указанная щелчком точка; пока её нет,
    /// тело следует за курсором (с привязкой к модели и сетке).
    Primitive { prim: Primitive, place: Option<super::snap::ModelSnap> },
    /// Выбор плоскости эскиза; `offset` — смещение вдоль нормали.
    SketchPlane { offset: f32 },
    Extrude { sketch: u64, distance: f32, symmetric: bool, op: usize, target: u64 },
    Combine { target: u64, tool: u64, op: usize, keep: bool },
    Mirror { plane: usize, copy: bool },
    Reduce { id: u64, percent: f32 },
    /// Вращение профиля. `axis`: 0 — X эскиза, 1 — Y эскиза, 2 — отрезок эскиза.
    Revolve { sketch: u64, angle: f32, axis: usize, op: usize, target: u64 },
    /// Сдвиг профиля по траектории другого эскиза.
    Sweep { profile: u64, path: u64, twist: f32, scale: f32, op: usize, target: u64 },
    /// Лофт через 2–3 сечения (`c == 0` — без третьего).
    Loft { a: u64, b: u64, c: u64, op: usize, target: u64 },
    /// Оболочка (Fusion) / толщина (Blender Solidify).
    Shell { id: u64, thickness: f32, outward: bool, solidify: bool },
    /// Массив: линейный или круговой.
    Pattern { id: u64, circular: bool, count: f32, spacing: f32, angle: f32, axis: usize, join: bool },
    /// Деформация: 0 — скручивание, 1 — сужение, 2 — изгиб.
    Deform { id: u64, kind: usize, axis: usize, angle: f32, scale: f32 },
    /// Подразделение поверхности.
    Subdivide { id: u64, levels: f32, smooth: bool },
    /// Сглаживание вершин.
    Smooth { id: u64, iterations: f32, factor: f32, taubin: bool },
    /// Пружина.
    Coil { radius: f32, pitch: f32, turns: f32, wire: f32 },
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
        !matches!(self, Dialog::Info { .. } | Dialog::Settings | Dialog::Help | Dialog::ConfirmQuit | Dialog::ConfirmDiscard { .. } | Dialog::Message { .. })
    }

    /// Диалог операции моделирования (см. [`Dialog::feature_result`]).
    pub fn is_feature(&self) -> bool {
        matches!(
            self,
            Dialog::Revolve { .. } | Dialog::Sweep { .. } | Dialog::Loft { .. } | Dialog::Shell { .. } | Dialog::Pattern { .. } | Dialog::Deform { .. } | Dialog::Subdivide { .. } | Dialog::Smooth { .. } | Dialog::Coil { .. }
        )
    }

    /// Тело, которое диалог изменяет на месте (скрывается на время предпросмотра).
    pub fn hides_body(&self) -> Option<u64> {
        match self {
            Dialog::Shell { id, .. } | Dialog::Deform { id, .. } | Dialog::Subdivide { id, .. } | Dialog::Smooth { id, .. } => Some(*id),
            _ => None,
        }
    }

    /// Результат операций моделирования (Revolve, Sweep, Loft, Shell, Pattern,
    /// деформации, подразделение, сглаживание, пружина): меш и его трансформация.
    /// `None` — диалог не из этого семейства.
    pub fn feature_result(&self, st: &State) -> Option<Result<(Mesh, Mat4), String>> {
        use crate::modeling as md;
        let world_of = |id: u64| -> Result<Mesh, String> {
            let b = st.scene.body(id).ok_or_else(|| "выберите тело".to_string())?;
            if b.mesh.is_empty() {
                return Err("выберите тело".into());
            }
            let mut m = b.mesh.clone();
            m.transform(b.transform);
            Ok(m)
        };
        let sketch_of = |id: u64| -> Result<(&crate::sketch::Sketch, Mat4), String> {
            let b = st.scene.body(id).ok_or_else(|| "выберите эскиз".to_string())?;
            b.sketch.as_ref().map(|s| (s, b.transform)).ok_or_else(|| "выберите эскиз".to_string())
        };
        let id = Mat4::identity();
        Some(match self {
            Dialog::Revolve { sketch, angle, axis, .. } => (|| {
                let (sk, tf) = sketch_of(*sketch)?;
                let ax = md::sketch_axis(sk, *axis).ok_or_else(|| "в эскизе нет отдельного отрезка для оси".to_string())?;
                md::revolve(sk, ax, *angle).map(|m| (m, tf))
            })(),
            Dialog::Sweep { profile, path, twist, scale, .. } => (|| {
                if profile == path {
                    return Err("профиль и траектория должны быть разными эскизами".into());
                }
                let (sk, tf) = sketch_of(*profile)?;
                let (pk, ptf) = sketch_of(*path)?;
                let (pts, closed) = pk.longest_path().ok_or_else(|| "в эскизе траектории нет линий".to_string())?;
                let world: Vec<crate::math::Vec3> = pts.iter().map(|p| ptf.transform_point(pk.plane.to_world(*p))).collect();
                md::sweep(sk, tf, &world, closed, *twist, scale / 100.0).map(|m| (m, id))
            })(),
            Dialog::Loft { a, b, c, .. } => (|| {
                let mut secs = Vec::new();
                for sid in [*a, *b, *c] {
                    if sid == 0 {
                        continue;
                    }
                    let (sk, tf) = sketch_of(sid)?;
                    secs.push(md::sketch_section(sk, tf).ok_or_else(|| "в одном из эскизов нет замкнутого контура".to_string())?);
                }
                if [*a, *b, *c].iter().filter(|x| **x != 0).collect::<std::collections::HashSet<_>>().len() != secs.len() {
                    return Err("сечения должны быть разными эскизами".into());
                }
                md::loft(&secs).map(|m| (m, id))
            })(),
            Dialog::Shell { id: bid, thickness, outward, .. } => world_of(*bid).and_then(|m| md::solidify(&m, if *outward { -thickness } else { *thickness }).map(|m| (m, id))),
            Dialog::Pattern { id: bid, circular, count, spacing, angle, axis, .. } => world_of(*bid).map(|m| {
                let mut out = Mesh::new();
                for tf in pattern_transforms(*circular, *count, *spacing, *angle, *axis).into_iter().skip(1) {
                    let mut c = m.clone();
                    c.transform(tf);
                    out.merge(&c);
                }
                out.compute_smooth_normals();
                (out, id)
            }),
            Dialog::Deform { id: bid, kind, axis, angle, scale } => world_of(*bid).map(|m| {
                let size = m.aabb().size();
                let dense = md::refine(&m, size.x.max(size.y).max(size.z) / 40.0, 400_000);
                let (k, amount) = match kind {
                    0 => (md::DeformKind::Twist, *angle),
                    1 => (md::DeformKind::Taper, scale / 100.0),
                    _ => (md::DeformKind::Bend, *angle),
                };
                (md::deform(&dense, k, *axis, amount), id)
            }),
            Dialog::Subdivide { id: bid, levels, smooth } => world_of(*bid).and_then(|m| {
                let lv = levels.round().clamp(1.0, 4.0) as u32;
                if m.triangle_count().saturating_mul(4usize.pow(lv)) > 2_000_000 {
                    return Err("слишком много треугольников — уменьшите число уровней".into());
                }
                let mut r = m;
                for _ in 0..lv {
                    r = md::subdivide(&r, *smooth);
                }
                Ok((r, id))
            }),
            Dialog::Smooth { id: bid, iterations, factor, taubin } => world_of(*bid).map(|m| (md::smooth(&m, iterations.round().clamp(1.0, 200.0) as usize, *factor, *taubin), id)),
            Dialog::Coil { radius, pitch, turns, wire } => md::coil(*radius, *pitch, *turns, *wire).map(|m| {
                let tf = st.placement_for(&m);
                (m, tf)
            }),
            _ => return None,
        })
    }

    /// Ключ предпросмотра (изменение — пересчёт меша предпросмотра).
    pub fn preview_key(&self, st: &State) -> Option<u64> {
        let mut h = DefaultHasher::new();
        match self {
            Dialog::Primitive { prim, place } => (format!("p{prim:?}{place:?}{:?}", place.or(st.snap_hover)), st.scene.revision).hash(&mut h),
            Dialog::Extrude { sketch, distance, symmetric, .. } => {
                (sketch, distance.to_bits(), symmetric).hash(&mut h);
                st.scene.revision.hash(&mut h);
            }
            Dialog::Mirror { plane, .. } => {
                ("m", plane, st.scene.selected, st.scene.revision).hash(&mut h);
            }
            Dialog::Revolve { .. } | Dialog::Sweep { .. } | Dialog::Loft { .. } | Dialog::Shell { .. } | Dialog::Pattern { .. } | Dialog::Deform { .. } | Dialog::Subdivide { .. } | Dialog::Smooth { .. } | Dialog::Coil { .. } => {
                // Операция и тело-цель на результат не влияют.
                let mut d = self.clone();
                match &mut d {
                    Dialog::Revolve { op, target, .. } | Dialog::Sweep { op, target, .. } | Dialog::Loft { op, target, .. } => {
                        *op = 0;
                        *target = 0;
                    }
                    Dialog::Pattern { join, .. } => *join = false,
                    _ => {}
                }
                (format!("{d:?}"), st.scene.revision).hash(&mut h);
                if matches!(d, Dialog::Coil { .. }) {
                    st.scene.bodies.len().hash(&mut h);
                }
            }
            _ => return None,
        }
        Some(h.finish() | 1)
    }

    /// Меш предпросмотра и его трансформация.
    pub fn build_preview(&self, st: &State) -> Option<(Mesh, Mat4)> {
        match self {
            Dialog::Primitive { prim, place } => {
                let m = prim.build();
                let tf = st.primitive_placement(&m, *place);
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
            _ => {
                let key = self.preview_key(st).unwrap_or(0);
                match self.feature_result(st)? {
                    Ok(r) => {
                        *st.preview_error.borrow_mut() = None;
                        Some(r)
                    }
                    Err(e) => {
                        *st.preview_error.borrow_mut() = Some((key, e));
                        None
                    }
                }
            }
        }
    }
}

/// Трансформации копий массива (первая — исходное положение).
pub fn pattern_transforms(circular: bool, count: f32, spacing: f32, angle: f32, axis: usize) -> Vec<Mat4> {
    let n = count.round().clamp(1.0, 500.0) as usize;
    let dir = match axis {
        0 => crate::math::Vec3::new(1.0, 0.0, 0.0),
        1 => crate::math::Vec3::new(0.0, 1.0, 0.0),
        _ => crate::math::Vec3::new(0.0, 0.0, 1.0),
    };
    (0..n)
        .map(|i| {
            if circular {
                // Полный круг — копии без повтора первой; иначе — включая конец дуги.
                let full = (angle.abs() - 360.0).abs() < 1e-3;
                let step = if full || n < 2 { angle / n as f32 } else { angle / (n - 1) as f32 };
                Mat4::rotation_axis(dir, (step * i as f32) * crate::math::DEG2RAD)
            } else {
                Mat4::translation(dir * (spacing * i as f32))
            }
        })
        .collect()
}

/// Перевод сообщений об ошибках моделирования.
pub fn model_error_text(e: &str) -> String {
    if !super::lang_en() {
        let mut c = e.chars();
        return match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => String::new(),
        };
    }
    match e {
        "в эскизе нет замкнутого контура" => "The sketch has no closed profile",
        "угол должен быть больше нуля" => "The angle must be non-zero",
        "профиль пересекает ось вращения" => "The profile crosses the rotation axis",
        "не удалось построить тело" => "Could not build the body",
        "в эскизе профиля нет замкнутого контура" => "The profile sketch has no closed profile",
        "траектория слишком короткая" => "The path is too short",
        "плоскость профиля должна пересекать траекторию поперёк" => "The profile plane must cross the path",
        "нужно минимум два сечения" => "At least two sections are required",
        "каждое сечение должно быть замкнутым контуром" => "Every section must be a closed profile",
        "сечения лежат в одном месте — разнесите их по высоте" => "Sections coincide — move them apart",
        "толщина должна быть больше нуля" => "Thickness must be positive",
        "пустая сетка" => "Empty mesh",
        "параметры должны быть больше нуля" => "Parameters must be positive",
        "радиус проволоки должен быть меньше радиуса витка" => "Wire radius must be smaller than the coil radius",
        "шаг меньше толщины проволоки — витки пересекаются" => "Pitch is smaller than the wire — turns overlap",
        "не удалось построить пружину" => "Could not build the coil",
        "выберите тело" => "Select a body",
        "выберите эскиз" => "Select a sketch",
        "в эскизе нет отдельного отрезка для оси" => "The sketch has no standalone line for the axis",
        "профиль и траектория должны быть разными эскизами" => "Profile and path must be different sketches",
        "в эскизе траектории нет линий" => "The path sketch has no curves",
        "в одном из эскизов нет замкнутого контура" => "One of the sketches has no closed profile",
        "сечения должны быть разными эскизами" => "Sections must be different sketches",
        "слишком много треугольников — уменьшите число уровней" => "Too many triangles — use fewer levels",
        other => other,
    }
    .to_string()
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

    fn sketch_ids(&self) -> Vec<(u64, String)> {
        self.scene.bodies.iter().filter(|b| b.sketch.is_some()).map(|b| (b.id, b.name.clone())).collect()
    }

    /// Строки «Операция» и «Тело» для операций, создающих тело.
    #[allow(clippy::too_many_arguments)]
    fn op_rows(ui: &mut Ui, x0: f32, y: &mut f32, w0: f32, row: f32, solids: &[(u64, String)], op: &mut usize, target: &mut u64, key: &str) {
        let s = ui.s;
        let fr = ui.labeled(Rect::new(x0, *y, w0, 26.0 * s), t("Операция", "Operation"), 100.0);
        let mut o = *op;
        if ui.segmented(id_of(&format!("{key}_op")), fr, &[t("Новое", "New"), t("Соед.", "Join"), t("Вырез.", "Cut"), t("Перес.", "Inter.")], &mut o) {
            *op = if solids.is_empty() { 0 } else { o };
        }
        *y += row;
        if *op != 0 {
            if !solids.iter().any(|(i, _)| i == target) {
                *target = solids.first().map(|(i, _)| *i).unwrap_or(0);
            }
            let fr = ui.labeled(Rect::new(x0, *y, w0, 26.0 * s), t("Тело", "Body"), 100.0);
            State::body_combo(ui, id_of(&format!("{key}_target")), fr, solids, target);
            *y += row;
        }
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
            Dialog::Primitive { prim, .. } => (State::primitive_name(prim).to_string(), prim_icon(prim), prim_param_count(prim) as f32 + 2.6),
            Dialog::SketchPlane { .. } => (t("Создать эскиз", "Create sketch").into(), Icon::Sketch, 8.6),
            Dialog::Extrude { op, .. } => (t("Выдавить", "Extrude").into(), Icon::Extrude, if *op == 0 { 5.0 } else { 6.0 }),
            Dialog::Combine { .. } => (t("Объединить", "Combine").into(), Icon::Union, 5.5),
            Dialog::Mirror { .. } => (t("Зеркало", "Mirror").into(), Icon::Mirror, 3.5),
            Dialog::Reduce { .. } => (t("Упростить сетку", "Reduce mesh").into(), Icon::Reduce, 4.0),
            Dialog::Revolve { op, .. } => (t("Вращение", "Revolve").into(), Icon::Revolve, if *op == 0 { 6.0 } else { 7.0 }),
            Dialog::Sweep { op, .. } => (t("Сдвиг по траектории", "Sweep").into(), Icon::Sweep, if *op == 0 { 7.0 } else { 8.0 }),
            Dialog::Loft { op, .. } => (t("Лофт", "Loft").into(), Icon::Loft, if *op == 0 { 6.0 } else { 7.0 }),
            Dialog::Shell { solidify, .. } => (if *solidify { t("Толщина", "Solidify") } else { t("Оболочка", "Shell") }.into(), if *solidify { Icon::Solidify } else { Icon::Shell }, 5.0),
            Dialog::Pattern { .. } => (t("Массив", "Pattern").into(), Icon::Pattern, 7.0),
            Dialog::Deform { kind, .. } => (
                [t("Скручивание", "Twist"), t("Сужение", "Taper"), t("Изгиб", "Bend")][(*kind).min(2)].into(),
                [Icon::Twist, Icon::Taper, Icon::Bend][(*kind).min(2)],
                5.0,
            ),
            Dialog::Subdivide { .. } => (t("Подразделить", "Subdivide").into(), Icon::Subdivide, 4.5),
            Dialog::Smooth { .. } => (t("Сгладить", "Smooth").into(), Icon::Smooth, 5.0),
            Dialog::Coil { .. } => (t("Пружина", "Coil").into(), Icon::Coil, 6.0),
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
            Dialog::Primitive { prim, place } => {
                let (l1, l2) = match place {
                    Some(sn) => (
                        format!("{} {}: {}", t("Точка указана", "Point set"), sn.kind.label(), fmt_vec(sn.point)),
                        t("Щелчок в виде — снова за курсором", "Click the view to move it again").to_string(),
                    ),
                    None => (
                        t("Тело следует за курсором — щёлкните точку", "The body follows the cursor — click a point").to_string(),
                        t("Привязка: вершины, рёбра, грани, сетка", "Snaps to vertices, edges, faces, grid").to_string(),
                    ),
                };
                ui.text_in(Rect::new(x0, y, w0, 20.0 * s), &l1, 11.5, if place.is_some() { theme::ACCENT() } else { theme::TEXT() }, 0);
                y += 22.0 * s;
                ui.text_in(Rect::new(x0, y, w0, 20.0 * s), &l2, 11.0, theme::TEXT_DIM(), 0);
                y += 22.0 * s;
                if place.is_none() {
                    ui.text_in(Rect::new(x0, y, w0, 20.0 * s), t("Shift — без привязки", "Shift — no snapping"), 11.0, theme::TEXT_DIM(), 0);
                }
                y += row;
                self.invalidate_view();
                edit_primitive(ui, prim, x0, &mut y, w0, row, "dlg_prim");
            }
            Dialog::SketchPlane { offset } => {
                show_ok = false;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Смещение", "Offset"), 100.0);
                ui.number_field(id_of("dlg_plane_off"), fr, offset, t("мм", "mm"), 5.0, -1e5, 1e5);
                y += row;
                ui.text_in(Rect::new(x0, y, w0, 20.0 * s), t("Выберите плоскость эскиза:", "Choose the sketch plane:"), 12.0, theme::TEXT(), 0);
                y += row;
                let off = *offset;
                let planes = [
                    (Some(crate::sketch::SketchPlane::Xy), t("XY — вид сверху", "XY — top"), Icon::Plane),
                    (Some(crate::sketch::SketchPlane::Xz), t("XZ — вид спереди", "XZ — front"), Icon::Plane),
                    (Some(crate::sketch::SketchPlane::Yz), t("YZ — вид справа", "YZ — right"), Icon::Plane),
                    (None, t("По текущему виду", "Current view"), Icon::ViewPlane),
                ];
                for (i, (pl, name, icon)) in planes.iter().enumerate() {
                    let br = Rect::new(x0, y, w0, 34.0 * s);
                    let id = crate::ui::id_idx(id_of("dlg_plane"), i);
                    let (hov, _, clicked) = ui.interact(id, br);
                    ui.fb.fill_rounded(br, 4.0 * s, if hov { theme::HOVER() } else { theme::FIELD() });
                    ui.fb.stroke_rounded(br, 4.0 * s, 1.0, if hov { theme::ACCENT() } else { theme::FIELD_BORDER() });
                    ui.icon(*icon, Rect::new(br.x + 8.0 * s, br.y + 5.0 * s, 24.0 * s, 24.0 * s), theme::TEXT());
                    ui.text_in(Rect::new(br.x + 40.0 * s, br.y, br.w - 44.0 * s, br.h), name, 12.5, theme::TEXT(), 0);
                    if clicked {
                        let plane = match pl {
                            Some(pl) => crate::sketch::SketchPlane::offset(*pl, off),
                            None => self.view_sketch_plane(off),
                        };
                        self.begin_sketch(plane, None);
                        return;
                    }
                    y += 40.0 * s;
                }
                ui.text_in(Rect::new(x0, y, w0, 20.0 * s), t("…или щёлкните грань модели в виде", "…or click a model face in the view"), 11.5, theme::TEXT_DIM(), 0);
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
            Dialog::Revolve { sketch, angle, axis, op, target } => {
                let sketches = self.sketch_ids();
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Профиль", "Profile"), 100.0);
                State::body_combo(ui, id_of("dlg_rv_sketch"), fr, &sketches, sketch);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Ось", "Axis"), 100.0);
                ui.segmented(id_of("dlg_rv_axis"), fr, &["X", "Y", t("Отрезок", "Line")], axis);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Угол", "Angle"), 100.0);
                ui.number_field(id_of("dlg_rv_angle"), fr, angle, "°", 15.0, -360.0, 360.0);
                y += row;
                Self::op_rows(ui, x0, &mut y, w0, row, &solids, op, target, "dlg_rv");
            }
            Dialog::Sweep { profile, path, twist, scale, op, target } => {
                let sketches = self.sketch_ids();
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Профиль", "Profile"), 100.0);
                State::body_combo(ui, id_of("dlg_sw_prof"), fr, &sketches, profile);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Траектория", "Path"), 100.0);
                State::body_combo(ui, id_of("dlg_sw_path"), fr, &sketches, path);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Закрутка", "Twist"), 100.0);
                ui.number_field(id_of("dlg_sw_twist"), fr, twist, "°", 15.0, -3600.0, 3600.0);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Масштаб конца", "End scale"), 100.0);
                ui.number_field(id_of("dlg_sw_scale"), fr, scale, "%", 10.0, 1.0, 1000.0);
                y += row;
                Self::op_rows(ui, x0, &mut y, w0, row, &solids, op, target, "dlg_sw");
            }
            Dialog::Loft { a, b, c, op, target } => {
                let sketches = self.sketch_ids();
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Сечение 1", "Section 1"), 100.0);
                State::body_combo(ui, id_of("dlg_lf_a"), fr, &sketches, a);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Сечение 2", "Section 2"), 100.0);
                State::body_combo(ui, id_of("dlg_lf_b"), fr, &sketches, b);
                y += row;
                let mut with_none = vec![(0u64, t("— нет —", "— none —").to_string())];
                with_none.extend(sketches.iter().cloned());
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Сечение 3", "Section 3"), 100.0);
                State::body_combo(ui, id_of("dlg_lf_c"), fr, &with_none, c);
                y += row;
                Self::op_rows(ui, x0, &mut y, w0, row, &solids, op, target, "dlg_lf");
            }
            Dialog::Shell { id, thickness, outward, solidify } => {
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Тело", "Body"), 100.0);
                State::body_combo(ui, id_of("dlg_sh_body"), fr, &solids, id);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Толщина", "Thickness"), 100.0);
                ui.number_field(id_of("dlg_sh_t"), fr, thickness, t("мм", "mm"), 0.5, 0.01, 1e4);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Направление", "Direction"), 100.0);
                let mut o = *outward as usize;
                if ui.segmented(id_of("dlg_sh_dir"), fr, &[t("Внутрь", "Inside"), t("Наружу", "Outside")], &mut o) {
                    *outward = o == 1;
                }
                y += row;
                let hint = if *solidify { t("Придаёт толщину поверхности; края закрываются торцами", "Adds thickness to a surface; open edges get rims") } else { t("Делает тело полым со стенкой заданной толщины", "Hollows the body with the given wall thickness") };
                ui.text_in(Rect::new(x0, y, w0, 18.0 * s), hint, 11.0, theme::TEXT_DIM(), 0);
                y += row;
            }
            Dialog::Pattern { id, circular, count, spacing, angle, axis, join } => {
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Тело", "Body"), 100.0);
                State::body_combo(ui, id_of("dlg_pt_body"), fr, &solids, id);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Тип", "Type"), 100.0);
                let mut c = *circular as usize;
                if ui.segmented(id_of("dlg_pt_type"), fr, &[t("Линейный", "Linear"), t("Круговой", "Circular")], &mut c) {
                    *circular = c == 1;
                }
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Ось", "Axis"), 100.0);
                ui.segmented(id_of("dlg_pt_axis"), fr, &["X", "Y", "Z"], axis);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Количество", "Count"), 100.0);
                ui.number_field(id_of("dlg_pt_count"), fr, count, t("шт", "pcs"), 1.0, 1.0, 500.0);
                *count = count.round();
                y += row;
                if *circular {
                    let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Угол", "Angle"), 100.0);
                    ui.number_field(id_of("dlg_pt_angle"), fr, angle, "°", 15.0, -360.0, 360.0);
                } else {
                    let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Шаг", "Spacing"), 100.0);
                    ui.number_field(id_of("dlg_pt_spacing"), fr, spacing, t("мм", "mm"), 1.0, -1e5, 1e5);
                }
                y += row;
                ui.checkbox(id_of("dlg_pt_join"), Rect::new(x0, y, w0, 24.0 * s), t("Объединить в одно тело", "Merge into one body"), join);
                y += row;
            }
            Dialog::Deform { id, kind, axis, angle, scale } => {
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Тело", "Body"), 100.0);
                State::body_combo(ui, id_of("dlg_df_body"), fr, &solids, id);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Вид", "Kind"), 100.0);
                ui.segmented(id_of("dlg_df_kind"), fr, &[t("Скрутка", "Twist"), t("Сужение", "Taper"), t("Изгиб", "Bend")], kind);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Ось", "Axis"), 100.0);
                ui.segmented(id_of("dlg_df_axis"), fr, &["X", "Y", "Z"], axis);
                y += row;
                if *kind == 1 {
                    let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Масштаб конца", "End scale"), 100.0);
                    ui.number_field(id_of("dlg_df_scale"), fr, scale, "%", 10.0, 0.0, 1000.0);
                } else {
                    let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Угол", "Angle"), 100.0);
                    ui.number_field(id_of("dlg_df_angle"), fr, angle, "°", 15.0, -3600.0, 3600.0);
                }
                y += row;
                ui.text_in(Rect::new(x0, y, w0, 18.0 * s), t("Сетка при необходимости уплотняется", "The mesh is refined when needed"), 11.0, theme::TEXT_DIM(), 0);
                y += row;
            }
            Dialog::Subdivide { id, levels, smooth } => {
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Тело", "Body"), 100.0);
                State::body_combo(ui, id_of("dlg_sd_body"), fr, &solids, id);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Уровни", "Levels"), 100.0);
                ui.number_field(id_of("dlg_sd_lv"), fr, levels, "", 1.0, 1.0, 4.0);
                *levels = levels.round();
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Метод", "Method"), 100.0);
                let mut m = (!*smooth) as usize;
                if ui.segmented(id_of("dlg_sd_m"), fr, &[t("Гладкий (Loop)", "Smooth (Loop)"), t("Простой", "Simple")], &mut m) {
                    *smooth = m == 0;
                }
                y += row;
                let tris = self.scene.body(*id).map(|b| b.mesh.triangle_count()).unwrap_or(0);
                let after = tris.saturating_mul(4usize.pow(levels.round().clamp(1.0, 4.0) as u32));
                ui.text_in(Rect::new(x0, y, w0, 18.0 * s), &format!("{} → {} {}", fmt_count(tris), fmt_count(after), t("треуг.", "tris")), 11.5, theme::TEXT_DIM(), 0);
                y += row;
                ok_label = t("Подразделить", "Subdivide");
            }
            Dialog::Smooth { id, iterations, factor, taubin } => {
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Тело", "Body"), 100.0);
                State::body_combo(ui, id_of("dlg_sm_body"), fr, &solids, id);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Проходов", "Iterations"), 100.0);
                ui.number_field(id_of("dlg_sm_it"), fr, iterations, "", 1.0, 1.0, 200.0);
                *iterations = iterations.round();
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Сила", "Factor"), 100.0);
                ui.number_field(id_of("dlg_sm_f"), fr, factor, "", 0.05, 0.0, 1.0);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Метод", "Method"), 100.0);
                let mut m = *taubin as usize;
                if ui.segmented(id_of("dlg_sm_m"), fr, &[t("Лаплас", "Laplacian"), t("Таубин", "Taubin")], &mut m) {
                    *taubin = m == 1;
                }
                y += row;
                ok_label = t("Сгладить", "Smooth");
            }
            Dialog::Coil { radius, pitch, turns, wire } => {
                ui.text_in(Rect::new(x0, y, w0, 20.0 * s), t("Ось пружины — Z; встанет рядом с моделью", "Coil axis is Z; placed next to the model"), 11.5, theme::TEXT_DIM(), 0);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Радиус витка", "Coil radius"), 110.0);
                ui.number_field(id_of("dlg_cl_r"), fr, radius, t("мм", "mm"), 1.0, 0.1, 1e4);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Шаг", "Pitch"), 110.0);
                ui.number_field(id_of("dlg_cl_p"), fr, pitch, t("мм", "mm"), 0.5, -1e4, 1e4);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Витков", "Turns"), 110.0);
                ui.number_field(id_of("dlg_cl_n"), fr, turns, "", 0.5, 0.1, 200.0);
                y += row;
                let fr = ui.labeled(Rect::new(x0, y, w0, 26.0 * s), t("Радиус проволоки", "Wire radius"), 110.0);
                ui.number_field(id_of("dlg_cl_w"), fr, wire, t("мм", "mm"), 0.1, 0.01, 1e3);
                y += row;
            }
            _ => {}
        }
        // Ошибка построения (из предпросмотра).
        if d.is_feature() {
            let key = d.preview_key(self);
            let err = self.preview_error.borrow().as_ref().filter(|(k, _)| Some(*k) == key).map(|(_, e)| e.clone());
            if let Some(e) = err {
                ui.text_in(Rect::new(x0, y, w0, 20.0 * s), &model_error_text(&e), 11.5, theme::DANGER(), 0);
                ok_enabled = false;
            }
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
            Dialog::Primitive { prim, place } => {
                self.create_primitive(*prim, *place);
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
            d if d.is_feature() => self.apply_feature(d),
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
                    ("Ctrl+I, Ctrl+E", t("Импорт, экспорт STL", "Import, export STL")),
                    ("Ctrl+T / W", t("Новая / закрыть вкладку", "New / close tab")),
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
                    (t("L R C A O S", "L R C A O S"), t("Инструменты эскиза", "Sketch tools")),
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
                            None => self.request_quit(),
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
                            // Выход: проверить остальные вкладки.
                            self.saved_revision = self.scene.revision;
                            self.request_quit();
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

/// Короткая запись точки: (x; y; z).
pub fn fmt_vec(p: crate::math::Vec3) -> String {
    format!("({}; {}; {})", crate::ui::fmt_num(p.x), crate::ui::fmt_num(p.y), crate::ui::fmt_num(p.z))
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
