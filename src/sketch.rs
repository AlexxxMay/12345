//! 2D-эскизы и выдавливание (extrude) — базовый workflow Fusion 360.
//!
//! Эскиз живёт на плоскости (XY/XZ/YZ или со смещением/поворотом),
//! содержит отрезки, окружности, дуги, прямоугольники, многоугольники,
//! эллипсы, пазы и сплайны; углы между отрезками можно скруглять.
//! Выдавливание строит призму: триангуляция профиля (ear clipping
//! с отверстиями) + боковые стенки + крышки.

use crate::math::*;
use crate::mesh::Mesh;

/// Плоскость эскиза.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SketchPlane {
    Xy,
    Xz,
    Yz,
    Custom { origin: Vec3, normal: Vec3 },
    /// Произвольная плоскость с заданными осями (грань модели, текущий вид):
    /// `u`, `v` — единичные взаимно перпендикулярные оси, нормаль — `u × v`.
    Frame { origin: Vec3, u: Vec3, v: Vec3 },
}

impl Default for SketchPlane {
    fn default() -> Self {
        SketchPlane::Xy
    }
}

impl SketchPlane {
    pub fn name(&self) -> &'static str {
        match self {
            SketchPlane::Xy => "XY",
            SketchPlane::Xz => "XZ",
            SketchPlane::Yz => "YZ",
            SketchPlane::Custom { .. } => "Custom",
            SketchPlane::Frame { .. } => "Frame",
        }
    }

    /// Плоскость по точке и нормали с осями, согласованными с камерой,
    /// которая смотрит вдоль нормали (`u` — вправо, `v` — вверх по экрану).
    /// Начало координат эскиза — проекция мирового начала на плоскость.
    pub fn from_point_normal(point: Vec3, normal: Vec3) -> SketchPlane {
        let n = normal.normalize();
        for base in [SketchPlane::Xy, SketchPlane::Xz, SketchPlane::Yz] {
            let (_, _, _, bn) = base.frame();
            if n.dot(bn) > 0.9999 {
                return SketchPlane::offset(base, point.dot(bn));
            }
        }
        // Как у камеры с осью Z вверх: «вверх» — проекция Z, при взгляде сверху/снизу — +Y.
        let z = Vec3::new(0.0, 0.0, 1.0);
        let pz = z - n * z.dot(n);
        let v = if pz.len() > 1e-4 { pz.normalize() } else { Vec3::new(0.0, 1.0, 0.0) };
        let u = v.cross(n).normalize();
        let origin = n * point.dot(n);
        SketchPlane::Frame { origin, u, v }
    }

    /// Стандартная плоскость, смещённая вдоль своей нормали на `offset`.
    pub fn offset(base: SketchPlane, offset: f32) -> SketchPlane {
        let (o, _, _, n) = base.frame();
        if offset.abs() < 1e-9 {
            return base;
        }
        SketchPlane::Custom { origin: o + n * offset, normal: n }
    }

    /// Подпись плоскости для интерфейса: «XY», «XY +10».
    pub fn label(&self) -> String {
        match *self {
            SketchPlane::Custom { origin, normal } => {
                for base in [SketchPlane::Xy, SketchPlane::Xz, SketchPlane::Yz] {
                    let (_, _, _, bn) = base.frame();
                    if normal.normalize().dot(bn) > 0.9999 {
                        let d = origin.dot(bn);
                        return format!("{} {}{}", base.name(), if d >= 0.0 { "+" } else { "−" }, crate::ui::fmt_num(d.abs()));
                    }
                }
                "Custom".into()
            }
            SketchPlane::Frame { origin, u, v } => {
                let n = u.cross(v).normalize();
                let f = |x: f32| crate::ui::fmt_num((x * 100.0).round() / 100.0);
                format!("⊥ ({}, {}, {}) {}", f(n.x), f(n.y), f(n.z), crate::ui::fmt_num(origin.dot(n)))
            }
            _ => self.name().into(),
        }
    }

    pub fn from_name(s: &str) -> Self {
        match s {
            "XZ" => SketchPlane::Xz,
            "YZ" => SketchPlane::Yz,
            _ => SketchPlane::Xy,
        }
    }

    /// (origin, u_axis, v_axis, normal) — локальная система координат эскиза.
    pub fn frame(&self) -> (Vec3, Vec3, Vec3, Vec3) {
        match *self {
            SketchPlane::Xy => (Vec3::zero(), Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0)),
            SketchPlane::Xz => (Vec3::zero(), Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, -1.0, 0.0)),
            SketchPlane::Yz => (Vec3::zero(), Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0)),
            SketchPlane::Custom { origin, normal } => {
                let n = normal.normalize();
                // Смещённые стандартные плоскости сохраняют оси базовой плоскости.
                for base in [SketchPlane::Xy, SketchPlane::Xz, SketchPlane::Yz] {
                    let (_, u, v, bn) = base.frame();
                    if n.dot(bn) > 0.9999 {
                        return (origin, u, v, bn);
                    }
                }
                let u = if n.abs().x < 0.9 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 1.0, 0.0) };
                let u = (u - n * u.dot(n)).normalize();
                let v = n.cross(u).normalize();
                (origin, u, v, n)
            }
            SketchPlane::Frame { origin, u, v } => (origin, u, v, u.cross(v).normalize()),
        }
    }

    /// Локальные 2D → мировые 3D.
    pub fn to_world(&self, p: Vec2) -> Vec3 {
        let (o, u, v, _) = self.frame();
        o + u * p.x + v * p.y
    }

    /// Мировые 3D → локальные 2D.
    pub fn to_local(&self, p: Vec3) -> Vec2 {
        let (o, u, v, _) = self.frame();
        let d = p - o;
        Vec2::new(d.dot(u), d.dot(v))
    }
}

/// Сущность эскиза (в локальных 2D-координатах, храним как Vec3 с z=0 для совместимости).
#[derive(Clone, Debug, PartialEq)]
pub enum SketchEntity {
    Line { a: Vec3, b: Vec3 },
    Circle { center: Vec3, radius: f32 },
    /// Дуга против часовой стрелки от `start_angle` до `end_angle` (радианы).
    Arc { center: Vec3, radius: f32, start_angle: f32, end_angle: f32 },
    Rect { min: Vec3, max: Vec3 },
    /// Правильный многоугольник, вписанный в окружность `radius`;
    /// `rotation` — угол первой вершины.
    Polygon { center: Vec3, radius: f32, sides: u32, rotation: f32 },
    /// Эллипс с полуосями `rx`, `ry`, повёрнутый на `rotation`.
    Ellipse { center: Vec3, rx: f32, ry: f32, rotation: f32 },
    /// Паз: отрезок `a`–`b`, скруглённый радиусом `radius` с обоих концов.
    Slot { a: Vec3, b: Vec3, radius: f32 },
    /// Сплайн через точки (центростремительный Catmull-Rom).
    Spline { points: Vec<Vec3>, closed: bool },
}

fn v2(p: Vec3) -> Vec2 {
    Vec2::new(p.x, p.y)
}

/// Положительный угол дуги CCW от `a0` до `a1` в (0, 2π].
pub fn arc_sweep(a0: f32, a1: f32) -> f32 {
    let mut sweep = a1 - a0;
    while sweep <= 1e-6 {
        sweep += TAU;
    }
    while sweep > TAU + 1e-6 {
        sweep -= TAU;
    }
    sweep
}

/// Точки сплайна Catmull-Rom (центростремительного) через опорные точки.
pub fn spline_points(pts: &[Vec2], closed: bool, per_span: usize) -> Vec<Vec2> {
    let n = pts.len();
    if n < 2 {
        return pts.to_vec();
    }
    if n == 2 && !closed {
        return pts.to_vec();
    }
    let get = |i: isize| -> Vec2 {
        if closed {
            pts[i.rem_euclid(n as isize) as usize]
        } else if i < 0 {
            // Отражение для естественного начала/конца.
            pts[0] * 2.0 - pts[1]
        } else if i as usize >= n {
            pts[n - 1] * 2.0 - pts[n - 2]
        } else {
            pts[i as usize]
        }
    };
    let spans = if closed { n } else { n - 1 };
    let mut out = Vec::with_capacity(spans * per_span + 1);
    for sidx in 0..spans {
        let i = sidx as isize;
        let (p0, p1, p2, p3) = (get(i - 1), get(i), get(i + 1), get(i + 2));
        let tj = |a: Vec2, b: Vec2| a.distance(b).max(1e-6).sqrt();
        let t1 = tj(p0, p1);
        let t2 = t1 + tj(p1, p2);
        let t3 = t2 + tj(p2, p3);
        for k in 0..per_span {
            let t = t1 + (t2 - t1) * k as f32 / per_span as f32;
            let a1 = p0 * ((t1 - t) / t1) + p1 * (t / t1);
            let a2 = p1 * ((t2 - t) / (t2 - t1)) + p2 * ((t - t1) / (t2 - t1));
            let a3 = p2 * ((t3 - t) / (t3 - t2)) + p3 * ((t - t2) / (t3 - t2));
            let b1 = a1 * ((t2 - t) / t2) + a2 * (t / t2);
            let b2 = a2 * ((t3 - t) / (t3 - t1)) + a3 * ((t - t1) / (t3 - t1));
            out.push(b1 * ((t2 - t) / (t2 - t1)) + b2 * ((t - t1) / (t2 - t1)));
        }
    }
    if !closed {
        out.push(pts[n - 1]);
    }
    out
}

/// Окружность через три точки: (центр, радиус). `None` — точки на одной прямой.
pub fn circle_through(a: Vec2, b: Vec2, c: Vec2) -> Option<(Vec2, f32)> {
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    if d.abs() < 1e-9 * (1.0 + a.distance(b) * b.distance(c)) {
        return None;
    }
    let (a2, b2, c2) = (a.x * a.x + a.y * a.y, b.x * b.x + b.y * b.y, c.x * c.x + c.y * c.y);
    let ux = (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d;
    let uy = (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d;
    let center = Vec2::new(ux, uy);
    Some((center, center.distance(a)))
}

impl SketchEntity {
    /// Дуга по трём точкам: начало, конец и точка на дуге.
    pub fn arc_3pt(start: Vec2, end: Vec2, through: Vec2) -> Option<SketchEntity> {
        let (c, r) = circle_through(start, end, through)?;
        let ang = |p: Vec2| (p.y - c.y).atan2(p.x - c.x);
        let (a0, a1, am) = (ang(start), ang(end), ang(through));
        // Дуга хранится CCW; если точка не на CCW-дуге от начала к концу — меняем местами.
        let (s, e) = if arc_sweep(a0, am) < arc_sweep(a0, a1) { (a0, a1) } else { (a1, a0) };
        Some(SketchEntity::Arc { center: Vec3::new(c.x, c.y, 0.0), radius: r, start_angle: s, end_angle: e })
    }

    /// Полилиния элемента в локальных 2D-координатах и признак замкнутости.
    pub fn polyline(&self) -> (Vec<Vec2>, bool) {
        let ring = |n: usize, f: &dyn Fn(f32) -> Vec2| -> Vec<Vec2> { (0..n).map(|i| f(i as f32 / n as f32 * TAU)).collect() };
        match self {
            SketchEntity::Line { a, b } => (vec![v2(*a), v2(*b)], false),
            SketchEntity::Rect { min, max } => (vec![Vec2::new(min.x, min.y), Vec2::new(max.x, min.y), Vec2::new(max.x, max.y), Vec2::new(min.x, max.y)], true),
            SketchEntity::Circle { center, radius } => (ring(64, &|a| Vec2::new(center.x + a.cos() * radius, center.y + a.sin() * radius)), true),
            SketchEntity::Arc { center, radius, start_angle, end_angle } => {
                let sweep = arc_sweep(*start_angle, *end_angle);
                let n = ((sweep / (5.0 * DEG2RAD)).ceil() as usize).clamp(4, 128);
                let pts = (0..=n)
                    .map(|i| {
                        let a = start_angle + sweep * (i as f32 / n as f32);
                        Vec2::new(center.x + a.cos() * radius, center.y + a.sin() * radius)
                    })
                    .collect();
                (pts, false)
            }
            SketchEntity::Polygon { center, radius, sides, rotation } => {
                let n = (*sides).clamp(3, 256) as usize;
                (ring(n, &|a| Vec2::new(center.x + (a + rotation).cos() * radius, center.y + (a + rotation).sin() * radius)), true)
            }
            SketchEntity::Ellipse { center, rx, ry, rotation } => {
                let (c, s) = (rotation.cos(), rotation.sin());
                (
                    ring(72, &|a| {
                        let (x, y) = (a.cos() * rx, a.sin() * ry);
                        Vec2::new(center.x + x * c - y * s, center.y + x * s + y * c)
                    }),
                    true,
                )
            }
            SketchEntity::Slot { a, b, radius } => {
                let (a, b) = (v2(*a), v2(*b));
                let d = b - a;
                let base = d.y.atan2(d.x);
                let mut pts = Vec::new();
                let n = 24;
                // Полуокружность у b (от -90° до +90° относительно направления), затем у a.
                for i in 0..=n {
                    let t = base - FRAC_PI_2 + PI * i as f32 / n as f32;
                    pts.push(Vec2::new(b.x + t.cos() * radius, b.y + t.sin() * radius));
                }
                for i in 0..=n {
                    let t = base + FRAC_PI_2 + PI * i as f32 / n as f32;
                    pts.push(Vec2::new(a.x + t.cos() * radius, a.y + t.sin() * radius));
                }
                (pts, true)
            }
            SketchEntity::Spline { points, closed } => {
                let p: Vec<Vec2> = points.iter().map(|q| v2(*q)).collect();
                (spline_points(&p, *closed, 12), *closed)
            }
        }
    }

    /// Характерные точки (для привязки курсора).
    pub fn key_points(&self) -> Vec<Vec2> {
        match self {
            SketchEntity::Line { a, b } => vec![v2(*a), v2(*b)],
            SketchEntity::Rect { .. } | SketchEntity::Polygon { .. } => self.polyline().0,
            SketchEntity::Circle { center, .. } | SketchEntity::Ellipse { center, .. } => vec![v2(*center)],
            SketchEntity::Arc { center, .. } => {
                let (p, _) = self.polyline();
                vec![v2(*center), p[0], *p.last().unwrap()]
            }
            SketchEntity::Slot { a, b, .. } => vec![v2(*a), v2(*b)],
            SketchEntity::Spline { points, .. } => points.iter().map(|q| v2(*q)).collect(),
        }
    }
}

/// Область профиля: внешний контур и отверстия.
#[derive(Clone, Debug)]
pub struct Region {
    pub outer: Vec<Vec2>,
    pub holes: Vec<Vec<Vec2>>,
}

/// Допуск совпадения концов при сшивании цепочек (мм).
const JOIN_TOL: f32 = 1e-3;

/// Эскиз.
#[derive(Clone, Debug, Default)]
pub struct Sketch {
    pub name: String,
    pub plane: SketchPlane,
    pub entities: Vec<SketchEntity>,
}

impl Sketch {
    pub fn new(plane: SketchPlane) -> Self {
        Self { name: "Эскиз".into(), plane, entities: Vec::new() }
    }

    /// Все характерные точки эскиза (для привязки и AABB).
    pub fn points(&self) -> Vec<Vec3> {
        let mut pts = Vec::new();
        for e in &self.entities {
            match e {
                SketchEntity::Circle { center, radius } => {
                    pts.push(*center);
                    pts.push(*center + Vec3::new(*radius, 0.0, 0.0));
                    pts.push(*center - Vec3::new(*radius, 0.0, 0.0));
                    pts.push(*center + Vec3::new(0.0, *radius, 0.0));
                    pts.push(*center - Vec3::new(0.0, *radius, 0.0));
                }
                _ => pts.extend(e.key_points().into_iter().map(|q| Vec3::new(q.x, q.y, 0.0))),
            }
        }
        pts
    }

    /// Все цепочки эскиза: (точки, замкнута). Отдельные отрезки, дуги и
    /// незамкнутые сплайны сшиваются по совпадающим концам.
    pub fn chains(&self) -> Vec<(Vec<Vec2>, bool)> {
        let mut segments: Vec<(Vec2, Vec2)> = Vec::new();
        let mut out: Vec<(Vec<Vec2>, bool)> = Vec::new();
        for e in &self.entities {
            let (pts, closed) = e.polyline();
            if closed {
                if pts.len() >= 3 {
                    out.push((pts, true));
                }
            } else {
                for w in pts.windows(2) {
                    if w[0].distance(w[1]) > 1e-9 {
                        segments.push((w[0], w[1]));
                    }
                }
            }
        }
        let mut segs = segments;
        while let Some((a, b)) = segs.pop() {
            let mut chain = std::collections::VecDeque::from(vec![a, b]);
            let mut grew = true;
            while grew {
                grew = false;
                let mut i = 0;
                while i < segs.len() {
                    let (s, e) = segs[i];
                    let last = *chain.back().unwrap();
                    let first = *chain.front().unwrap();
                    if s.distance(last) < JOIN_TOL {
                        chain.push_back(e);
                    } else if e.distance(last) < JOIN_TOL {
                        chain.push_back(s);
                    } else if e.distance(first) < JOIN_TOL {
                        chain.push_front(s);
                    } else if s.distance(first) < JOIN_TOL {
                        chain.push_front(e);
                    } else {
                        i += 1;
                        continue;
                    }
                    segs.swap_remove(i);
                    grew = true;
                }
            }
            let mut chain: Vec<Vec2> = chain.into_iter().collect();
            let closed = chain.len() > 2 && chain.first().unwrap().distance(*chain.last().unwrap()) < JOIN_TOL;
            if closed {
                chain.pop();
            }
            if closed && chain.len() >= 3 {
                out.push((chain, true));
            } else if !closed && chain.len() >= 2 {
                out.push((chain, false));
            }
        }
        out
    }

    /// Контуры профиля: список замкнутых полилиний (2D, локальные координаты).
    pub fn profiles(&self) -> Vec<Vec<Vec2>> {
        self.chains().into_iter().filter(|(_, c)| *c).map(|(p, _)| p).collect()
    }

    /// Самая длинная цепочка (траектория для сдвига): точки и замкнутость.
    pub fn longest_path(&self) -> Option<(Vec<Vec2>, bool)> {
        let len = |p: &Vec<Vec2>, closed: bool| -> f32 {
            let mut l: f32 = p.windows(2).map(|w| w[0].distance(w[1])).sum();
            if closed && p.len() > 2 {
                l += p[0].distance(*p.last().unwrap());
            }
            l
        };
        self.chains().into_iter().max_by(|a, b| len(&a.0, a.1).partial_cmp(&len(&b.0, b.1)).unwrap_or(std::cmp::Ordering::Equal))
    }

    /// Области профиля по вложенности контуров: чётная глубина — материал,
    /// нечётная — отверстие в ближайшем охватывающем контуре.
    pub fn regions(&self) -> Vec<Region> {
        regions_from_profiles(self.profiles())
    }

    /// Скругление угла между двумя отрезками в точке `p` радиусом `r`.
    /// Прямоугольники, которым принадлежит угол, разбиваются на отрезки.
    pub fn fillet_corner(&mut self, p: Vec2, r: f32, tol: f32) -> Result<(), String> {
        if r <= 0.0 {
            return Err("радиус должен быть больше нуля".into());
        }
        // Прямоугольник с этим углом → четыре отрезка.
        if let Some(i) = self.entities.iter().position(|e| matches!(e, SketchEntity::Rect { .. }) && e.polyline().0.iter().any(|q| q.distance(p) < tol)) {
            let (pts, _) = self.entities[i].polyline();
            self.entities.remove(i);
            for k in 0..4 {
                let (a, b) = (pts[k], pts[(k + 1) % 4]);
                self.entities.insert(i + k, SketchEntity::Line { a: Vec3::new(a.x, a.y, 0.0), b: Vec3::new(b.x, b.y, 0.0) });
            }
        }
        // Два отрезка с общим концом в p.
        let mut hits: Vec<(usize, bool)> = Vec::new(); // (индекс, угол на конце a?)
        let mut corner = p;
        for (i, e) in self.entities.iter().enumerate() {
            if let SketchEntity::Line { a, b } = e {
                if v2(*a).distance(p) < tol {
                    hits.push((i, true));
                    corner = v2(*a);
                } else if v2(*b).distance(p) < tol {
                    hits.push((i, false));
                    corner = v2(*b);
                }
            }
        }
        if hits.len() != 2 {
            return Err("щёлкните по углу, где сходятся ровно два отрезка".into());
        }
        let far = |i: usize, at_a: bool| -> Vec2 {
            match self.entities[i] {
                SketchEntity::Line { a, b } => {
                    if at_a {
                        v2(b)
                    } else {
                        v2(a)
                    }
                }
                _ => unreachable!(),
            }
        };
        let (f1, f2) = (far(hits[0].0, hits[0].1), far(hits[1].0, hits[1].1));
        let (l1, l2) = (f1.distance(corner), f2.distance(corner));
        let d1 = (f1 - corner) * (1.0 / l1.max(1e-9));
        let d2 = (f2 - corner) * (1.0 / l2.max(1e-9));
        let cosang = (d1.x * d2.x + d1.y * d2.y).clamp(-1.0, 1.0);
        let theta = cosang.acos();
        if theta < 1e-3 || theta > PI - 1e-3 {
            return Err("отрезки на одной прямой — скруглять нечего".into());
        }
        let tdist = r / (theta * 0.5).tan();
        if tdist > l1 - 1e-6 || tdist > l2 - 1e-6 {
            return Err("радиус слишком большой для этих отрезков".into());
        }
        let t1 = corner + d1 * tdist;
        let t2 = corner + d2 * tdist;
        let bis = d1 + d2;
        let bis = bis * (1.0 / bis.len().max(1e-9));
        let c = corner + bis * (r / (theta * 0.5).sin());
        let ang = |q: Vec2| (q.y - c.y).atan2(q.x - c.x);
        let (a1, a2) = (ang(t1), ang(t2));
        let (s, e) = if arc_sweep(a1, a2) <= PI { (a1, a2) } else { (a2, a1) };
        for (k, &(i, at_a)) in hits.iter().enumerate() {
            let tp = if k == 0 { t1 } else { t2 };
            if let SketchEntity::Line { a, b } = &mut self.entities[i] {
                if at_a {
                    *a = Vec3::new(tp.x, tp.y, 0.0);
                } else {
                    *b = Vec3::new(tp.x, tp.y, 0.0);
                }
            }
        }
        self.entities.push(SketchEntity::Arc { center: Vec3::new(c.x, c.y, 0.0), radius: r, start_angle: s, end_angle: e });
        Ok(())
    }

    /// Выдавливание профиля на `distance` вдоль нормали плоскости.
    /// Возвращает меш в мировых координатах.
    pub fn extrude(&self, distance: f32) -> Option<Mesh> {
        let regions = self.regions();
        if regions.is_empty() {
            return None;
        }
        let (_, _, _, normal) = self.plane.frame();
        let mut mesh = Mesh::new();
        for region in &regions {
            let Some(tri) = triangulate_region(region) else { continue };
            let bottom_base = mesh.positions.len() as u32;
            for p in &tri.verts {
                mesh.add_vertex(self.plane.to_world(*p));
            }
            let top_base = mesh.positions.len() as u32;
            for p in &tri.verts {
                mesh.add_vertex(self.plane.to_world(*p) + normal * distance);
            }
            for t in &tri.tris {
                mesh.add_triangle(bottom_base + t[0], bottom_base + t[2], bottom_base + t[1]);
                mesh.add_triangle(top_base + t[0], top_base + t[1], top_base + t[2]);
            }
            let mut contours: Vec<(&Vec<Vec2>, bool)> = vec![(&region.outer, false)];
            contours.extend(region.holes.iter().map(|h| (h, true)));
            for (contour, is_hole) in contours {
                let ccw = signed_area(contour) > 0.0;
                let flip = ccw == is_hole;
                let n = contour.len();
                for i in 0..n {
                    let j = (i + 1) % n;
                    let b0 = bottom_base + contour_index(&tri.verts, contour[i]);
                    let b1 = bottom_base + contour_index(&tri.verts, contour[j]);
                    let t0 = top_base + contour_index(&tri.verts, contour[i]);
                    let t1 = top_base + contour_index(&tri.verts, contour[j]);
                    if flip {
                        mesh.add_triangle(b0, t1, b1);
                        mesh.add_triangle(b0, t0, t1);
                    } else {
                        mesh.add_triangle(b0, b1, t1);
                        mesh.add_triangle(b0, t1, t0);
                    }
                }
            }
        }
        if mesh.indices.is_empty() {
            return None;
        }
        mesh.weld();
        if distance < 0.0 {
            mesh.flip_winding();
        }
        mesh.compute_smooth_normals();
        mesh.ensure_outward();
        Some(mesh)
    }
}

/// Расстояние от точки до отрезка.
pub fn dist_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let l2 = ab.len2();
    if l2 < 1e-18 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / l2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Эквидистанта полилинии. Для замкнутой `d > 0` — наружу, для открытой —
/// влево по направлению обхода. Углы — со срезом-«митрой» (не дальше 4·d).
pub fn offset_polyline(pts: &[Vec2], closed: bool, d: f32) -> Vec<Vec2> {
    let n = pts.len();
    if n < 2 {
        return pts.to_vec();
    }
    // Нормаль «влево» у отрезка; для CCW-контура лево — внутрь, поэтому знак меняем.
    let sign = if closed && signed_area(pts) > 0.0 { -1.0 } else { 1.0 };
    let left = |a: Vec2, b: Vec2| -> Vec2 {
        let t = (b - a).normalize();
        Vec2::new(-t.y, t.x)
    };
    (0..n)
        .map(|i| {
            let prev = if i > 0 { Some(pts[i - 1]) } else if closed { Some(pts[n - 1]) } else { None };
            let next = if i + 1 < n { Some(pts[i + 1]) } else if closed { Some(pts[0]) } else { None };
            let p = pts[i];
            let nrm = match (prev, next) {
                (Some(a), Some(b)) => {
                    let (n1, n2) = (left(a, p), left(p, b));
                    let m = (n1 + n2).normalize();
                    let c = m.dot(n1).max(0.25);
                    m * (1.0 / c)
                }
                (Some(a), None) => left(a, p),
                (None, Some(b)) => left(p, b),
                _ => Vec2::zero(),
            };
            p + nrm * (d * sign)
        })
        .collect()
}

impl SketchEntity {
    /// Эквидистанта элемента на расстояние `d` (наружу для замкнутых, влево для открытых).
    pub fn offset(&self, d: f32) -> Option<SketchEntity> {
        let ok = |r: f32| r > 1e-6;
        let v3 = |q: Vec2| Vec3::new(q.x, q.y, 0.0);
        match self {
            SketchEntity::Circle { center, radius } => ok(radius + d).then(|| SketchEntity::Circle { center: *center, radius: radius + d }),
            SketchEntity::Arc { center, radius, start_angle, end_angle } => {
                ok(radius + d).then(|| SketchEntity::Arc { center: *center, radius: radius + d, start_angle: *start_angle, end_angle: *end_angle })
            }
            SketchEntity::Rect { min, max } => {
                let (mn, mx) = (*min - Vec3::new(d, d, 0.0), *max + Vec3::new(d, d, 0.0));
                (mx.x - mn.x > 1e-6 && mx.y - mn.y > 1e-6).then(|| SketchEntity::Rect { min: mn, max: mx })
            }
            SketchEntity::Polygon { center, radius, sides, rotation } => {
                let r = radius + d / (PI / (*sides).max(3) as f32).cos();
                ok(r).then(|| SketchEntity::Polygon { center: *center, radius: r, sides: *sides, rotation: *rotation })
            }
            SketchEntity::Slot { a, b, radius } => ok(radius + d).then(|| SketchEntity::Slot { a: *a, b: *b, radius: radius + d }),
            SketchEntity::Line { a, b } => {
                let o = offset_polyline(&[v2(*a), v2(*b)], false, d);
                Some(SketchEntity::Line { a: v3(o[0]), b: v3(o[1]) })
            }
            SketchEntity::Ellipse { .. } | SketchEntity::Spline { .. } => {
                let (pts, closed) = self.polyline();
                // Прореживаем до ~36 опорных точек и строим сплайн.
                let step = (pts.len() / 36).max(1);
                let mut sel: Vec<Vec2> = pts.iter().step_by(step).copied().collect();
                if !closed && sel.last() != pts.last() {
                    sel.push(*pts.last().unwrap());
                }
                let o = offset_polyline(&sel, closed, d);
                if closed && signed_area(&o).abs() < 1e-9 {
                    return None;
                }
                Some(SketchEntity::Spline { points: o.into_iter().map(v3).collect(), closed })
            }
        }
    }
}

impl Sketch {
    /// Ближайший к точке элемент (в пределах `tol`).
    pub fn nearest_entity(&self, p: Vec2, tol: f32) -> Option<usize> {
        let mut best: Option<(f32, usize)> = None;
        for (i, e) in self.entities.iter().enumerate() {
            let (pts, closed) = e.polyline();
            let mut d = f32::MAX;
            for w in pts.windows(2) {
                d = d.min(dist_to_segment(p, w[0], w[1]));
            }
            if closed && pts.len() > 2 {
                d = d.min(dist_to_segment(p, pts[pts.len() - 1], pts[0]));
            }
            if d < tol && best.map_or(true, |(bd, _)| d < bd) {
                best = Some((d, i));
            }
        }
        best.map(|(_, i)| i)
    }

    /// Смещение элемента под точкой `p`. Отрезки, соединённые в цепочку,
    /// смещаются вместе (с подрезкой углов). Возвращает число новых элементов.
    pub fn offset_at(&mut self, p: Vec2, tol: f32, d: f32) -> Result<usize, String> {
        let Some(i) = self.nearest_entity(p, tol) else {
            return Err("щёлкните по линии эскиза".into());
        };
        if d.abs() < 1e-9 {
            return Err("расстояние должно быть ненулевым".into());
        }
        if let SketchEntity::Line { .. } = self.entities[i] {
            // Цепочка отрезков, содержащая выбранный.
            let lines: Vec<(Vec2, Vec2)> = self.entities.iter().filter_map(|e| if let SketchEntity::Line { a, b } = e { Some((v2(*a), v2(*b))) } else { None }).collect();
            let picked = if let SketchEntity::Line { a, b } = self.entities[i] { (v2(a), v2(b)) } else { unreachable!() };
            let tmp = Sketch { name: String::new(), plane: self.plane, entities: lines.iter().map(|(a, b)| SketchEntity::Line { a: Vec3::new(a.x, a.y, 0.0), b: Vec3::new(b.x, b.y, 0.0) }).collect() };
            let near = |q: Vec2| q.distance(picked.0) < JOIN_TOL || q.distance(picked.1) < JOIN_TOL;
            for (pts, closed) in tmp.chains() {
                let n = pts.len();
                let segs = if closed { n } else { n - 1 };
                let contains = (0..segs).any(|k| near(pts[k]) && near(pts[(k + 1) % n]));
                if !contains {
                    continue;
                }
                let o = offset_polyline(&pts, closed, d);
                for k in 0..segs {
                    let (a, b) = (o[k], o[(k + 1) % n]);
                    self.entities.push(SketchEntity::Line { a: Vec3::new(a.x, a.y, 0.0), b: Vec3::new(b.x, b.y, 0.0) });
                }
                return Ok(segs);
            }
        }
        match self.entities[i].offset(d) {
            Some(e) => {
                self.entities.push(e);
                Ok(1)
            }
            None => Err("слишком большое смещение внутрь".into()),
        }
    }
}

/// Точка внутри многоугольника (чётность пересечений).
pub fn point_in_polygon(p: Vec2, poly: &[Vec2]) -> bool {
    let mut inside = false;
    let n = poly.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Разбор замкнутых контуров на области «внешний контур + отверстия».
pub fn regions_from_profiles(profiles: Vec<Vec<Vec2>>) -> Vec<Region> {
    let n = profiles.len();
    let areas: Vec<f32> = profiles.iter().map(|p| signed_area(p).abs()).collect();
    // Пробная точка контура — середина первого ребра, слегка смещённая внутрь
    // не нужна: контуры эскиза не пересекаются, достаточно вершины.
    let probe = |p: &Vec<Vec2>| (p[0] + p[1]) * 0.5;
    let contains = |outer: usize, inner: usize| areas[outer] > areas[inner] && point_in_polygon(probe(&profiles[inner]), &profiles[outer]);
    let mut parent: Vec<Option<usize>> = vec![None; n];
    let mut depth = vec![0usize; n];
    for i in 0..n {
        let mut best: Option<usize> = None;
        for j in 0..n {
            if i != j && contains(j, i) {
                depth[i] += 1;
                if best.map_or(true, |b| areas[j] < areas[b]) {
                    best = Some(j);
                }
            }
        }
        parent[i] = best;
    }
    let mut regions: Vec<(usize, Region)> = Vec::new();
    for i in 0..n {
        if depth[i] % 2 == 0 {
            regions.push((i, Region { outer: profiles[i].clone(), holes: Vec::new() }));
        }
    }
    for i in 0..n {
        if depth[i] % 2 == 1 {
            if let Some(par) = parent[i] {
                if let Some((_, r)) = regions.iter_mut().find(|(k, _)| *k == par) {
                    r.holes.push(profiles[i].clone());
                }
            }
        }
    }
    regions.into_iter().map(|(_, r)| r).collect()
}

/// Триангуляция области (вершины и треугольники в 2D).
pub struct Triangulation {
    pub verts: Vec<Vec2>,
    pub tris: Vec<[u32; 3]>,
}

/// Триангуляция области с отверстиями.
pub fn triangulate_region(r: &Region) -> Option<Triangulation> {
    let holes: Vec<&Vec<Vec2>> = r.holes.iter().collect();
    triangulate_profile(&r.outer, &holes)
}

fn contour_index(verts: &[Vec2], p: Vec2) -> u32 {
    verts
        .iter()
        .position(|v| v.distance(p) < 1e-6)
        .unwrap_or(0) as u32
}

pub fn signed_area(poly: &[Vec2]) -> f32 {
    let mut s = 0.0;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        s += a.x * b.y - b.x * a.y;
    }
    s * 0.5
}


/// Триангуляция профиля с отверстиями (мосты + ear clipping).
fn triangulate_profile(outer: &[Vec2], holes: &[&Vec<Vec2>]) -> Option<Triangulation> {
    if outer.len() < 3 {
        return None;
    }
    let mut poly: Vec<Vec2> = outer.to_vec();
    if signed_area(&poly) < 0.0 {
        poly.reverse();
    }
    for hole in holes {
        if hole.len() < 3 {
            continue;
        }
        let mut h: Vec<Vec2> = (*hole).clone();
        if signed_area(&h) > 0.0 {
            h.reverse();
        }
        // Ближайшая пара.
        let mut best: Option<(usize, usize, f32)> = None;
        for (i, hp) in h.iter().enumerate() {
            for (j, op) in poly.iter().enumerate() {
                let d = hp.distance(*op);
                if best.map_or(true, |(_, _, bd)| d < bd) {
                    best = Some((i, j, d));
                }
            }
        }
        let (hi, oi, _) = best?;
        // Мост: outer[..=oi], hole[hi..], outer[oi], outer[oi+1..]
        let mut insert: Vec<Vec2> = Vec::new();
        for k in 0..h.len() {
            insert.push(h[(hi + k) % h.len()]);
        }
        insert.push(h[hi]);
        insert.push(poly[oi]);
        for (k, v) in insert.into_iter().enumerate() {
            poly.insert(oi + 1 + k, v);
        }
    }
    let tris = ear_clip(&poly);
    if tris.is_empty() {
        return None;
    }
    Some(Triangulation {
        verts: poly,
        tris: tris
            .chunks_exact(3)
            .map(|t| [t[0] as u32, t[1] as u32, t[2] as u32])
            .collect(),
    })
}

fn sign(p1: Vec2, p2: Vec2, p3: Vec2) -> f32 {
    (p1.x - p3.x) * (p2.y - p3.y) - (p2.x - p3.x) * (p1.y - p3.y)
}

fn point_in_tri(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let d1 = sign(p, a, b);
    let d2 = sign(p, b, c);
    let d3 = sign(p, c, a);
    let has_neg = d1 < -1e-12 || d2 < -1e-12 || d3 < -1e-12;
    let has_pos = d1 > 1e-12 || d2 > 1e-12 || d3 > 1e-12;
    !(has_neg && has_pos)
}

pub fn ear_clip(poly: &[Vec2]) -> Vec<usize> {
    let n = poly.len();
    if n < 3 {
        return Vec::new();
    }
    let mut idx: Vec<usize> = (0..n).collect();
    if signed_area(poly) < 0.0 {
        idx.reverse();
    }
    let mut tris = Vec::new();
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n * 2 {
        guard += 1;
        let mut clipped = false;
        for i in 0..idx.len() {
            let i0 = idx[(i + idx.len() - 1) % idx.len()];
            let i1 = idx[i];
            let i2 = idx[(i + 1) % idx.len()];
            let (a, b, c) = (poly[i0], poly[i1], poly[i2]);
            if sign(a, b, c) <= 1e-12 {
                continue;
            }
            let mut blocked = false;
            for &j in &idx {
                if j == i0 || j == i1 || j == i2 {
                    continue;
                }
                if poly[j].distance(a) < 1e-9 || poly[j].distance(b) < 1e-9 || poly[j].distance(c) < 1e-9 {
                    continue;
                }
                if point_in_tri(poly[j], a, b, c) {
                    blocked = true;
                    break;
                }
            }
            if blocked {
                continue;
            }
            tris.push(i0);
            tris.push(i1);
            tris.push(i2);
            idx.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            // Убираем вырожденную (коллинеарную) вершину, если она есть.
            let mut removed = false;
            for i in 0..idx.len() {
                let i0 = idx[(i + idx.len() - 1) % idx.len()];
                let i1 = idx[i];
                let i2 = idx[(i + 1) % idx.len()];
                if sign(poly[i0], poly[i1], poly[i2]).abs() <= 1e-9 {
                    idx.remove(i);
                    removed = true;
                    break;
                }
            }
            if !removed {
                break;
            }
        }
    }
    if idx.len() == 3 {
        tris.extend_from_slice(&idx);
    }
    tris
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_sketch() -> Sketch {
        let mut sk = Sketch::new(SketchPlane::Xy);
        sk.entities.push(SketchEntity::Rect {
            min: Vec3::new(0.0, 0.0, 0.0),
            max: Vec3::new(10.0, 10.0, 0.0),
        });
        sk
    }

    #[test]
    fn extrude_rect() {
        let sk = rect_sketch();
        let m = sk.extrude(5.0).unwrap();
        assert!((m.volume().abs() - 500.0).abs() < 1e-2, "vol={}", m.volume());
        assert!(m.is_closed());
    }

    #[test]
    fn extrude_with_hole() {
        let mut sk = rect_sketch();
        sk.entities.push(SketchEntity::Circle {
            center: Vec3::new(5.0, 5.0, 0.0),
            radius: 2.0,
        });
        let m = sk.extrude(5.0).unwrap();
        let expect = (100.0 - PI * 4.0) * 5.0;
        assert!((m.volume().abs() - expect).abs() < 5.0, "vol={} expect={expect}", m.volume());
    }

    #[test]
    fn extrude_circle() {
        let mut sk = Sketch::new(SketchPlane::Xy);
        sk.entities.push(SketchEntity::Circle {
            center: Vec3::zero(),
            radius: 5.0,
        });
        let m = sk.extrude(10.0).unwrap();
        let expect = PI * 25.0 * 10.0;
        assert!((m.volume().abs() - expect).abs() < 5.0, "vol={} expect={expect}", m.volume());
    }

    #[test]
    fn new_entities_are_closed_profiles() {
        let mut sk = Sketch::new(SketchPlane::Xy);
        sk.entities.push(SketchEntity::Polygon { center: Vec3::zero(), radius: 10.0, sides: 6, rotation: 0.0 });
        let m = sk.extrude(2.0).unwrap();
        let hex = 1.5 * 3f32.sqrt() * 100.0;
        assert!((m.volume() - hex * 2.0).abs() < 0.1, "vol={}", m.volume());
        let mut sk = Sketch::new(SketchPlane::Xy);
        sk.entities.push(SketchEntity::Ellipse { center: Vec3::zero(), rx: 10.0, ry: 5.0, rotation: 0.3 });
        let m = sk.extrude(1.0).unwrap();
        assert!((m.volume() - PI * 50.0).abs() / (PI * 50.0) < 0.01, "vol={}", m.volume());
        let mut sk = Sketch::new(SketchPlane::Xy);
        sk.entities.push(SketchEntity::Slot { a: Vec3::zero(), b: Vec3::new(20.0, 0.0, 0.0), radius: 3.0 });
        let m = sk.extrude(1.0).unwrap();
        let expect = 20.0 * 6.0 + PI * 9.0;
        assert!((m.volume() - expect).abs() / expect < 0.01, "vol={}", m.volume());
        let mut sk = Sketch::new(SketchPlane::Xy);
        let pts = vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0), Vec3::new(10.0, 10.0, 0.0), Vec3::new(0.0, 10.0, 0.0)];
        sk.entities.push(SketchEntity::Spline { points: pts, closed: true });
        assert_eq!(sk.profiles().len(), 1);
        assert!(sk.extrude(1.0).unwrap().is_closed());
    }

    #[test]
    fn arc_and_lines_form_profile() {
        // Полукруг: дуга по трём точкам + отрезок-диаметр.
        let mut sk = Sketch::new(SketchPlane::Xy);
        let arc = SketchEntity::arc_3pt(Vec2::new(-5.0, 0.0), Vec2::new(5.0, 0.0), Vec2::new(0.0, 5.0)).unwrap();
        sk.entities.push(arc);
        sk.entities.push(SketchEntity::Line { a: Vec3::new(-5.0, 0.0, 0.0), b: Vec3::new(5.0, 0.0, 0.0) });
        assert_eq!(sk.profiles().len(), 1);
        let m = sk.extrude(1.0).unwrap();
        assert!((m.volume() - PI * 12.5).abs() < 0.2, "vol={}", m.volume());
        // Точка снизу — дуга идёт через низ.
        let a2 = SketchEntity::arc_3pt(Vec2::new(-5.0, 0.0), Vec2::new(5.0, 0.0), Vec2::new(0.0, -5.0)).unwrap();
        let (pts, _) = a2.polyline();
        assert!(pts.iter().all(|p| p.y <= 1e-3));
    }

    #[test]
    fn fillet_rect_corner() {
        let mut sk = rect_sketch();
        sk.fillet_corner(Vec2::new(10.0, 10.0), 2.0, 0.01).unwrap();
        assert_eq!(sk.entities.len(), 5);
        assert_eq!(sk.profiles().len(), 1);
        let m = sk.extrude(1.0).unwrap();
        let expect = 100.0 - (4.0 - PI);
        assert!((m.volume() - expect).abs() < 0.05, "vol={} expect={expect}", m.volume());
        assert!(sk.fillet_corner(Vec2::new(0.0, 0.0), 20.0, 0.01).is_err());
    }

    #[test]
    fn nested_regions() {
        // Прямоугольник с отверстием и островком внутри отверстия.
        let mut sk = Sketch::new(SketchPlane::Xy);
        sk.entities.push(SketchEntity::Rect { min: Vec3::new(-10.0, -10.0, 0.0), max: Vec3::new(10.0, 10.0, 0.0) });
        sk.entities.push(SketchEntity::Circle { center: Vec3::zero(), radius: 6.0 });
        sk.entities.push(SketchEntity::Circle { center: Vec3::zero(), radius: 3.0 });
        let r = sk.regions();
        assert_eq!(r.len(), 2);
        let m = sk.extrude(1.0).unwrap();
        let expect = 400.0 - PI * 36.0 + PI * 9.0;
        assert!((m.volume() - expect).abs() < 0.5, "vol={} expect={expect}", m.volume());
    }

    #[test]
    fn offset_entities() {
        let mut sk = rect_sketch();
        sk.offset_at(Vec2::new(10.0, 5.0), 0.5, 1.0).unwrap();
        assert_eq!(sk.entities[1], SketchEntity::Rect { min: Vec3::new(-1.0, -1.0, 0.0), max: Vec3::new(11.0, 11.0, 0.0) });
        // Квадрат из отрезков смещается целиком внутрь.
        let mut sk = Sketch::new(SketchPlane::Xy);
        let c = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        for k in 0..4 {
            let (a, b) = (c[k], c[(k + 1) % 4]);
            sk.entities.push(SketchEntity::Line { a: Vec3::new(a.0, a.1, 0.0), b: Vec3::new(b.0, b.1, 0.0) });
        }
        assert_eq!(sk.offset_at(Vec2::new(5.0, 0.1), 0.5, -2.0).unwrap(), 4);
        let regions = sk.regions();
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].holes.len(), 1);
        assert!((signed_area(&regions[0].holes[0]).abs() - 36.0).abs() < 1e-3);
        // Окружность и слишком большое смещение внутрь.
        let mut sk = Sketch::new(SketchPlane::Xy);
        sk.entities.push(SketchEntity::Circle { center: Vec3::zero(), radius: 3.0 });
        assert!(sk.offset_at(Vec2::new(3.0, 0.0), 0.5, -5.0).is_err());
        assert!(sk.offset_at(Vec2::new(20.0, 0.0), 0.5, 1.0).is_err());
    }

    #[test]
    fn offset_plane_keeps_axes() {
        let p = SketchPlane::offset(SketchPlane::Xz, 15.0);
        let (o, u, v, n) = p.frame();
        let (_, u0, v0, n0) = SketchPlane::Xz.frame();
        assert!(u.distance(u0) < 1e-6 && v.distance(v0) < 1e-6 && n.distance(n0) < 1e-6);
        assert!((o.y + 15.0).abs() < 1e-6);
        assert_eq!(p.label(), "XZ +15");
    }

    #[test]
    fn profiles_chain_lines() {
        let mut sk = Sketch::new(SketchPlane::Xy);
        sk.entities.push(SketchEntity::Line { a: Vec3::new(0.0, 0.0, 0.0), b: Vec3::new(1.0, 0.0, 0.0) });
        sk.entities.push(SketchEntity::Line { a: Vec3::new(1.0, 0.0, 0.0), b: Vec3::new(1.0, 1.0, 0.0) });
        sk.entities.push(SketchEntity::Line { a: Vec3::new(1.0, 1.0, 0.0), b: Vec3::new(0.0, 1.0, 0.0) });
        sk.entities.push(SketchEntity::Line { a: Vec3::new(0.0, 1.0, 0.0), b: Vec3::new(0.0, 0.0, 0.0) });
        let p = sk.profiles();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].len(), 4);
    }

    #[test]
    fn frame_plane_matches_camera() {
        use crate::renderer::software::Camera;
        for n in [Vec3::new(1.0, -1.0, 0.8), Vec3::new(0.3, 0.2, -0.9), Vec3::new(0.0, 1.0, 0.2)] {
            let n = n.normalize();
            let pl = SketchPlane::from_point_normal(Vec3::new(5.0, 6.0, 7.0), n);
            let (o, u, v, nn) = pl.frame();
            let mut cam = Camera::default();
            cam.set_view_dir(n);
            assert!(nn.dot(n) > 0.9999);
            assert!(u.dot(cam.right()) > 0.999, "{u:?} {:?}", cam.right());
            assert!(v.dot(cam.up()) > 0.999);
            // Точка лежит на плоскости, начало — проекция мирового начала.
            assert!((Vec3::new(5.0, 6.0, 7.0) - o).dot(nn).abs() < 1e-4);
            assert!(o.cross(nn).len() < 1e-4);
        }
        // Плоскость, параллельная стандартной, остаётся смещённой стандартной.
        let p = SketchPlane::from_point_normal(Vec3::new(0.0, 0.0, 12.0), Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(p.label(), "XY +12");
    }
}
