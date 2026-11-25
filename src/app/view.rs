//! 3D-вид: навигация, выбор, манипуляторы, эскизы, измерения, ViewCube.

use super::{t, Drag, NavMode, SketchSession, State, Tool};
use crate::math::{Aabb, Color, Mat4, Ray, Vec2, Vec3};
use crate::renderer::canvas::{argb, fade, Rect};
use crate::renderer::software::{self, DisplayMesh, DisplayMode, Highlight, Line3, RenderItem, RenderSettings};
use crate::sketch::{SketchEntity, SketchPlane};
use crate::ui::{theme, Cursor, Key, Ui};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Стандартные виды.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewDir {
    Front,
    Back,
    Right,
    Left,
    Top,
    Bottom,
    Iso,
}

impl ViewDir {
    pub fn vector(&self) -> Vec3 {
        match self {
            ViewDir::Front => Vec3::new(0.0, -1.0, 0.0),
            ViewDir::Back => Vec3::new(0.0, 1.0, 0.0),
            ViewDir::Right => Vec3::new(1.0, 0.0, 0.0),
            ViewDir::Left => Vec3::new(-1.0, 0.0, 0.0),
            ViewDir::Top => Vec3::new(0.0, 0.0, 1.0),
            ViewDir::Bottom => Vec3::new(0.0, 0.0, -1.0),
            ViewDir::Iso => Vec3::new(1.0, -1.0, 0.8).normalize(),
        }
    }
}

/// Перетаскивание манипулятора.
#[derive(Clone, Copy, Debug)]
pub struct GizmoDrag {
    pub body: u64,
    /// 0..2 — оси X/Y/Z, 3 — равномерно (масштаб).
    pub axis: usize,
    pub start_mouse: (f32, f32),
    pub start_tf: Mat4,
    pub center: Vec3,
    pub center_px: (f32, f32),
    /// Экранный вектор оси на единицу длины.
    pub axis_px: (f32, f32),
    pub size: f32,
    pub value: f32,
}

const AXIS_COLORS: [u32; 3] = [0xFFE0_3E3E, 0xFF35_A853, 0xFF2F_6FE0];
const AXES: [Vec3; 3] = [Vec3 { x: 1.0, y: 0.0, z: 0.0 }, Vec3 { x: 0.0, y: 1.0, z: 0.0 }, Vec3 { x: 0.0, y: 0.0, z: 1.0 }];

fn nice_step(raw: f32) -> f32 {
    let p = 10f32.powf(raw.max(1e-6).log10().floor());
    let m = raw / p;
    if m >= 5.0 {
        5.0 * p
    } else if m >= 2.0 {
        2.0 * p
    } else {
        p
    }
}

fn hash_f(h: &mut DefaultHasher, v: f32) {
    v.to_bits().hash(h);
}

fn dist_to_seg(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = (dx * dx + dy * dy).max(1e-6);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0);
    let (qx, qy) = (a.0 + dx * t - p.0, a.1 + dy * t - p.1);
    (qx * qx + qy * qy).sqrt()
}

fn ray_plane(ray: &Ray, plane: &SketchPlane) -> Option<Vec2> {
    let (o, _, _, n) = plane.frame();
    let denom = ray.dir.dot(n);
    if denom.abs() < 1e-8 {
        return None;
    }
    let t = (o - ray.origin).dot(n) / denom;
    if t < 0.0 {
        return None;
    }
    Some(plane.to_local(ray.point_at(t)))
}

impl State {
    // -----------------------------------------------------------------------
    // Проекции и выбор
    // -----------------------------------------------------------------------

    /// Мировая точка → экран (координаты окна).
    pub fn to_screen(&self, p: Vec3) -> Option<(f32, f32)> {
        let v = self.layout.view;
        self.cam.project(p, v.w, v.h).map(|(x, y)| (x + v.x, y + v.y))
    }

    /// Луч из точки окна.
    pub fn screen_ray(&self, x: f32, y: f32) -> Ray {
        let v = self.layout.view;
        self.cam.ray(x - v.x, y - v.y, v.w, v.h)
    }

    /// Мировой размер пикселя на глубине точки.
    pub fn world_per_px(&self, p: Vec3) -> f32 {
        let vh = self.layout.view.h.max(1.0);
        if self.cam.ortho {
            self.cam.pixel_size(vh)
        } else {
            let depth = (p - self.cam.eye()).dot(-self.cam.back_dir()).max(self.cam.distance * 0.01);
            2.0 * depth * (self.cam.fov_y * 0.5).tan() / vh
        }
    }

    /// Выбор по буферу идентификаторов: (id тела, точка на поверхности).
    pub fn pick(&self, x: f32, y: f32) -> (Option<u64>, Option<Vec3>) {
        let v = self.layout.view;
        if !v.contains(x, y) || self.view_fb.width < 2 {
            return (None, None);
        }
        let fx = (x - v.x) * self.view_fb.width as f32 / v.w;
        let fy = (y - v.y) * self.view_fb.height as f32 / v.h;
        let ix = (fx as usize).min(self.view_fb.width - 1);
        let iy = (fy as usize).min(self.view_fb.height - 1);
        let i = iy * self.view_fb.width + ix;
        let id = self.view_fb.ids[i];
        if id == 0 {
            return (None, None);
        }
        let q = self.view_fb.depth[i];
        let p = self.cam.unproject(fx, fy, q, self.view_fb.width as f32, self.view_fb.height as f32);
        (Some(id as u64), p)
    }

    fn mark_navigating(&mut self) {
        self.navigating_until = Some(Instant::now() + Duration::from_millis(220));
        self.wake_ms = Some(230);
    }

    fn is_navigating(&self) -> bool {
        self.navigating_until.map(|t| Instant::now() < t).unwrap_or(false) || matches!(self.drag, Some(Drag::Orbit) | Some(Drag::Pan) | Some(Drag::Zoom { .. }))
    }

    // -----------------------------------------------------------------------
    // Ввод в 3D-виде
    // -----------------------------------------------------------------------

    pub(super) fn viewport_input(&mut self, ui: &mut Ui) {
        let v = self.layout.view;
        let input = ui.input;
        let (mx, my) = input.mouse;
        let modal = self.dialog.as_ref().map(|d| !d.is_side_panel()).unwrap_or(false);
        let in_view = v.contains(mx, my) && ui.mouse_in_viewport() && !modal && self.menu.is_none() && !ui.popup_open();

        // Колесо — масштаб к курсору.
        if in_view && input.wheel != 0.0 {
            let factor = 0.87f32.powf(input.wheel.clamp(-5.0, 5.0));
            let toward = self.pick(mx, my).1.or_else(|| {
                let ray = self.screen_ray(mx, my);
                ray_plane(&ray, &SketchPlane::Xy).map(|p| Vec3::new(p.x, p.y, 0.0))
            });
            self.cam_anim = None;
            self.cam.zoom_at(factor, toward);
            self.mark_navigating();
        }

        // Наведение.
        if self.drag.is_none() {
            let (id, _) = if in_view { self.pick(mx, my) } else { (None, None) };
            let id = id.filter(|i| self.scene.body(*i).map(|b| !b.mesh.is_empty()).unwrap_or(false));
            self.hover_body = if self.tool.is_sketch() || self.tool == Tool::Measure { None } else { id };
            self.gizmo_hover = if in_view { self.gizmo_hit(mx, my, ui.s) } else { None };
            if in_view {
                if self.tool.is_sketch() || self.tool == Tool::Measure {
                    ui.st.cursor = Cursor::Crosshair;
                } else if self.gizmo_hover.is_some() {
                    ui.st.cursor = Cursor::Move;
                } else if self.hover_body.is_some() {
                    ui.st.cursor = Cursor::Hand;
                }
            }
        }

        // Эскиз: отслеживание точки под курсором.
        if self.sketch.is_some() && self.tool.is_sketch() {
            if in_view {
                self.update_sketch_hover(mx, my, input.mods.shift, ui.s);
            } else if let Some(s) = &mut self.sketch {
                s.hover = None;
            }
        }

        // Нажатия.
        if in_view && self.drag.is_none() {
            if input.pressed[0] {
                if self.tool.is_sketch() && self.sketch.is_some() {
                    self.sketch_click();
                } else if self.tool == Tool::Measure {
                    self.measure_click(mx, my, ui.s);
                } else if let Some(axis) = self.gizmo_hover {
                    self.start_gizmo(axis, mx, my, ui.s);
                } else {
                    self.drag = Some(Drag::Pending { button: 0, start: (mx, my) });
                }
            } else if input.pressed[1] {
                if self.tool.is_sketch() {
                    // Правая кнопка — завершить цепочку.
                    let busy = self.sketch.as_ref().map(|s| s.busy()).unwrap_or(false);
                    if busy {
                        self.sketch_commit();
                    } else {
                        self.drag = Some(Drag::Pending { button: 1, start: (mx, my) });
                    }
                } else {
                    self.drag = Some(Drag::Pending { button: 1, start: (mx, my) });
                }
            } else if input.pressed[2] {
                self.drag = Some(Drag::Pending { button: 2, start: (mx, my) });
            }
        }

        // Перетаскивание.
        let (dx, dy) = (mx - input.mouse_prev.0, my - input.mouse_prev.1);
        match self.drag {
            Some(Drag::Pending { button, start }) => {
                let moved = ((mx - start.0).powi(2) + (my - start.1).powi(2)).sqrt() > 4.0 * ui.s.max(1.0);
                if input.released[button] {
                    self.drag = None;
                    if button == 0 {
                        self.click_select();
                    } else if button == 1 {
                        self.open_context_menu(mx, my);
                    }
                } else if moved {
                    self.cam_anim = None;
                    let shift = input.mods.shift;
                    self.drag = Some(match (button, self.nav_mode) {
                        (0, NavMode::Pan) => Drag::Pan,
                        (0, NavMode::Zoom) => Drag::Zoom { anchor: start },
                        (0, _) => {
                            if shift {
                                Drag::Pan
                            } else {
                                Drag::Orbit
                            }
                        }
                        (2, _) => {
                            if shift {
                                Drag::Orbit
                            } else {
                                Drag::Pan
                            }
                        }
                        _ => {
                            if shift {
                                Drag::Orbit
                            } else {
                                Drag::Pan
                            }
                        }
                    });
                }
            }
            Some(Drag::Orbit) => {
                if dx != 0.0 || dy != 0.0 {
                    self.cam.orbit(-dx * 0.0085, dy * 0.0085);
                    self.mark_navigating();
                }
                ui.st.cursor = Cursor::Move;
            }
            Some(Drag::Pan) => {
                if dx != 0.0 || dy != 0.0 {
                    let wpp = self.world_per_px(self.cam.target);
                    self.cam.target = self.cam.target - self.cam.right() * (dx * wpp) + self.cam.up() * (dy * wpp);
                    self.mark_navigating();
                }
                ui.st.cursor = Cursor::Move;
            }
            Some(Drag::Zoom { .. }) => {
                if dy != 0.0 {
                    self.cam.zoom_at(1.012f32.powf(dy), None);
                    self.mark_navigating();
                }
            }
            Some(Drag::Gizmo(g)) => {
                self.update_gizmo(g, mx, my, input.mods.shift);
                ui.st.cursor = Cursor::Move;
            }
            None => {}
        }
        if self.drag.is_some() && !input.any_down() {
            if let Some(Drag::Gizmo(g)) = self.drag {
                self.scene.commit_transform(g.body, g.start_tf);
            }
            self.drag = None;
            self.invalidate_view();
        }
    }

    fn click_select(&mut self) {
        match self.hover_body {
            Some(id) => {
                self.scene.selected = Some(id);
            }
            None => {
                // Клик по эскизу: проверка близости к линиям эскиза.
                let (mx, my) = (self.last_mouse().0, self.last_mouse().1);
                let hit = self.pick_sketch_body(mx, my);
                self.scene.selected = hit;
            }
        }
        self.invalidate_view();
    }

    fn last_mouse(&self) -> (f32, f32) {
        self.mouse_cache
    }

    /// Выбор тела-эскиза по близости к его линиям.
    pub fn pick_sketch_body(&self, mx: f32, my: f32) -> Option<u64> {
        let tol = self.px(7.0);
        for b in self.scene.bodies.iter().rev() {
            if !b.visible {
                continue;
            }
            let Some(sk) = &b.sketch else { continue };
            for (a, c) in sketch_segments(sk) {
                let pa = self.to_screen(b.transform.transform_point(a));
                let pc = self.to_screen(b.transform.transform_point(c));
                if let (Some(pa), Some(pc)) = (pa, pc) {
                    if dist_to_seg((mx, my), pa, pc) < tol {
                        return Some(b.id);
                    }
                }
            }
        }
        None
    }

    // -----------------------------------------------------------------------
    // Манипуляторы
    // -----------------------------------------------------------------------

    fn gizmo_body(&self) -> Option<(u64, Vec3)> {
        if !matches!(self.tool, Tool::Move | Tool::Rotate | Tool::Scale) || self.sketch.is_some() {
            return None;
        }
        let id = self.scene.selected?;
        let b = self.scene.body(id)?;
        if !b.visible || b.mesh.is_empty() {
            return None;
        }
        Some((id, b.world_aabb().center()))
    }

    fn gizmo_size(&self, center: Vec3, s: f32) -> f32 {
        self.world_per_px(center) * 90.0 * s
    }

    fn ring_points(center: Vec3, axis: usize, r: f32) -> Vec<Vec3> {
        let (u, w) = match axis {
            0 => (AXES[1], AXES[2]),
            1 => (AXES[2], AXES[0]),
            _ => (AXES[0], AXES[1]),
        };
        (0..=64)
            .map(|i| {
                let a = i as f32 / 64.0 * std::f32::consts::TAU;
                center + u * (r * a.cos()) + w * (r * a.sin())
            })
            .collect()
    }

    fn gizmo_hit(&self, mx: f32, my: f32, s: f32) -> Option<usize> {
        let (_, c) = self.gizmo_body()?;
        let size = self.gizmo_size(c, s);
        let cp = self.to_screen(c)?;
        let tol = 8.0 * s;
        match self.tool {
            Tool::Move | Tool::Scale => {
                if self.tool == Tool::Scale && ((mx - cp.0).powi(2) + (my - cp.1).powi(2)).sqrt() < tol * 1.2 {
                    return Some(3);
                }
                let mut best = None;
                let mut bd = tol;
                for (i, a) in AXES.iter().enumerate() {
                    if let Some(ep) = self.to_screen(c + *a * size) {
                        let d = dist_to_seg((mx, my), cp, ep);
                        if d < bd {
                            bd = d;
                            best = Some(i);
                        }
                    }
                }
                best
            }
            Tool::Rotate => {
                let mut best = None;
                let mut bd = tol;
                for i in 0..3 {
                    let pts = Self::ring_points(c, i, size * 0.85);
                    let sp: Vec<Option<(f32, f32)>> = pts.iter().map(|p| self.to_screen(*p)).collect();
                    for w in sp.windows(2) {
                        if let (Some(a), Some(b)) = (w[0], w[1]) {
                            let d = dist_to_seg((mx, my), a, b);
                            if d < bd {
                                bd = d;
                                best = Some(i);
                            }
                        }
                    }
                }
                best
            }
            _ => None,
        }
    }

    fn start_gizmo(&mut self, axis: usize, mx: f32, my: f32, s: f32) {
        let Some((id, c)) = self.gizmo_body() else { return };
        let Some(b) = self.scene.body(id) else { return };
        let size = self.gizmo_size(c, s);
        let cp = self.to_screen(c).unwrap_or((mx, my));
        let axis_px = if axis < 3 {
            let ep = self.to_screen(c + AXES[axis] * size).unwrap_or(cp);
            ((ep.0 - cp.0) / size, (ep.1 - cp.1) / size)
        } else {
            (0.0, 0.0)
        };
        self.drag = Some(Drag::Gizmo(GizmoDrag {
            body: id,
            axis,
            start_mouse: (mx, my),
            start_tf: b.transform,
            center: c,
            center_px: cp,
            axis_px,
            size,
            value: 0.0,
        }));
    }

    fn update_gizmo(&mut self, mut g: GizmoDrag, mx: f32, my: f32, shift: bool) {
        let (dx, dy) = (mx - g.start_mouse.0, my - g.start_mouse.1);
        let snap = self.cfg.snap && !shift;
        let new_tf = match self.tool {
            Tool::Move => {
                let l2 = g.axis_px.0 * g.axis_px.0 + g.axis_px.1 * g.axis_px.1;
                if l2 < 1e-9 {
                    return;
                }
                let mut d = (dx * g.axis_px.0 + dy * g.axis_px.1) / l2;
                if snap {
                    let st = nice_step(g.size / 20.0);
                    d = (d / st).round() * st;
                }
                g.value = d;
                Mat4::translation(AXES[g.axis] * d) * g.start_tf
            }
            Tool::Rotate => {
                let a0 = (g.start_mouse.1 - g.center_px.1).atan2(g.start_mouse.0 - g.center_px.0);
                let a1 = (my - g.center_px.1).atan2(mx - g.center_px.0);
                let mut da = a1 - a0;
                while da > std::f32::consts::PI {
                    da -= std::f32::consts::TAU;
                }
                while da < -std::f32::consts::PI {
                    da += std::f32::consts::TAU;
                }
                // Ось на зрителя: движение по часовой стрелке на экране — отрицательный угол.
                if AXES[g.axis].dot(self.cam.back_dir()) > 0.0 {
                    da = -da;
                }
                let mut deg = da.to_degrees();
                if snap {
                    deg = (deg / 5.0).round() * 5.0;
                }
                g.value = deg;
                Mat4::translation(g.center) * Mat4::rotation_axis(AXES[g.axis], deg.to_radians()) * Mat4::translation(-g.center) * g.start_tf
            }
            Tool::Scale => {
                let k = if g.axis < 3 {
                    let l2 = g.axis_px.0 * g.axis_px.0 + g.axis_px.1 * g.axis_px.1;
                    if l2 < 1e-9 {
                        return;
                    }
                    1.0 + (dx * g.axis_px.0 + dy * g.axis_px.1) / l2 / g.size
                } else {
                    1.0 + (dx - dy) / (g.size / self.world_per_px(g.center)) * 0.5
                };
                let mut k = k.max(0.01);
                if snap {
                    k = ((k * 20.0).round() / 20.0).max(0.05);
                }
                g.value = k;
                let sv = if g.axis < 3 {
                    let mut v = Vec3::one();
                    v.set(g.axis, k);
                    v
                } else {
                    Vec3::splat(k)
                };
                Mat4::translation(g.center) * Mat4::scale(sv) * Mat4::translation(-g.center) * g.start_tf
            }
            _ => return,
        };
        if let Some(b) = self.scene.body_mut(g.body) {
            b.transform = new_tf;
        }
        self.scene.revision += 1;
        self.drag = Some(Drag::Gizmo(g));
    }

    fn gizmo_lines(&self, s: f32, out: &mut Vec<Line3>) {
        let Some((_, c)) = self.gizmo_body() else { return };
        let size = self.gizmo_size(c, s);
        let active = match self.drag {
            Some(Drag::Gizmo(g)) => Some(g.axis),
            _ => self.gizmo_hover,
        };
        let col = |i: usize| if active == Some(i) { 0xFFFF_C21A } else { AXIS_COLORS[i] };
        match self.tool {
            Tool::Move | Tool::Scale => {
                for i in 0..3 {
                    out.push(Line3 { a: c, b: c + AXES[i] * size, color: col(i), width: 2.6, depth_test: false });
                }
            }
            Tool::Rotate => {
                for i in 0..3 {
                    let pts = Self::ring_points(c, i, size * 0.85);
                    for w in pts.windows(2) {
                        out.push(Line3 { a: w[0], b: w[1], color: col(i), width: 2.4, depth_test: false });
                    }
                }
            }
            _ => {}
        }
    }

    fn draw_gizmo_2d(&self, ui: &mut Ui) {
        let Some((_, c)) = self.gizmo_body() else { return };
        let s = ui.s;
        let size = self.gizmo_size(c, s);
        let Some(cp) = self.to_screen(c) else { return };
        let active = match self.drag {
            Some(Drag::Gizmo(g)) => Some(g.axis),
            _ => self.gizmo_hover,
        };
        let col = |i: usize| if active == Some(i) { 0xFFFF_C21A } else { AXIS_COLORS[i] };
        ui.fb.set_clip(self.layout.view);
        match self.tool {
            Tool::Move => {
                for i in 0..3 {
                    if let Some(ep) = self.to_screen(c + AXES[i] * size) {
                        let (dx, dy) = (ep.0 - cp.0, ep.1 - cp.1);
                        let l = (dx * dx + dy * dy).sqrt().max(1e-3);
                        let (ux, uy) = (dx / l, dy / l);
                        let hs = 7.0 * s;
                        let tip = (ep.0 + ux * hs * 1.6, ep.1 + uy * hs * 1.6);
                        ui.fb.fill_polygon(&[tip, (ep.0 - uy * hs * 0.7, ep.1 + ux * hs * 0.7), (ep.0 + uy * hs * 0.7, ep.1 - ux * hs * 0.7)], col(i));
                    }
                }
                ui.fb.fill_circle(cp.0, cp.1, 4.0 * s, 0xFFFF_FFFF);
                ui.fb.stroke_circle(cp.0, cp.1, 4.0 * s, 1.5 * s, 0xFF30_3540);
            }
            Tool::Scale => {
                for i in 0..3 {
                    if let Some(ep) = self.to_screen(c + AXES[i] * size) {
                        let hs = 5.5 * s;
                        ui.fb.fill_rect(Rect::new(ep.0 - hs, ep.1 - hs, hs * 2.0, hs * 2.0), col(i));
                    }
                }
                let hs = 6.0 * s;
                ui.fb.fill_rounded(Rect::new(cp.0 - hs, cp.1 - hs, hs * 2.0, hs * 2.0), 2.0 * s, if active == Some(3) { 0xFFFF_C21A } else { 0xFFFF_FFFF });
                ui.fb.stroke_rounded(Rect::new(cp.0 - hs, cp.1 - hs, hs * 2.0, hs * 2.0), 2.0 * s, 1.5 * s, 0xFF30_3540);
            }
            Tool::Rotate => {
                ui.fb.fill_circle(cp.0, cp.1, 3.5 * s, 0xFF30_3540);
            }
            _ => {}
        }
        // Значение при перетаскивании.
        if let Some(Drag::Gizmo(g)) = self.drag {
            let txt = match self.tool {
                Tool::Move => format!("{} {}", crate::ui::fmt_num(g.value), t("мм", "mm")),
                Tool::Rotate => format!("{}°", crate::ui::fmt_num(g.value)),
                _ => format!("× {}", crate::ui::fmt_num(g.value)),
            };
            let (mx, my) = ui.input.mouse;
            label_box(ui, mx + 16.0 * s, my + 14.0 * s, &txt, theme::TOOLTIP(), 0xFFFF_FFFF);
        }
        ui.fb.reset_clip();
    }

    // -----------------------------------------------------------------------
    // Эскизы
    // -----------------------------------------------------------------------

    /// Начать эскиз на плоскости (или редактировать существующий).
    pub fn begin_sketch(&mut self, plane: SketchPlane, body_id: Option<u64>) {
        let sketch = body_id
            .and_then(|id| self.scene.body(id))
            .and_then(|b| b.sketch.clone())
            .unwrap_or_else(|| {
                let mut s = crate::sketch::Sketch::new(plane);
                s.name = self.unique_name(t("Эскиз", "Sketch"));
                s
            });
        let prev = self.cam;
        let plane = sketch.plane;
        self.sketch = Some(SketchSession {
            body_id,
            sketch,
            anchor: None,
            chain_start: None,
            pts: Vec::new(),
            hover: None,
            hover_snapped_to_point: false,
            prev_camera: prev,
        });
        self.tab = super::Tab::Sketch;
        self.tool = Tool::Line;
        self.dialog = None;
        // Камера — нормально к плоскости.
        let (_, _, _, n) = plane.frame();
        let mut to = self.cam;
        to.set_view_dir(n);
        to.ortho = false;
        self.animate_camera(to);
        self.invalidate_view();
    }

    fn sketch_snap_step(&self, s: f32) -> f32 {
        let c = self.cam.target;
        nice_step(self.world_per_px(c) * 14.0 * s)
    }

    fn update_sketch_hover(&mut self, mx: f32, my: f32, shift: bool, s: f32) {
        let step = self.sketch_snap_step(s);
        let snap = self.cfg.snap && !shift;
        let ray = self.screen_ray(mx, my);
        let tol = 9.0 * s;
        let Some(sess) = &self.sketch else { return };
        let plane = sess.sketch.plane;
        let Some(p) = ray_plane(&ray, &plane) else {
            if let Some(sm) = &mut self.sketch {
                sm.hover = None;
            }
            return;
        };
        // Привязка к существующим точкам эскиза.
        let mut best: Option<(f32, Vec2)> = None;
        let mut candidates: Vec<Vec2> = sess.sketch.points().iter().map(|q| Vec2::new(q.x, q.y)).collect();
        candidates.push(Vec2::zero());
        if let Some(cs) = sess.chain_start {
            candidates.push(cs);
        }
        for q in candidates {
            if let Some(sp) = self.to_screen(plane.to_world(q)) {
                let d = ((sp.0 - mx).powi(2) + (sp.1 - my).powi(2)).sqrt();
                if d < tol && best.map_or(true, |(bd, _)| d < bd) {
                    best = Some((d, q));
                }
            }
        }
        let (hp, snapped) = match best {
            Some((_, q)) => (q, true),
            None => {
                if snap {
                    (Vec2::new((p.x / step).round() * step, (p.y / step).round() * step), false)
                } else {
                    (p, false)
                }
            }
        };
        if let Some(sm) = &mut self.sketch {
            if sm.hover != Some(hp) {
                sm.hover = Some(hp);
                sm.hover_snapped_to_point = snapped;
            }
        }
    }

    /// Допуск выбора на эскизе в мировых единицах (~10 px).
    fn sketch_pick_tol(&self) -> f32 {
        self.world_per_px(self.cam.target) * 10.0 * self.scale * self.cfg.ui_scale
    }

    fn sketch_click(&mut self) {
        let tool = self.tool;
        let params = self.sketch_params;
        let tol = self.sketch_pick_tol();
        let mut err: Option<String> = None;
        let Some(sess) = &mut self.sketch else { return };
        let Some(p) = sess.hover else { return };
        let v3 = |q: Vec2| Vec3::new(q.x, q.y, 0.0);
        let same = |a: Vec2, b: Vec2| a.distance(b) < 1e-6;
        match tool {
            Tool::Line => match sess.anchor {
                None => {
                    sess.anchor = Some(p);
                    sess.chain_start = Some(p);
                }
                Some(a) => {
                    if !same(a, p) {
                        sess.sketch.entities.push(SketchEntity::Line { a: v3(a), b: v3(p) });
                        if sess.chain_start.map(|c| same(c, p)).unwrap_or(false) {
                            // Контур замкнут.
                            sess.reset_op();
                        } else {
                            sess.anchor = Some(p);
                        }
                    }
                }
            },
            // Два щелчка.
            Tool::Rectangle | Tool::Circle | Tool::Polygon => match sess.anchor {
                None => sess.anchor = Some(p),
                Some(a) => {
                    if let Some(e) = pending_entity(tool, Some(a), &sess.pts, p, &params) {
                        if entity_valid(&e) {
                            sess.sketch.entities.push(e);
                        }
                    }
                    sess.reset_op();
                }
            },
            // Три щелчка.
            Tool::Arc | Tool::Ellipse | Tool::Slot => match sess.anchor {
                None => sess.anchor = Some(p),
                Some(a) if sess.pts.is_empty() => {
                    if !same(a, p) {
                        sess.pts.push(p);
                    }
                }
                Some(a) => {
                    if let Some(e) = pending_entity(tool, Some(a), &sess.pts, p, &params) {
                        if entity_valid(&e) {
                            sess.sketch.entities.push(e);
                        }
                    }
                    sess.reset_op();
                }
            },
            Tool::Spline => {
                if sess.pts.len() >= 3 && same(sess.pts[0], p) {
                    let points = sess.pts.iter().map(|q| v3(*q)).collect();
                    sess.sketch.entities.push(SketchEntity::Spline { points, closed: true });
                    sess.reset_op();
                } else if sess.pts.last().map_or(true, |q| !same(*q, p)) {
                    sess.pts.push(p);
                    sess.anchor = Some(sess.pts[0]);
                    sess.chain_start = Some(sess.pts[0]);
                }
            }
            Tool::SketchFillet => {
                if let Err(e) = sess.sketch.fillet_corner(p, params.fillet_radius, tol) {
                    err = Some(e);
                }
            }
            Tool::SketchOffset => {
                if let Err(e) = sess.sketch.offset_at(p, tol, params.offset) {
                    err = Some(e);
                }
            }
            _ => {}
        }
        if let Some(e) = err {
            let msg = if super::lang_en() { sketch_error_en(&e) } else { e };
            self.toast(msg, true);
        }
        self.invalidate_view();
    }

    /// Завершить многошаговую операцию (ПКМ / Enter): сплайн сохраняется,
    /// остальные операции сбрасываются.
    pub fn sketch_commit(&mut self) {
        let tool = self.tool;
        if let Some(sess) = &mut self.sketch {
            if tool == Tool::Spline && sess.pts.len() >= 2 {
                let points = sess.pts.iter().map(|q| Vec3::new(q.x, q.y, 0.0)).collect();
                sess.sketch.entities.push(SketchEntity::Spline { points, closed: false });
            }
            sess.reset_op();
            self.invalidate_view();
        }
    }

    /// Линии эскизов для 3D-наложения.
    fn sketch_lines(&self, out: &mut Vec<Line3>) {
        let selected = self.scene.selected;
        for b in &self.scene.bodies {
            if !b.visible {
                continue;
            }
            if self.sketch.as_ref().and_then(|s| s.body_id) == Some(b.id) {
                continue;
            }
            let Some(sk) = &b.sketch else { continue };
            let col = if selected == Some(b.id) { 0xFF00_7AD9 } else { 0xE034_3A44 };
            for (a, c) in sketch_segments(sk) {
                out.push(Line3 { a: b.transform.transform_point(a), b: b.transform.transform_point(c), color: col, width: 1.6, depth_test: true });
            }
        }
        if let Some(sess) = &self.sketch {
            let plane = sess.sketch.plane;
            // Сетка плоскости эскиза.
            let step = nice_step(self.world_per_px(self.cam.target) * 14.0 * self.scale * self.cfg.ui_scale);
            let major = step * 5.0;
            let c = plane.to_local(self.cam.target);
            let wpp = self.world_per_px(self.cam.target).max(1e-9);
            let v = self.layout.view;
            let n = ((v.w.max(v.h) * 0.75 * wpp / step).ceil() as i32 + 2).clamp(10, 240);
            let cx = (c.x / major).round() * major;
            let cy = (c.y / major).round() * major;
            let half = n as f32 * step;
            for i in -n..=n {
                let off = i as f32 * step;
                let is_major = (((cx + off) / major).round() * major - (cx + off)).abs() < step * 0.01;
                let fadef = 1.0 - (off.abs() / half).powi(6);
                let a = ((if is_major { 60.0 } else { 26.0 }) * fadef) as u8;
                let col = argb(a, 70, 90, 120);
                let x = cx + off;
                let y = cy + off;
                out.push(Line3 { a: plane.to_world(Vec2::new(x, cy - half)), b: plane.to_world(Vec2::new(x, cy + half)), color: col, width: 1.0, depth_test: false });
                out.push(Line3 { a: plane.to_world(Vec2::new(cx - half, y)), b: plane.to_world(Vec2::new(cx + half, y)), color: col, width: 1.0, depth_test: false });
            }
            // Оси плоскости.
            out.push(Line3 { a: plane.to_world(Vec2::new(cx - half, 0.0)), b: plane.to_world(Vec2::new(cx + half, 0.0)), color: 0xA0D0_4040, width: 1.2, depth_test: false });
            out.push(Line3 { a: plane.to_world(Vec2::new(0.0, cy - half)), b: plane.to_world(Vec2::new(0.0, cy + half)), color: 0xA040_A050, width: 1.2, depth_test: false });
            let col = 0xFF1F_2328;
            for (a, c) in sketch_segments(&sess.sketch) {
                out.push(Line3 { a, b: c, color: col, width: 1.8, depth_test: false });
            }
            // Предпросмотр текущей операции.
            if let Some(h) = sess.hover {
                let pc = 0xFFF0_8A24;
                let w = |q: Vec2| plane.to_world(q);
                if let Some(e) = pending_entity(self.tool, sess.anchor, &sess.pts, h, &self.sketch_params) {
                    let (pts, closed) = e.polyline();
                    for i in 1..pts.len() {
                        out.push(Line3 { a: w(pts[i - 1]), b: w(pts[i]), color: pc, width: 1.8, depth_test: false });
                    }
                    if closed && pts.len() > 2 {
                        out.push(Line3 { a: w(pts[pts.len() - 1]), b: w(pts[0]), color: pc, width: 1.8, depth_test: false });
                    }
                }
                // Вспомогательные линии: радиус, ось эллипса, опорные точки сплайна.
                let guide = 0x80F0_8A24;
                match (self.tool, sess.anchor) {
                    (Tool::Circle | Tool::Polygon, Some(a)) => out.push(Line3 { a: w(a), b: w(h), color: guide, width: 1.0, depth_test: false }),
                    (Tool::Ellipse | Tool::Slot | Tool::Arc, Some(a)) if !sess.pts.is_empty() => out.push(Line3 { a: w(a), b: w(sess.pts[0]), color: guide, width: 1.0, depth_test: false }),
                    (Tool::Spline, _) => {
                        for i in 1..sess.pts.len() {
                            out.push(Line3 { a: w(sess.pts[i - 1]), b: w(sess.pts[i]), color: guide, width: 1.0, depth_test: false });
                        }
                    }
                    _ => {}
                }
                // Подсветка элемента под курсором для смещения.
                if self.tool == Tool::SketchOffset {
                    if let Some(i) = sess.sketch.nearest_entity(h, self.sketch_pick_tol()) {
                        let (pts, closed) = sess.sketch.entities[i].polyline();
                        let hc = 0xFF2F_7FD8;
                        for k in 1..pts.len() {
                            out.push(Line3 { a: w(pts[k - 1]), b: w(pts[k]), color: hc, width: 2.6, depth_test: false });
                        }
                        if closed && pts.len() > 2 {
                            out.push(Line3 { a: w(pts[pts.len() - 1]), b: w(pts[0]), color: hc, width: 2.6, depth_test: false });
                        }
                    }
                }
            }
        }
    }

    fn draw_sketch_2d(&self, ui: &mut Ui) {
        let Some(sess) = &self.sketch else { return };
        let s = ui.s;
        let plane = sess.sketch.plane;
        ui.fb.set_clip(self.layout.view);
        for q in sess.sketch.points() {
            if let Some(p) = self.to_screen(plane.to_world(Vec2::new(q.x, q.y))) {
                let r = 3.0 * s;
                ui.fb.fill_rect(Rect::new(p.0 - r, p.1 - r, r * 2.0, r * 2.0), 0xFF1F_2328);
            }
        }
        if let Some(h) = sess.hover {
            if let Some(p) = self.to_screen(plane.to_world(h)) {
                let col = if sess.hover_snapped_to_point { 0xFF2E_9E5B } else { 0xFFF0_8A24 };
                ui.fb.stroke_circle(p.0, p.1, 5.0 * s, 1.6 * s, col);
                // Подпись размера.
                let f = crate::ui::fmt_num;
                let pending = pending_entity(self.tool, sess.anchor, &sess.pts, h, &self.sketch_params);
                let txt = match (self.tool, sess.anchor, &pending) {
                    (Tool::Line, Some(a), _) => format!("{} {}", f(a.distance(h)), t("мм", "mm")),
                    (Tool::Rectangle, Some(a), _) => format!("{} × {}", f((h.x - a.x).abs()), f((h.y - a.y).abs())),
                    (Tool::Circle, Some(a), _) => format!("⌀ {}", f(a.distance(h) * 2.0)),
                    (_, _, Some(SketchEntity::Arc { radius, start_angle, end_angle, .. })) => {
                        format!("R {}  {}°", f(*radius), f(crate::sketch::arc_sweep(*start_angle, *end_angle) * crate::math::RAD2DEG))
                    }
                    (_, _, Some(SketchEntity::Polygon { radius, sides, .. })) => format!("{}  R {}", sides, f(*radius)),
                    (_, _, Some(SketchEntity::Ellipse { rx, ry, .. })) => format!("{} × {}", f(rx * 2.0), f(ry * 2.0)),
                    (_, _, Some(SketchEntity::Slot { a, b, radius })) => format!("{} × {}", f(a.distance(*b) + radius * 2.0), f(radius * 2.0)),
                    (Tool::Spline, _, _) if !sess.pts.is_empty() => format!("{}: {}", t("точек", "points"), sess.pts.len() + 1),
                    (Tool::SketchFillet, _, _) => format!("R {}", f(self.sketch_params.fillet_radius)),
                    (Tool::SketchOffset, _, _) => format!("{} {}", f(self.sketch_params.offset), t("мм", "mm")),
                    (_, Some(a), _) => format!("{} {}", f(a.distance(h)), t("мм", "mm")),
                    _ => format!("{}, {}", f(h.x), f(h.y)),
                };
                label_box(ui, p.0 + 14.0 * s, p.1 + 12.0 * s, &txt, theme::LABEL_BG(), theme::TEXT());
            }
        }
        ui.fb.reset_clip();
    }

    // -----------------------------------------------------------------------
    // Измерение
    // -----------------------------------------------------------------------

    fn measure_click(&mut self, mx: f32, my: f32, s: f32) {
        let (id, p) = self.pick(mx, my);
        let Some(mut p) = p else {
            self.measure = Default::default();
            self.invalidate_view();
            return;
        };
        // Привязка к ближайшей вершине.
        if let Some(id) = id {
            if let Some(idx) = self.scene.index_of(id) {
                if let Some(dm) = self.display_mesh(idx) {
                    let tf = self.scene.bodies[idx].transform;
                    let tol = 10.0 * s;
                    let mut best = tol;
                    let wpp = self.world_per_px(p);
                    let rad2 = (wpp * tol * 3.0).powi(2);
                    for &q in &dm.positions {
                        let wq = tf.transform_point(q);
                        if (wq - p).len2() > rad2 {
                            continue;
                        }
                        if let Some(sp) = self.to_screen(wq) {
                            let d = ((sp.0 - mx).powi(2) + (sp.1 - my).powi(2)).sqrt();
                            if d < best {
                                best = d;
                                p = wq;
                            }
                        }
                    }
                }
            }
        }
        let m = &mut self.measure;
        if m.a.is_none() || m.b.is_some() {
            m.a = Some(p);
            m.b = None;
        } else {
            m.b = Some(p);
        }
        self.invalidate_view();
    }

    fn draw_measure_2d(&self, ui: &mut Ui) {
        let s = ui.s;
        ui.fb.set_clip(self.layout.view);
        let mk = |ui: &mut Ui, p: Vec3, st: &State| {
            if let Some(sp) = st.to_screen(p) {
                ui.fb.fill_circle(sp.0, sp.1, 5.0 * s, 0xFFFF_FFFF);
                ui.fb.fill_circle(sp.0, sp.1, 3.5 * s, theme::ACCENT());
            }
        };
        if let Some(a) = self.measure.a {
            mk(ui, a, self);
        }
        if let Some(b) = self.measure.b {
            mk(ui, b, self);
        }
        if let (Some(a), Some(b)) = (self.measure.a, self.measure.b) {
            if let (Some(pa), Some(pb)) = (self.to_screen(a), self.to_screen(b)) {
                let d = b - a;
                let mm = t("мм", "mm");
                let txt = format!("{} {mm}", crate::ui::fmt_num(d.len()));
                let sub = format!("ΔX {}  ΔY {}  ΔZ {}", crate::ui::fmt_num(d.x), crate::ui::fmt_num(d.y), crate::ui::fmt_num(d.z));
                let (cx, cy) = ((pa.0 + pb.0) * 0.5, (pa.1 + pb.1) * 0.5);
                label_box(ui, cx + 8.0 * s, cy - 30.0 * s, &txt, theme::ACCENT(), 0xFFFF_FFFF);
                label_box(ui, cx + 8.0 * s, cy - 8.0 * s, &sub, theme::LABEL_BG(), theme::TEXT());
            }
        }
        ui.fb.reset_clip();
    }

    // -----------------------------------------------------------------------
    // Отрисовка 3D
    // -----------------------------------------------------------------------

    pub(super) fn draw_view(&mut self, ui: &mut Ui) {
        let v = self.layout.view;
        self.mouse_cache = ui.input.mouse;
        let navigating = self.is_navigating();
        let mut rs = self.cfg.render_scale;
        if navigating && self.cfg.fast_nav && self.last_render_ms > 28.0 {
            rs *= 0.5;
        }
        let vw = ((v.w * rs) as usize).max(16);
        let vh = ((v.h * rs) as usize).max(16);

        // Ключ состояния: 3D перерисовывается только при изменениях.
        let mut h = DefaultHasher::new();
        for f in [self.cam.target.x, self.cam.target.y, self.cam.target.z, self.cam.yaw, self.cam.pitch, self.cam.distance, rs, self.scale] {
            hash_f(&mut h, f);
        }
        (vw, vh, self.cam.ortho, self.scene.revision, self.scene.selected, self.hover_body, self.cfg.show_grid, self.cfg.dark_bg, theme::is_dark(), self.cfg.gpu, self.gpu.is_some()).hash(&mut h);
        (self.cfg.mode as u8, self.tool, self.gizmo_hover, navigating, self.cfg.crease_deg as u32, self.scene.bodies.len()).hash(&mut h);
        if let Some(Drag::Gizmo(g)) = self.drag {
            g.axis.hash(&mut h);
        }
        if let Some(sess) = &self.sketch {
            sess.sketch.entities.len().hash(&mut h);
            format!("{:?}{:?}{:?}", sess.anchor, sess.hover, sess.sketch.plane).hash(&mut h);
        }
        format!("{:?}{:?}", self.measure.a, self.measure.b).hash(&mut h);
        let preview_key = self.dialog.as_ref().and_then(|d| d.preview_key(self));
        preview_key.hash(&mut h);
        for b in &self.scene.bodies {
            (b.visible, b.material.shading, b.material.color.r, b.material.color.g, b.material.color.b).hash(&mut h);
        }
        let key = h.finish() | 1;

        if key != self.view_key || self.view_fb.width != vw || self.view_fb.height != vh {
            self.view_key = key;
            self.view_fb.resize(vw, vh);
            self.render_3d(rs, navigating, preview_key);
        }
        ui.fb.blit_scaled(&self.view_fb, v);
    }

    fn render_3d(&mut self, rs: f32, navigating: bool, preview_key: Option<u64>) {
        let bounds = self.scene_bounds();
        self.cam.update_clip(&bounds);
        // Предпросмотр команды.
        let mut preview: Option<(Arc<DisplayMesh>, Mat4)> = None;
        if let Some(pk) = preview_key {
            if self.preview_cache.as_ref().map(|(k, _, _)| *k) != Some(pk) {
                self.preview_cache = self.dialog.as_ref().and_then(|d| d.build_preview(self)).map(|(m, tf)| (pk, Arc::new(DisplayMesh::build(&m, false, 35.0)), tf));
                // Перерисовать панель диалога: могла появиться или исчезнуть ошибка.
                self.wake_ms = Some(0);
            }
            preview = self.preview_cache.as_ref().map(|(_, dm, tf)| (dm.clone(), *tf));
        }
        let mut meshes: Vec<(usize, Arc<DisplayMesh>)> = Vec::new();
        let editing = self.sketch.as_ref().and_then(|s| s.body_id);
        // Тело, изменяемое диалогом на месте, скрываем, пока виден предпросмотр.
        let hidden = if preview.is_some() { self.dialog.as_ref().and_then(|d| d.hides_body()) } else { None };
        for i in 0..self.scene.bodies.len() {
            let b = &self.scene.bodies[i];
            if !b.visible || Some(b.id) == editing || (hidden.is_some() && Some(b.id) == hidden) {
                continue;
            }
            if let Some(dm) = self.display_mesh(i) {
                meshes.push((i, dm));
            }
        }
        // Заливка профилей эскизов.
        let mut fills: Vec<(Arc<DisplayMesh>, Mat4)> = Vec::new();
        {
            let mut sketches: Vec<(crate::sketch::Sketch, Mat4)> = Vec::new();
            for b in &self.scene.bodies {
                if b.visible && Some(b.id) != editing {
                    if let Some(sk) = &b.sketch {
                        sketches.push((sk.clone(), b.transform));
                    }
                }
            }
            if let Some(sess) = &self.sketch {
                sketches.push((sess.sketch.clone(), Mat4::identity()));
            }
            for (sk, tf) in sketches {
                let mut hh = DefaultHasher::new();
                format!("{:?}{:?}", sk.entities, sk.plane).hash(&mut hh);
                let k = hh.finish();
                let dm = match self.fill_cache.get(&k) {
                    Some(d) => d.clone(),
                    None => {
                        let Some(m) = sk.extrude(0.0) else { continue };
                        let d = Arc::new(DisplayMesh::build(&m, true, 35.0));
                        if self.fill_cache.len() > 64 {
                            self.fill_cache.clear();
                        }
                        self.fill_cache.insert(k, d.clone());
                        d
                    }
                };
                fills.push((dm, tf));
            }
        }

        let selected = self.scene.selected;
        let mut items: Vec<RenderItem> = Vec::with_capacity(meshes.len() + 2);
        let mut total_tris = 0;
        for (i, dm) in &meshes {
            let b = &self.scene.bodies[*i];
            total_tris += dm.triangle_count();
            let hl = if selected == Some(b.id) {
                Highlight::Selected
            } else if self.hover_body == Some(b.id) {
                Highlight::Hover
            } else {
                Highlight::None
            };
            items.push(RenderItem { id: b.id as u32, mesh: dm, model: b.transform, color: b.material.color, highlight: hl, wire: b.material.shading == 2, ghost: false });
        }
        if let Some((dm, tf)) = &preview {
            items.push(RenderItem { id: 0, mesh: dm, model: *tf, color: Color::rgb(90, 160, 235), highlight: Highlight::None, wire: false, ghost: true });
        }
        for (dm, tf) in &fills {
            items.push(RenderItem { id: 0, mesh: dm, model: *tf, color: Color::rgb(250, 190, 120), highlight: Highlight::None, wire: false, ghost: true });
        }

        let mut lines: Vec<Line3> = Vec::new();
        if self.cfg.show_grid && self.sketch.is_none() {
            let step = software::grid_lines(&self.cam, &mut lines, self.cfg.dark_bg || theme::is_dark());
            lines.push(Line3 { a: Vec3::zero(), b: Vec3::new(0.0, 0.0, step * 5.0), color: 0xC03C_6FD8, width: 1.3, depth_test: true });
        }
        self.sketch_lines(&mut lines);
        if let (Some(a), Some(b)) = (self.measure.a, self.measure.b) {
            lines.push(Line3 { a, b, color: 0xFF0A_7BD8, width: 2.0, depth_test: false });
        }
        let s = self.scale * self.cfg.ui_scale;
        self.gizmo_lines(s, &mut lines);

        let mut settings = RenderSettings::default();
        settings.mode = self.cfg.mode;
        settings.threads = if self.cfg.threads == 0 { software::default_threads() } else { self.cfg.threads };
        settings.line_scale = (s * rs).max(0.6);
        settings.skip_edges = navigating && self.cfg.fast_nav && total_tris > 300_000;
        if self.cfg.dark_bg {
            settings.bg_top = 0xFF4A_5260;
            settings.bg_bottom = 0xFF1E_2228;
            settings.edge_color = 0xD010_1214;
        } else if theme::is_dark() {
            // Тёмная тема: графитовый фон вида.
            settings.bg_top = 0xFF4A_4D53;
            settings.bg_bottom = 0xFF26_282B;
            settings.edge_color = 0xD010_1214;
        }
        if self.cfg.mode == DisplayMode::Wireframe {
            settings.skip_edges = settings.skip_edges && total_tris > 2_000_000;
        }
        let aspect_cam = self.cam;
        // Видеокарта (если выбрана и доступна), иначе — программный рендер.
        let mut gpu_stats = None;
        if self.cfg.gpu {
            if self.gpu.is_none() && self.gpu_error.is_none() {
                match crate::renderer::gpu::probe::check().and_then(|_| crate::renderer::gpu::GpuRenderer::new()) {
                    Ok(g) => self.gpu = Some(g),
                    Err(e) => self.gpu_error = Some(e),
                }
            }
            if let Some(g) = self.gpu.as_mut() {
                match g.render(&mut self.view_fb, &aspect_cam, &items, &lines, &settings) {
                    Ok(st) => gpu_stats = Some(st),
                    Err(e) => {
                        self.gpu = None;
                        self.gpu_error = Some(e);
                    }
                }
            }
        }
        let stats = match gpu_stats {
            Some(st) => st,
            None => software::render(&mut self.view_fb, &aspect_cam, &items, &lines, &settings),
        };
        self.last_render_ms = stats.millis;
        self.last_tris = total_tris;
    }

    pub(super) fn draw_view_overlays(&mut self, ui: &mut Ui) {
        let v = self.layout.view;
        let s = ui.s;
        self.draw_gizmo_2d(ui);
        self.draw_sketch_2d(ui);
        self.draw_measure_2d(ui);

        // Подсказка для пустой сцены.
        if self.scene.bodies.is_empty() && self.sketch.is_none() && self.dialog.is_none() {
            let lines = [
                t("Начните работу", "Get started"),
                t("Откройте модель STL / STEP / 3MF / OBJ / PLY — Ctrl+O, или перетащите файл в окно", "Open an STL / STEP / 3MF / OBJ / PLY model — Ctrl+O, or drop a file here"),
                t("Создайте тело на ленте «Твёрдое тело» или начните эскиз", "Create a body on the Solid tab or start a sketch"),
                t("ЛКМ — вращение, ПКМ/СКМ — сдвиг, колесо — масштаб", "LMB orbit · RMB/MMB pan · wheel zoom"),
            ];
            let cy = v.y + v.h * 0.40;
            for (i, l) in lines.iter().enumerate() {
                let size = if i == 0 { 20.0 } else { 13.0 };
                let tw = ui.text_width(l, size);
                let y = cy + if i == 0 { 0.0 } else { 14.0 * s + i as f32 * 22.0 * s };
                ui.text(v.x + (v.w - tw) * 0.5, y, l, size, if i == 0 { theme::TEXT() } else { theme::TEXT_DIM() });
            }
        }

        // Триада осей в левом нижнем углу.
        let o = (v.x + 46.0 * s, v.bottom() - 46.0 * s);
        let len = 26.0 * s;
        let right = self.cam.right();
        let up = self.cam.up();
        let names = ["X", "Y", "Z"];
        let mut order: Vec<usize> = vec![0, 1, 2];
        order.sort_by(|&a, &b| AXES[a].dot(self.cam.back_dir()).partial_cmp(&AXES[b].dot(self.cam.back_dir())).unwrap_or(std::cmp::Ordering::Equal));
        for i in order {
            let a = AXES[i];
            let e = (o.0 + a.dot(right) * len, o.1 - a.dot(up) * len);
            ui.fb.line(o.0, o.1, e.0, e.1, 2.2 * s, AXIS_COLORS[i]);
            ui.text(e.0 + (e.0 - o.0) * 0.25 - 4.0 * s, e.1 + (e.1 - o.1) * 0.25 - 8.0 * s, names[i], 11.0, AXIS_COLORS[i]);
        }

        // Плашка режима эскиза.
        if let Some(sess) = &self.sketch {
            let txt = format!(
                "{} «{}» · {} {} · {}",
                t("Эскиз", "Sketch"),
                sess.sketch.name,
                t("плоскость", "plane"),
                sess.sketch.plane.label(),
                t("Enter — завершить, Esc — отмена операции, Shift — без привязки", "Enter — finish, Esc — cancel op, Shift — no snap")
            );
            let tw = ui.text_width(&txt, 12.0) + 24.0 * s;
            let r = Rect::new(v.x + (v.w - tw) * 0.5, v.y + 10.0 * s, tw, 28.0 * s);
            ui.fb.fill_rounded(r, 14.0 * s, 0xE6F0_8A24);
            ui.text_in(r, &txt, 12.0, 0xFFFF_FFFF, 1);
        }

        // Статистика производительности.
        let info = format!(
            "{} {} · {:.0} {}",
            fmt_count(self.last_tris),
            t("треуг.", "tris"),
            self.last_render_ms,
            t("мс", "ms")
        );
        let tw = ui.text_width(&info, 10.5);
        ui.text(v.right() - tw - 10.0 * s, v.bottom() - 18.0 * s, &info, 10.5, fade(theme::TEXT_DIM(), 0.9));
    }

    // -----------------------------------------------------------------------
    // ViewCube
    // -----------------------------------------------------------------------

    pub(super) fn draw_viewcube(&mut self, ui: &mut Ui) {
        let r = self.layout.cube;
        ui.region(r);
        let s = ui.s;
        let (cx, cy) = (r.x + r.w * 0.5, r.y + r.h * 0.52);
        let size = r.w * 0.29;
        let right = self.cam.right();
        let up = self.cam.up();
        let back = self.cam.back_dir();
        let proj = |p: Vec3| (cx + p.dot(right) * size, cy - p.dot(up) * size);

        // Наведение: направление вида.
        let (mx, my) = ui.input.mouse;
        let hover_dir = if ui.hovered(r) && self.drag.is_none() {
            let sx = (mx - cx) / size;
            let sy = -(my - cy) / size;
            let origin = right * sx + up * sy + back * 10.0;
            let ray = Ray::new(origin, -back);
            let bx = Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));
            bx.ray_intersect(&ray).map(|t| {
                let p = ray.point_at(t);
                let mut d = Vec3::zero();
                for i in 0..3 {
                    if p.get(i).abs() > 0.55 {
                        d.set(i, p.get(i).signum());
                    }
                }
                d
            })
        } else {
            None
        };


        let faces: [(Vec3, &str, &str); 6] = [
            (Vec3::new(0.0, 0.0, 1.0), "ВЕРХ", "TOP"),
            (Vec3::new(0.0, 0.0, -1.0), "НИЗ", "BOTTOM"),
            (Vec3::new(0.0, -1.0, 0.0), "СПЕРЕДИ", "FRONT"),
            (Vec3::new(0.0, 1.0, 0.0), "СЗАДИ", "BACK"),
            (Vec3::new(1.0, 0.0, 0.0), "СПРАВА", "RIGHT"),
            (Vec3::new(-1.0, 0.0, 0.0), "СЛЕВА", "LEFT"),
        ];
        for (n, ru, en) in faces {
            let facing = n.dot(back);
            if facing <= 0.01 {
                continue;
            }
            // Углы грани.
            let (u, w) = if n.z.abs() > 0.5 { (Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0)) } else if n.x.abs() > 0.5 { (Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0)) } else { (Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0)) };
            let pts: Vec<(f32, f32)> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].iter().map(|&(a, b)| proj(n + u * a + w * b)).collect();
            let shade = 0.80 + 0.20 * facing;
            let dark = theme::is_dark();
            let base = (shade * if dark { 0.40 } else { 1.0 } * 255.0) as u32;
            let mut col = 0xFF00_0000 | (base.min(255) << 16) | ((base.min(255)) << 8) | (base + 6).min(255);
            if hover_dir == Some(n) {
                col = if dark { 0xFF2F_4A66 } else { 0xFFBF_DDF8 };
            }
            ui.fb.fill_polygon(&pts, col);
            ui.fb.polyline(&pts, 1.2 * s, if dark { 0xFF6A_707A } else { 0xFF8A_96A6 }, true);
            if facing > 0.22 {
                let label = if super::lang_en() { en } else { ru };
                let mut fs = 9.0 * facing.sqrt().max(0.75);
                // Подпись должна умещаться в проекцию грани.
                let span = (pts[1].0 - pts[0].0).abs().max((pts[2].0 - pts[3].0).abs()).max((pts[3].0 - pts[0].0).abs());
                let tw0 = ui.text_width(label, fs);
                if tw0 > span * 0.86 && tw0 > 0.0 {
                    fs = (fs * span * 0.86 / tw0).max(6.0);
                }
                let c = proj(n);
                let tw = ui.text_width(label, fs);
                let a = (((facing - 0.22) / 0.2).min(1.0) * 255.0) as u8;
                ui.text(c.0 - tw * 0.5, c.1 - 6.0 * s * facing.sqrt(), label, fs, if theme::is_dark() { argb(a, 0xD0, 0xD3, 0xD8) } else { argb(a, 0x3A, 0x42, 0x4E) });
            }
        }
        // Подсветка ребра/угла.
        if let Some(d) = hover_dir {
            let nz = (d.x != 0.0) as u8 + (d.y != 0.0) as u8 + (d.z != 0.0) as u8;
            if nz > 1 {
                let p = proj(d * 0.92);
                ui.fb.fill_circle(p.0, p.1, 5.0 * s, theme::ACCENT());
            }
            ui.st.cursor = Cursor::Hand;
            if ui.input.pressed[0] {
                let mut to = self.cam;
                to.set_view_dir(d);
                self.animate_camera(to);
            }
        }
        // Кнопка «Домой».
        let hb = Rect::new(r.x, r.y, 22.0 * s, 22.0 * s);
        if ui.icon_button(crate::ui::id_of("cube_home"), hb, crate::ui::icons::Icon::Home, t("Начальный вид (Home)", "Home view (Home)"), false, true) {
            self.run_action(super::Action::ViewHome);
        }
        let ob = Rect::new(r.right() - 22.0 * s, r.y, 22.0 * s, 22.0 * s);
        if ui.icon_button(crate::ui::id_of("cube_fit"), ob, crate::ui::icons::Icon::Fit, t("Вписать (F)", "Fit (F)"), false, true) {
            self.run_action(super::Action::Fit);
        }
    }

    /// Обработка клавиши Esc в контексте вида.
    pub fn escape(&mut self) {
        if self.menu.is_some() {
            self.menu = None;
            return;
        }
        if let Some(sess) = &mut self.sketch {
            if sess.busy() {
                sess.reset_op();
                self.invalidate_view();
                return;
            }
            if self.tool != Tool::Select {
                self.tool = Tool::Select;
                return;
            }
            return;
        }
        if self.dialog.is_some() {
            self.dialog = None;
            self.invalidate_view();
            return;
        }
        if self.tool != Tool::Select {
            self.tool = Tool::Select;
            self.measure = Default::default();
        } else {
            self.scene.selected = None;
        }
        self.invalidate_view();
    }

    /// Обработка Escape для перехвата из горячих клавиш.
    pub fn handle_key_escape(&mut self, ui: &Ui) -> bool {
        ui.key(Key::Escape)
    }
}

/// Элемент, который получится при щелчке в точке `h` (для предпросмотра и фиксации).
pub fn pending_entity(tool: Tool, anchor: Option<Vec2>, pts: &[Vec2], h: Vec2, params: &super::SketchParams) -> Option<SketchEntity> {
    let v3 = |q: Vec2| Vec3::new(q.x, q.y, 0.0);
    let line_dist = |a: Vec2, b: Vec2, p: Vec2| -> f32 {
        let d = (b - a).normalize();
        ((p - a).x * d.y - (p - a).y * d.x).abs()
    };
    match tool {
        Tool::Line => anchor.map(|a| SketchEntity::Line { a: v3(a), b: v3(h) }),
        Tool::Rectangle => anchor.map(|a| SketchEntity::Rect { min: Vec3::new(a.x.min(h.x), a.y.min(h.y), 0.0), max: Vec3::new(a.x.max(h.x), a.y.max(h.y), 0.0) }),
        Tool::Circle => anchor.map(|a| SketchEntity::Circle { center: v3(a), radius: a.distance(h) }),
        Tool::Polygon => anchor.map(|a| SketchEntity::Polygon { center: v3(a), radius: a.distance(h), sides: (params.polygon_sides.round() as u32).clamp(3, 64), rotation: (h.y - a.y).atan2(h.x - a.x) }),
        Tool::Arc => {
            let a = anchor?;
            match pts.first() {
                None => Some(SketchEntity::Line { a: v3(a), b: v3(h) }),
                Some(&b) => SketchEntity::arc_3pt(a, b, h).or(Some(SketchEntity::Line { a: v3(a), b: v3(b) })),
            }
        }
        Tool::Ellipse => {
            let a = anchor?;
            match pts.first() {
                None => Some(SketchEntity::Line { a: v3(a * 2.0 - h), b: v3(h) }),
                Some(&b) => Some(SketchEntity::Ellipse { center: v3(a), rx: a.distance(b), ry: line_dist(a, b, h).max(1e-4), rotation: (b.y - a.y).atan2(b.x - a.x) }),
            }
        }
        Tool::Slot => {
            let a = anchor?;
            match pts.first() {
                None => Some(SketchEntity::Line { a: v3(a), b: v3(h) }),
                Some(&b) => Some(SketchEntity::Slot { a: v3(a), b: v3(b), radius: line_dist(a, b, h).max(1e-4) }),
            }
        }
        Tool::Spline => {
            if pts.is_empty() {
                return None;
            }
            let mut p: Vec<Vec3> = pts.iter().map(|q| v3(*q)).collect();
            p.push(v3(h));
            Some(SketchEntity::Spline { points: p, closed: false })
        }
        _ => None,
    }
}

/// Элемент не вырожден (ненулевые размеры).
fn entity_valid(e: &SketchEntity) -> bool {
    let eps = 1e-6;
    match e {
        SketchEntity::Line { a, b } => a.distance(*b) > eps,
        SketchEntity::Rect { min, max } => max.x - min.x > eps && max.y - min.y > eps,
        SketchEntity::Circle { radius, .. } | SketchEntity::Polygon { radius, .. } | SketchEntity::Arc { radius, .. } => *radius > eps,
        SketchEntity::Ellipse { rx, ry, .. } => *rx > eps && *ry > 1e-3,
        SketchEntity::Slot { a, b, radius } => a.distance(*b) > eps && *radius > 1e-3,
        SketchEntity::Spline { points, .. } => points.len() >= 2,
    }
}

/// Перевод сообщений об ошибках эскиза.
fn sketch_error_en(e: &str) -> String {
    match e {
        "радиус должен быть больше нуля" => "Radius must be positive",
        "щёлкните по углу, где сходятся ровно два отрезка" => "Click a corner where exactly two lines meet",
        "отрезки на одной прямой — скруглять нечего" => "The lines are collinear — nothing to fillet",
        "радиус слишком большой для этих отрезков" => "The radius is too large for these lines",
        "щёлкните по линии эскиза" => "Click a sketch curve",
        "расстояние должно быть ненулевым" => "Distance must be non-zero",
        "слишком большое смещение внутрь" => "The inward offset is too large",
        other => other,
    }
    .to_string()
}

/// Отрезки эскиза в мировых координатах (без трансформации тела).
pub fn sketch_segments(sk: &crate::sketch::Sketch) -> Vec<(Vec3, Vec3)> {
    let plane = sk.plane;
    let mut out = Vec::new();
    for e in &sk.entities {
        let (pts, closed) = e.polyline();
        let w: Vec<Vec3> = pts.iter().map(|p| plane.to_world(*p)).collect();
        for i in 1..w.len() {
            out.push((w[i - 1], w[i]));
        }
        if closed && w.len() > 2 {
            out.push((w[w.len() - 1], w[0]));
        }
    }
    out
}

/// Плашка с текстом.
pub fn label_box(ui: &mut Ui, x: f32, y: f32, text: &str, bg: u32, fg: u32) {
    let s = ui.s;
    let tw = ui.text_width(text, 12.0);
    let r = Rect::new(x, y, tw + 14.0 * s, 22.0 * s);
    ui.fb.shadow(r, 4.0 * s, 3.0 * s, 40);
    ui.fb.fill_rounded(r, 4.0 * s, bg);
    ui.text_in(r, text, 12.0, fg, 1);
}

/// Число с разделителями тысяч.
pub fn fmt_count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('\u{2009}');
        }
        out.push(c);
    }
    out
}
