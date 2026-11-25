//! Инструмент «Править грани»: выбор грани, ребра или вершины сетки и
//! перетаскивание стрелками X / Y / Z (для грани — ещё и по нормали).
//! Режим «Вытянуть» добавляет боковые стенки по контуру грани.

use super::snap::ray_body;
use super::view::{dist_to_seg, label_box, nice_step, AXES, AXIS_COLORS};
use super::{t, Drag, State, Tool};
use crate::math::{Mat4, Vec3};
use crate::meshedit::{self as me, MeshTopo};
use crate::renderer::canvas::Rect;
use crate::renderer::software::Line3;
use crate::scene::Body;
use crate::ui::{theme, Ui};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditKind {
    Face,
    Edge,
    Vertex,
}

/// Выделенный элемент сетки. Вершины хранятся представителями групп
/// (индексами вершин меша), грань — списком треугольников: оба вида
/// индексов переживают вытягивание и перестроение топологии.
#[derive(Clone, Debug, PartialEq)]
pub struct EditSel {
    pub body: u64,
    pub kind: EditKind,
    pub reps: Vec<u32>,
    pub tris: Vec<u32>,
}

/// Перетаскивание элемента.
#[derive(Clone, Debug)]
pub struct MeshDrag {
    pub axis: usize,
    pub start_mouse: (f32, f32),
    pub axis_px: (f32, f32),
    pub size: f32,
    pub value: f32,
    pub dir_world: Vec3,
    pub start_body: Body,
    pub verts: Vec<u32>,
    pub base: Vec<Vec3>,
    pub extruded: bool,
}

#[derive(Default)]
pub struct MeshEditState {
    /// Топология: (тело, ревизия сцены, топология).
    topo: Option<(u64, u64, Arc<MeshTopo>)>,
    pub hover: Option<EditSel>,
    pub sel: Option<EditSel>,
    /// Режим грани: `false` — тянуть (растягивать каркас), `true` — вытянуть со стенками.
    pub extrude: bool,
    pub axis_hover: Option<usize>,
    pub drag: Option<MeshDrag>,
    last_mouse: (f32, f32),
}

impl State {
    fn medit_topo(&mut self, body: u64) -> Option<Arc<MeshTopo>> {
        let bd = self.scene.body(body)?;
        if let Some((b, rev, tp)) = &self.medit.topo {
            // Во время перетаскивания топология не меняется (кроме вытягивания —
            // тогда меняется число вершин и топология строится заново).
            if *b == body && (*rev == self.scene.revision || self.medit.drag.is_some()) && tp.group.len() == bd.mesh.positions.len() {
                return Some(tp.clone());
            }
        }
        if bd.mesh.is_empty() {
            return None;
        }
        let tp = Arc::new(MeshTopo::build(&bd.mesh));
        self.medit.topo = Some((body, self.scene.revision, tp.clone()));
        Some(tp)
    }

    /// Группы вершин выделения (по текущей топологии).
    fn sel_groups(&mut self, sel: &EditSel) -> Vec<u32> {
        let Some(tp) = self.medit_topo(sel.body) else { return Vec::new() };
        let Some(b) = self.scene.body(sel.body) else { return Vec::new() };
        match sel.kind {
            EditKind::Face => tp.groups_of_tris(&b.mesh, &sel.tris),
            _ => sel.reps.iter().filter_map(|&v| tp.group.get(v as usize).copied()).collect(),
        }
    }

    /// Центр выделения в мировых координатах и нормаль грани.
    fn sel_center(&mut self, sel: &EditSel) -> Option<(Vec3, Vec3)> {
        let groups = self.sel_groups(sel);
        let tp = self.medit_topo(sel.body)?;
        let b = self.scene.body(sel.body)?;
        if groups.is_empty() {
            return None;
        }
        let mut c = Vec3::zero();
        for &g in &groups {
            c += tp.group_pos(&b.mesh, g);
        }
        c = c * (1.0 / groups.len() as f32);
        let mut n = Vec3::zero();
        if sel.kind == EditKind::Face {
            for &tr in &sel.tris {
                if let Some(tn) = tp.tri_normals.get(tr as usize) {
                    n += *tn;
                }
            }
        }
        let nw = if n.len2() > 0.0 { b.transform.normal_matrix().transform_vector(n).normalize() } else { Vec3::zero() };
        Some((b.transform.transform_point(c), nw))
    }

    fn medit_axis_dir(&mut self, sel: &EditSel, axis: usize) -> Option<Vec3> {
        if axis < 3 {
            Some(AXES[axis])
        } else {
            // Стрелка по нормали — только если нормаль не совпадает с осью.
            let (_, n) = self.sel_center(sel)?;
            (n.len2() > 0.5 && AXES.iter().all(|a| a.dot(n).abs() < 0.999)).then_some(n)
        }
    }

    fn medit_axes(&mut self) -> Vec<(usize, Vec3)> {
        let Some(sel) = self.medit.sel.clone() else { return Vec::new() };
        (0..4).filter_map(|i| self.medit_axis_dir(&sel, i).map(|d| (i, d))).collect()
    }

    /// Элемент сетки под курсором.
    fn medit_pick(&mut self, mx: f32, my: f32, s: f32) -> Option<EditSel> {
        let (id, _) = self.pick(mx, my);
        let id = id?;
        let b = self.scene.body(id)?;
        if !b.visible || b.mesh.is_empty() || b.sketch.is_some() {
            return None;
        }
        let ray = self.screen_ray(mx, my);
        let (_, _, _, tri) = ray_body(b, &ray)?;
        let tf = b.transform;
        let tp = self.medit_topo(id)?;
        let b = self.scene.body(id)?;
        let mesh = &b.mesh;
        let idx = mesh.tri_indices(tri);
        let pts: Vec<Option<(f32, f32)>> = idx.iter().map(|&v| self.to_screen(tf.transform_point(mesh.positions[v as usize]))).collect();
        // Вершина.
        let tol_v = 10.0 * s;
        let mut best: Option<(f32, u32)> = None;
        for k in 0..3 {
            if let Some(sp) = pts[k] {
                let d = ((sp.0 - mx).powi(2) + (sp.1 - my).powi(2)).sqrt();
                if d < tol_v && best.map_or(true, |(bd, _)| d < bd) {
                    best = Some((d, idx[k]));
                }
            }
        }
        if let Some((_, v)) = best {
            return Some(EditSel { body: id, kind: EditKind::Vertex, reps: vec![v], tris: Vec::new() });
        }
        // Ребро: только «настоящее» — граница или излом между соседними треугольниками.
        let tol_e = 7.0 * s;
        let g = tp.tri_groups(mesh, tri);
        let n0 = tp.tri_normals[tri];
        let mut best: Option<(f32, usize)> = None;
        for k in 0..3 {
            let (a, c) = (g[k], g[(k + 1) % 3]);
            let key = if a < c { (a, c) } else { (c, a) };
            let feature = match tp.edge_tris.get(&key) {
                Some(list) => list.len() != 2 || list.iter().any(|&o| tp.tri_normals[o as usize].dot(n0) < 0.9998),
                None => true,
            };
            if !feature {
                continue;
            }
            if let (Some(sa), Some(sb)) = (pts[k], pts[(k + 1) % 3]) {
                let d = dist_to_seg((mx, my), sa, sb);
                if d < tol_e && best.map_or(true, |(bd, _)| d < bd) {
                    best = Some((d, k));
                }
            }
        }
        if let Some((_, k)) = best {
            return Some(EditSel { body: id, kind: EditKind::Edge, reps: vec![idx[k], idx[(k + 1) % 3]], tris: Vec::new() });
        }
        let tris = tp.face_region(mesh, tri, 1.0);
        Some(EditSel { body: id, kind: EditKind::Face, reps: Vec::new(), tris })
    }

    fn medit_axis_hit(&mut self, mx: f32, my: f32, s: f32) -> Option<usize> {
        let sel = self.medit.sel.clone()?;
        let (c, _) = self.sel_center(&sel)?;
        let size = self.gizmo_size(c, s);
        let cp = self.to_screen(c)?;
        let mut best = None;
        let mut bd = 8.0 * s;
        for (i, d) in self.medit_axes() {
            if let Some(ep) = self.to_screen(c + d * size) {
                let dd = dist_to_seg((mx, my), cp, ep);
                if dd < bd {
                    bd = dd;
                    best = Some(i);
                }
            }
        }
        best
    }

    /// Наведение (каждый кадр, пока инструмент активен).
    pub(super) fn medit_hover(&mut self, mx: f32, my: f32, in_view: bool, s: f32) {
        if self.medit.drag.is_some() {
            return;
        }
        // Выделение тела, которого больше нет.
        if let Some(sel) = &self.medit.sel {
            if self.scene.body(sel.body).map(|b| !b.visible).unwrap_or(true) {
                self.medit.sel = None;
            }
        }
        let (axis, hover) = if in_view {
            let axis = self.medit_axis_hit(mx, my, s);
            let hover = if axis.is_some() {
                None
            } else if self.medit.last_mouse == (mx, my) && self.medit.hover.is_some() {
                self.medit.hover.clone()
            } else {
                self.medit_pick(mx, my, s)
            };
            (axis, hover)
        } else {
            (None, None)
        };
        self.medit.last_mouse = (mx, my);
        if axis != self.medit.axis_hover || hover != self.medit.hover {
            self.medit.axis_hover = axis;
            self.medit.hover = hover;
            self.invalidate_view();
        }
    }

    /// Щелчок без перетаскивания: выбор элемента под курсором.
    pub(super) fn medit_click(&mut self) {
        let h = self.medit.hover.clone();
        if let Some(h) = &h {
            self.scene.selected = Some(h.body);
        }
        self.medit.sel = h;
        self.invalidate_view();
    }

    /// Нажатие ЛКМ на стрелку — начало перетаскивания.
    pub(super) fn medit_start(&mut self, mx: f32, my: f32, s: f32) -> bool {
        let Some(axis) = self.medit.axis_hover else { return false };
        let Some(sel) = self.medit.sel.clone() else { return false };
        let Some(dir) = self.medit_axis_dir(&sel, axis) else { return false };
        let Some((c, _)) = self.sel_center(&sel) else { return false };
        let Some(tp) = self.medit_topo(sel.body) else { return false };
        let Some(start_body) = self.scene.body(sel.body).cloned() else { return false };
        let size = self.gizmo_size(c, s);
        let cp = self.to_screen(c).unwrap_or((mx, my));
        let ep = self.to_screen(c + dir * size).unwrap_or(cp);
        let axis_px = ((ep.0 - cp.0) / size, (ep.1 - cp.1) / size);
        let mut mesh = start_body.mesh.clone();
        let mut extruded = false;
        let verts = if sel.kind == EditKind::Face && self.medit.extrude {
            extruded = true;
            me::extrude_region(&mut mesh, &tp, &sel.tris)
        } else {
            let groups = self.sel_groups(&sel);
            tp.verts_of_groups(groups)
        };
        let base: Vec<Vec3> = verts.iter().map(|&v| mesh.positions[v as usize]).collect();
        if let Some(b) = self.scene.body_mut(sel.body) {
            b.mesh = mesh;
        }
        self.medit.drag = Some(MeshDrag { axis, start_mouse: (mx, my), axis_px, size, value: 0.0, dir_world: dir, start_body, verts, base, extruded });
        self.drag = Some(Drag::MeshEdit);
        true
    }

    pub(super) fn medit_update(&mut self, mx: f32, my: f32, shift: bool) {
        let Some(mut g) = self.medit.drag.take() else { return };
        let l2 = g.axis_px.0 * g.axis_px.0 + g.axis_px.1 * g.axis_px.1;
        if l2 > 1e-9 {
            let (dx, dy) = (mx - g.start_mouse.0, my - g.start_mouse.1);
            let mut d = (dx * g.axis_px.0 + dy * g.axis_px.1) / l2;
            if self.cfg.snap && !shift {
                let st = nice_step(g.size / 20.0);
                d = (d / st).round() * st;
            }
            if d != g.value {
                g.value = d;
                let id = g.start_body.id;
                let inv = g.start_body.transform.inverse().unwrap_or(Mat4::identity());
                let local = inv.transform_vector(g.dir_world * d);
                if let Some(b) = self.scene.body_mut(id) {
                    for (k, &v) in g.verts.iter().enumerate() {
                        if let Some(p) = b.mesh.positions.get_mut(v as usize) {
                            *p = g.base[k] + local;
                        }
                    }
                }
                // Отпечаток меша выборочный — сбрасываем кэш отображения явно.
                self.cache.remove(&id);
                self.scene.revision += 1;
            }
        }
        self.medit.drag = Some(g);
    }

    /// Отпускание кнопки: изменение записывается одним шагом истории.
    pub(super) fn medit_finish(&mut self) {
        let Some(g) = self.medit.drag.take() else { return };
        let id = g.start_body.id;
        let Some(cur) = self.scene.body(id).cloned() else { return };
        // Вернуть исходное тело, затем заменить с записью в историю.
        if let Some(pos) = self.scene.index_of(id) {
            self.scene.bodies[pos] = g.start_body.clone();
        }
        self.cache.remove(&id);
        if g.value.abs() < 1e-9 {
            self.scene.revision += 1;
            self.invalidate_view();
            return;
        }
        let mut after = cur;
        after.primitive = None;
        if after.mesh.normals.len() == after.mesh.positions.len() {
            after.mesh.compute_smooth_normals();
        } else if !after.mesh.normals.is_empty() {
            after.mesh.normals.clear();
        }
        if g.extruded {
            after.material.shading = 0;
        }
        self.scene.replace_body(after);
        self.status = format!("{} {}", t("Сдвиг", "Offset"), crate::ui::fmt_num(g.value));
        self.invalidate_view();
    }

    /// Esc: снять выделение элемента. Возвращает `true`, если было что снимать.
    pub(super) fn medit_escape(&mut self) -> bool {
        if self.medit.drag.is_some() {
            if let Some(g) = self.medit.drag.take() {
                if let Some(pos) = self.scene.index_of(g.start_body.id) {
                    self.scene.bodies[pos] = g.start_body.clone();
                }
                self.cache.remove(&g.start_body.id);
                self.scene.revision += 1;
            }
            self.drag = None;
            return true;
        }
        if self.medit.sel.is_some() {
            self.medit.sel = None;
            self.invalidate_view();
            return true;
        }
        false
    }

    /// Ключ для перерисовки 3D-вида.
    pub(super) fn medit_key(&self) -> String {
        if self.tool != Tool::EditMesh {
            return String::new();
        }
        format!("{:?}{:?}{:?}{}", self.medit.hover.as_ref().map(|h| (h.kind, h.reps.clone(), h.tris.first().copied(), h.tris.len())), self.medit.sel, self.medit.axis_hover, self.medit.extrude)
    }

    /// Линии выделения и стрелок.
    pub(super) fn medit_lines(&mut self, s: f32, out: &mut Vec<Line3>) {
        if self.tool != Tool::EditMesh {
            return;
        }
        let hov = self.medit.hover.clone();
        let sel = self.medit.sel.clone();
        if let Some(h) = &hov {
            if Some(h) != sel.as_ref() {
                self.sel_outline(h, 0xFF7C_C4FF, 2.0, out);
            }
        }
        if let Some(sl) = &sel {
            self.sel_outline(sl, 0xFFFF_9F1A, 3.0, out);
            if let Some((c, _)) = self.sel_center(sl) {
                let size = self.gizmo_size(c, s);
                let active = self.medit.drag.as_ref().map(|g| g.axis).or(self.medit.axis_hover);
                for (i, d) in self.medit_axes() {
                    let col = if active == Some(i) { 0xFFFF_C21A } else if i < 3 { AXIS_COLORS[i] } else { 0xFFF5_F6F8 };
                    out.push(Line3 { a: c, b: c + d * size, color: col, width: 2.6, depth_test: false });
                }
            }
        }
    }

    fn sel_outline(&mut self, sel: &EditSel, color: u32, width: f32, out: &mut Vec<Line3>) {
        let Some(tp) = self.medit_topo(sel.body) else { return };
        let Some(b) = self.scene.body(sel.body) else { return };
        let tf = b.transform;
        let wp = |g: u32| tf.transform_point(tp.group_pos(&b.mesh, g));
        match sel.kind {
            EditKind::Face => {
                for (a, c) in tp.region_boundary(&b.mesh, &sel.tris) {
                    out.push(Line3 { a: wp(a), b: wp(c), color, width, depth_test: false });
                }
            }
            EditKind::Edge => {
                let gs: Vec<u32> = sel.reps.iter().filter_map(|&v| tp.group.get(v as usize).copied()).collect();
                if gs.len() == 2 {
                    out.push(Line3 { a: wp(gs[0]), b: wp(gs[1]), color, width: width + 1.0, depth_test: false });
                }
            }
            EditKind::Vertex => {}
        }
    }

    /// 2D: вершины, наконечники стрелок, значение сдвига, подсказка режима.
    pub(super) fn draw_medit_2d(&mut self, ui: &mut Ui) {
        if self.tool != Tool::EditMesh {
            return;
        }
        let s = ui.s;
        let v = self.layout.view;
        ui.fb.set_clip(v);
        for (sel, col) in [(self.medit.hover.clone(), 0xFF7C_C4FFu32), (self.medit.sel.clone(), 0xFFFF_9F1A)] {
            let Some(sel) = sel else { continue };
            if sel.kind == EditKind::Vertex {
                if let Some((c, _)) = self.sel_center(&sel) {
                    if let Some(sp) = self.to_screen(c) {
                        let h = 5.0 * s;
                        let r = Rect::new(sp.0 - h, sp.1 - h, h * 2.0, h * 2.0);
                        ui.fb.fill_rect(r, 0xFFFF_FFFF);
                        ui.fb.stroke_rect(r, 2.0 * s, col);
                    }
                }
            }
        }
        if let Some(sel) = self.medit.sel.clone() {
            if let Some((c, _)) = self.sel_center(&sel) {
                let size = self.gizmo_size(c, s);
                let active = self.medit.drag.as_ref().map(|g| g.axis).or(self.medit.axis_hover);
                if let Some(cp) = self.to_screen(c) {
                    for (i, d) in self.medit_axes() {
                        let col = if active == Some(i) { 0xFFFF_C21A } else if i < 3 { AXIS_COLORS[i] } else { 0xFFF5_F6F8 };
                        if let Some(ep) = self.to_screen(c + d * size) {
                            let (dx, dy) = (ep.0 - cp.0, ep.1 - cp.1);
                            let l = (dx * dx + dy * dy).sqrt().max(1e-3);
                            let (ux, uy) = (dx / l, dy / l);
                            let hs = 7.0 * s;
                            let tip = (ep.0 + ux * hs * 1.6, ep.1 + uy * hs * 1.6);
                            let tri = [tip, (ep.0 - uy * hs * 0.7, ep.1 + ux * hs * 0.7), (ep.0 + uy * hs * 0.7, ep.1 - ux * hs * 0.7)];
                            ui.fb.fill_polygon(&tri, col);
                            if i == 3 {
                                ui.fb.polyline(&tri, 1.2 * s, 0xFF30_3540, true);
                            }
                        }
                    }
                    ui.fb.fill_circle(cp.0, cp.1, 4.0 * s, 0xFFFF_FFFF);
                    ui.fb.stroke_circle(cp.0, cp.1, 4.0 * s, 1.5 * s, 0xFF30_3540);
                }
            }
        }
        if let Some(g) = &self.medit.drag {
            let txt = format!("{} {}", crate::ui::fmt_num(g.value), t("мм", "mm"));
            let (mx, my) = ui.input.mouse;
            label_box(ui, mx + 16.0 * s, my + 14.0 * s, &txt, theme::TOOLTIP(), 0xFFFF_FFFF);
        }
        ui.fb.reset_clip();

        // Панель режима над видом.
        let pw = 470.0 * s;
        let r = Rect::new(v.x + (v.w - pw) * 0.5, v.y + 10.0 * s, pw, 34.0 * s);
        ui.panel(r, 17.0);
        let seg = Rect::new(r.x + 8.0 * s, r.y + 5.0 * s, 190.0 * s, 24.0 * s);
        let mut m = self.medit.extrude as usize;
        if ui.segmented(crate::ui::id_of("medit_mode"), seg, &[t("Тянуть", "Move"), t("Вытянуть", "Extrude")], &mut m) {
            self.medit.extrude = m == 1;
            self.invalidate_view();
        }
        let hint = match &self.medit.sel {
            None => t("Щёлкните грань, ребро или вершину", "Click a face, edge or vertex"),
            Some(_) => t("Тяните стрелку · Shift — без шага · Esc", "Drag an arrow · Shift — no step · Esc"),
        };
        ui.text_in(Rect::new(seg.right() + 10.0 * s, r.y, r.right() - seg.right() - 18.0 * s, r.h), hint, 11.5, theme::TEXT_DIM(), 0);
    }
}
