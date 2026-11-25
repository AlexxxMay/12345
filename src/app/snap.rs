//! Привязка к модели: вершины, рёбра, грани и сетка.
//!
//! Используется при размещении нового тела (тело следует за курсором),
//! при выборе плоскости эскиза по грани и при рисовании эскиза.

use super::view::{dist_to_seg, ray_plane};
use super::{t, Dialog, State};
use crate::math::{Aabb, Mat4, Ray, Vec3};
use crate::renderer::software::{self, Line3};
use crate::scene::Body;
use crate::sketch::SketchPlane;
use crate::ui::{theme, Ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SnapKind {
    Vertex,
    Edge,
    Face,
    Grid,
    Free,
}

/// Точка привязки под курсором.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelSnap {
    pub point: Vec3,
    /// Нормаль (к зрителю) грани под курсором; для сетки — +Z.
    pub normal: Vec3,
    pub kind: SnapKind,
    pub body: Option<u64>,
    /// Ребро, к которому выполнена привязка.
    pub edge: Option<(Vec3, Vec3)>,
}

impl SnapKind {
    pub fn label(&self) -> &'static str {
        match self {
            SnapKind::Vertex => t("вершина", "vertex"),
            SnapKind::Edge => t("ребро", "edge"),
            SnapKind::Face => t("грань", "face"),
            SnapKind::Grid => t("сетка", "grid"),
            SnapKind::Free => t("свободно", "free"),
        }
    }
}

/// Пересечение луча с телом: (t, точка, нормаль, треугольник) в мировых координатах.
pub fn ray_body(body: &Body, ray: &Ray) -> Option<(f32, Vec3, Vec3, usize)> {
    if body.mesh.is_empty() {
        return None;
    }
    let inv = body.transform.inverse()?;
    let lray = Ray::new(inv.transform_point(ray.origin), inv.transform_vector(ray.dir));
    body.mesh.aabb().ray_intersect(&lray)?;
    let mut best: Option<(f32, usize)> = None;
    for i in 0..body.mesh.triangle_count() {
        let [a, b, c] = body.mesh.tri(i);
        if let Some((tt, _, _)) = lray.intersect_triangle(a, b, c) {
            if best.map_or(true, |(bt, _)| tt < bt) {
                best = Some((tt, i));
            }
        }
    }
    let (lt, tri) = best?;
    let wp = body.transform.transform_point(lray.point_at(lt));
    let [a, b, c] = body.mesh.tri(tri);
    let mut n = body.transform.normal_matrix().transform_vector((b - a).cross(c - a)).normalize();
    if n.dot(ray.dir) > 0.0 {
        n = -n;
    }
    Some(((wp - ray.origin).dot(ray.dir), wp, n, tri))
}

/// Ближайшая к лучу точка отрезка `a–b`.
pub fn closest_on_segment_to_ray(a: Vec3, b: Vec3, ray: &Ray) -> Vec3 {
    let u = b - a;
    let w = a - ray.origin;
    let (uu, ud, dd) = (u.dot(u), u.dot(ray.dir), ray.dir.dot(ray.dir));
    let (uw, dw) = (u.dot(w), ray.dir.dot(w));
    let den = uu * dd - ud * ud;
    let s = if den.abs() < 1e-12 || uu < 1e-20 { 0.0 } else { ((ud * dw - dd * uw) / den).clamp(0.0, 1.0) };
    a + u * s
}

/// Поворот, переводящий +Z в направление `n`.
pub fn rotation_z_to(n: Vec3) -> Mat4 {
    let n = n.normalize();
    let z = Vec3::new(0.0, 0.0, 1.0);
    let axis = z.cross(n);
    let l = axis.len();
    if l < 1e-6 {
        return if n.z >= 0.0 { Mat4::identity() } else { Mat4::rotation_axis(Vec3::new(1.0, 0.0, 0.0), std::f32::consts::PI) };
    }
    Mat4::rotation_axis(axis * (1.0 / l), n.z.clamp(-1.0, 1.0).acos())
}

/// Трансформация тела с габаритами `bb`: основание (центр нижней грани)
/// ставится в точку привязки, ось Z тела — по нормали.
pub fn placement_at(bb: &Aabb, snap: &ModelSnap) -> Mat4 {
    let base = Vec3::new((bb.min.x + bb.max.x) * 0.5, (bb.min.y + bb.max.y) * 0.5, bb.min.z);
    let n = if snap.kind == SnapKind::Grid || snap.kind == SnapKind::Free { Vec3::new(0.0, 0.0, 1.0) } else { snap.normal };
    Mat4::translation(snap.point) * rotation_z_to(n) * Mat4::translation(-base)
}

impl State {
    /// Привязка под курсором. `grid` — разрешить привязку к плоскости XY
    /// (когда под курсором нет модели); `free` — без привязки (Shift).
    pub(super) fn model_snap(&mut self, mx: f32, my: f32, s: f32, grid: bool, free: bool) -> Option<ModelSnap> {
        if !self.layout.view.contains(mx, my) {
            return None;
        }
        let ray = self.screen_ray(mx, my);
        // Тело под курсором (по буферу идентификаторов, затем точный луч).
        let (id, _) = self.pick(mx, my);
        let hit = id.and_then(|id| {
            let b = self.scene.body(id)?;
            if !b.visible || b.sketch.is_some() {
                return None;
            }
            ray_body(b, &ray).map(|(_, p, n, _)| (id, p, n))
        });
        if let Some((id, p, n)) = hit {
            let mut snap = ModelSnap { point: p, normal: n, kind: SnapKind::Face, body: Some(id), edge: None };
            if free {
                return Some(snap);
            }
            let idx = self.scene.index_of(id)?;
            let tf = self.scene.bodies[idx].transform;
            if let Some(dm) = self.display_mesh(idx) {
                let tol = 10.0 * s;
                let wpp = self.world_per_px(p);
                let rad2 = (wpp * tol * 3.0).powi(2);
                // Вершины.
                let mut best = tol;
                for &q in &dm.positions {
                    let wq = tf.transform_point(q);
                    if (wq - p).len2() > rad2 {
                        continue;
                    }
                    if let Some(sp) = self.to_screen(wq) {
                        let d = ((sp.0 - mx).powi(2) + (sp.1 - my).powi(2)).sqrt();
                        if d < best {
                            best = d;
                            snap.point = wq;
                            snap.kind = SnapKind::Vertex;
                        }
                    }
                }
                if snap.kind == SnapKind::Face {
                    // Рёбра (характерные: излом или граница).
                    let mut best = tol * 0.8;
                    for e in &dm.feature_edges {
                        let (a, b) = (tf.transform_point(e[0]), tf.transform_point(e[1]));
                        let c = closest_on_segment_to_ray(a, b, &ray);
                        if (c - p).len2() > rad2 {
                            continue;
                        }
                        if let (Some(sa), Some(sb)) = (self.to_screen(a), self.to_screen(b)) {
                            let d = dist_to_seg((mx, my), sa, sb);
                            if d < best {
                                best = d;
                                snap.point = c;
                                snap.kind = SnapKind::Edge;
                                snap.edge = Some((a, b));
                            }
                        }
                    }
                }
            }
            return Some(snap);
        }
        if !grid {
            return None;
        }
        let p = ray_plane(&ray, &SketchPlane::Xy)?;
        let mut q = Vec3::new(p.x, p.y, 0.0);
        let mut kind = SnapKind::Free;
        if !free && self.cfg.snap {
            let st = software::grid_step(&self.cam) * 0.5;
            q = Vec3::new((q.x / st).round() * st, (q.y / st).round() * st, 0.0);
            kind = SnapKind::Grid;
        }
        Some(ModelSnap { point: q, normal: Vec3::new(0.0, 0.0, 1.0), kind, body: None, edge: None })
    }

    /// Плоскость эскиза по текущему виду: через центр вида, перпендикулярно
    /// направлению взгляда, оси эскиза совпадают с осями экрана.
    pub fn view_sketch_plane(&self, offset: f32) -> SketchPlane {
        let n = self.cam.back_dir().normalize();
        let p = self.cam.target + n * offset;
        let base = SketchPlane::from_point_normal(p, n);
        if !matches!(base, SketchPlane::Frame { .. }) {
            return base;
        }
        let u = self.cam.right().normalize();
        let v = n.cross(u).normalize();
        SketchPlane::Frame { origin: n * p.dot(n), u, v }
    }

    /// Трансформация тела, создаваемого диалогом «Примитив».
    pub fn primitive_placement(&self, mesh: &crate::mesh::Mesh, place: Option<ModelSnap>) -> Mat4 {
        match place.or(self.snap_hover) {
            Some(sn) => placement_at(&mesh.aabb(), &sn),
            None => self.placement_for(mesh),
        }
    }

    /// Обновление привязки под курсором для режимов размещения:
    /// новое тело (следует за курсором) и плоскость эскиза по грани.
    pub(super) fn update_snap_hover(&mut self, mx: f32, my: f32, in_view: bool, shift: bool, s: f32) {
        let mode = match &self.dialog {
            Some(Dialog::Primitive { place: None, .. }) => 1,
            Some(Dialog::SketchPlane { .. }) => 2,
            _ => 0,
        };
        let new = if mode == 0 || !in_view || self.drag.is_some() { if self.drag.is_some() { self.snap_hover } else { None } } else { self.model_snap(mx, my, s, mode == 1, shift) };
        if new != self.snap_hover {
            self.snap_hover = new;
            self.invalidate_view();
        }
    }

    /// Щелчок в виде в режимах размещения. Возвращает `true`, если щелчок обработан.
    pub(super) fn snap_click(&mut self) -> bool {
        let snap = self.snap_hover;
        match &mut self.dialog {
            Some(Dialog::Primitive { place, .. }) => {
                if place.is_some() {
                    // Повторный щелчок — снова следовать за курсором.
                    *place = None;
                } else if let Some(sn) = snap {
                    *place = Some(sn);
                    self.snap_hover = None;
                }
                self.invalidate_view();
                true
            }
            Some(Dialog::SketchPlane { offset }) => {
                let Some(sn) = snap else { return false };
                if sn.body.is_none() {
                    return false;
                }
                let n = sn.normal.normalize();
                let plane = SketchPlane::from_point_normal(sn.point + n * *offset, n);
                self.snap_hover = None;
                self.begin_sketch(plane, None);
                true
            }
            _ => false,
        }
    }

    /// 3D-линии привязки: контур плоскости эскиза на грани.
    pub(super) fn snap_lines(&self, out: &mut Vec<Line3>) {
        let Some(sn) = self.snap_hover else { return };
        if let Some((a, b)) = sn.edge {
            out.push(Line3 { a, b, color: 0xFFFF_9F1A, width: 3.0, depth_test: false });
        }
        if matches!(self.dialog, Some(Dialog::SketchPlane { .. })) && sn.body.is_some() {
            let n = sn.normal.normalize();
            let pl = SketchPlane::from_point_normal(sn.point, n);
            let (_, u, v, _) = pl.frame();
            let r = self.world_per_px(sn.point) * 60.0 * self.scale * self.cfg.ui_scale;
            let c = sn.point + n * (r * 0.01);
            let pts = [c + (u + v) * r, c + (v - u) * r, c - (u + v) * r, c + (u - v) * r];
            for i in 0..4 {
                out.push(Line3 { a: pts[i], b: pts[(i + 1) % 4], color: 0xFFF0_8A24, width: 2.0, depth_test: false });
            }
            out.push(Line3 { a: sn.point, b: sn.point + n * r, color: 0xFF2F_6FE0, width: 2.0, depth_test: false });
        }
    }

    /// Привязка точки эскиза к вершинам и рёбрам модели: точка модели
    /// проецируется на плоскость эскиза. Возвращает (точка эскиза, точка модели, тип).
    pub(super) fn sketch_model_snap(&mut self, mx: f32, my: f32, s: f32) -> Option<(crate::math::Vec2, Vec3, SnapKind)> {
        let sess = self.sketch.as_ref()?;
        let plane = sess.sketch.plane;
        let editing = sess.body_id;
        let ray = self.screen_ray(mx, my);
        let tol_v = 9.0 * s;
        let tol_e = 6.0 * s;
        let mut best_v: Option<(f32, Vec3)> = None;
        let mut best_e: Option<(f32, Vec3)> = None;
        for idx in 0..self.scene.bodies.len() {
            let b = &self.scene.bodies[idx];
            if !b.visible || b.sketch.is_some() || b.mesh.is_empty() || Some(b.id) == editing || b.mesh.positions.len() > 400_000 {
                continue;
            }
            let tf = b.transform;
            let Some(dm) = self.display_mesh(idx) else { continue };
            for &q in &dm.positions {
                let w = tf.transform_point(q);
                if let Some(sp) = self.to_screen(w) {
                    let d = ((sp.0 - mx).powi(2) + (sp.1 - my).powi(2)).sqrt();
                    if d < tol_v && best_v.map_or(true, |(bd, _)| d < bd) {
                        best_v = Some((d, w));
                    }
                }
            }
            if best_v.is_some() {
                continue;
            }
            for e in &dm.feature_edges {
                let (a, c) = (tf.transform_point(e[0]), tf.transform_point(e[1]));
                if let (Some(sa), Some(sc)) = (self.to_screen(a), self.to_screen(c)) {
                    let d = dist_to_seg((mx, my), sa, sc);
                    if d < tol_e && best_e.map_or(true, |(bd, _)| d < bd) {
                        best_e = Some((d, closest_on_segment_to_ray(a, c, &ray)));
                    }
                }
            }
        }
        let (w, k) = match (best_v, best_e) {
            (Some((_, w)), _) => (w, SnapKind::Vertex),
            (None, Some((_, w))) => (w, SnapKind::Edge),
            _ => return None,
        };
        Some((plane.to_local(w), w, k))
    }

    /// Маркер точки привязки и подпись её типа.
    pub(super) fn draw_snap_2d(&self, ui: &mut Ui) {
        if let Some((w, k)) = self.sketch.as_ref().and_then(|s| s.hover_model) {
            if let Some(sp) = self.to_screen(w) {
                ui.fb.set_clip(self.layout.view);
                draw_snap_marker(ui, sp, k, ui.s);
                ui.fb.reset_clip();
            }
        }
        let Some(sn) = self.snap_hover else { return };
        if self.dialog.is_none() {
            return;
        }
        let Some(sp) = self.to_screen(sn.point) else { return };
        let s = ui.s;
        ui.fb.set_clip(self.layout.view);
        draw_snap_marker(ui, sp, sn.kind, s);
        let txt = sn.kind.label();
        let tw = ui.text_width(txt, 11.0) + 12.0 * s;
        let r = crate::renderer::canvas::Rect::new(sp.0 + 12.0 * s, sp.1 + 10.0 * s, tw, 20.0 * s);
        ui.fb.fill_rounded(r, 4.0 * s, theme::TOOLTIP());
        ui.text_in(r, txt, 11.0, 0xFFFF_FFFF, 1);
        ui.fb.reset_clip();
    }
}

/// Маркер привязки: квадрат — вершина, ромб — ребро, круг — грань, крест — сетка.
pub fn draw_snap_marker(ui: &mut Ui, sp: (f32, f32), kind: SnapKind, s: f32) {
    let col = 0xFFFF_9F1A;
    let (x, y) = sp;
    match kind {
        SnapKind::Vertex => {
            let h = 5.5 * s;
            let r = crate::renderer::canvas::Rect::new(x - h, y - h, h * 2.0, h * 2.0);
            ui.fb.fill_rect(r, 0xFFFF_FFFF);
            ui.fb.stroke_rect(r, 2.0 * s, col);
        }
        SnapKind::Edge => {
            let h = 6.5 * s;
            let pts = [(x, y - h), (x + h, y), (x, y + h), (x - h, y)];
            ui.fb.fill_polygon(&pts, 0xFFFF_FFFF);
            ui.fb.polyline(&pts, 2.0 * s, col, true);
        }
        SnapKind::Face => {
            ui.fb.fill_circle(x, y, 5.5 * s, 0xFFFF_FFFF);
            ui.fb.stroke_circle(x, y, 5.5 * s, 2.0 * s, col);
        }
        SnapKind::Grid | SnapKind::Free => {
            let h = 6.0 * s;
            ui.fb.line(x - h, y, x + h, y, 2.0 * s, col);
            ui.fb.line(x, y - h, x, y + h, 2.0 * s, col);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::primitive_box;

    #[test]
    fn ray_hits_box_top_with_normal() {
        let mut b = Body::new("b", primitive_box(10.0, 10.0, 10.0));
        b.transform = Mat4::translation(Vec3::new(100.0, 0.0, 0.0));
        let bb = b.world_aabb();
        let top = bb.max.z;
        let ray = Ray::new(Vec3::new(bb.center().x, bb.center().y, top + 50.0), Vec3::new(0.0, 0.0, -1.0));
        let (tt, p, n, _) = ray_body(&b, &ray).expect("попадание");
        assert!((tt - 50.0).abs() < 1e-3);
        assert!((p.z - top).abs() < 1e-3);
        assert!(n.z > 0.999);
    }

    #[test]
    fn placement_on_side_face() {
        let bb = primitive_box(4.0, 4.0, 2.0).aabb();
        let sn = ModelSnap { point: Vec3::new(10.0, 0.0, 5.0), normal: Vec3::new(1.0, 0.0, 0.0), kind: SnapKind::Face, body: Some(1), edge: None };
        let m = placement_at(&bb, &sn);
        let w = bb.transformed(m);
        // Основание — на грани x = 10, тело выступает вдоль +X на высоту 2.
        assert!((w.min.x - 10.0).abs() < 1e-4, "{:?}", w);
        assert!((w.max.x - 12.0).abs() < 1e-4, "{:?}", w);
        assert!((w.center().z - 5.0).abs() < 1e-4);
    }

    #[test]
    fn segment_ray_closest() {
        let ray = Ray::new(Vec3::new(3.0, 5.0, 10.0), Vec3::new(0.0, 0.0, -1.0));
        let c = closest_on_segment_to_ray(Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0), &ray);
        assert!((c - Vec3::new(3.0, 0.0, 0.0)).len() < 1e-4);
    }
}
