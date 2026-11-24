//! 2D-эскизы и выдавливание (extrude) — базовый workflow Fusion 360.
//!
//! Эскиз живёт на плоскости (XY/XZ/YZ или со смещением/поворотом),
//! содержит линии, окружности, дуги, прямоугольники.
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
                let u = if n.abs().x < 0.9 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 1.0, 0.0) };
                let u = (u - n * u.dot(n)).normalize();
                let v = n.cross(u).normalize();
                (origin, u, v, n)
            }
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
    Arc { center: Vec3, radius: f32, start_angle: f32, end_angle: f32 },
    Rect { min: Vec3, max: Vec3 },
}

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

    /// Все точки эскиза (для AABB).
    pub fn points(&self) -> Vec<Vec3> {
        let mut pts = Vec::new();
        for e in &self.entities {
            match e {
                SketchEntity::Line { a, b } => {
                    pts.push(*a);
                    pts.push(*b);
                }
                SketchEntity::Circle { center, radius } => {
                    pts.push(*center + Vec3::new(*radius, 0.0, 0.0));
                    pts.push(*center - Vec3::new(*radius, 0.0, 0.0));
                    pts.push(*center + Vec3::new(0.0, *radius, 0.0));
                    pts.push(*center - Vec3::new(0.0, *radius, 0.0));
                }
                SketchEntity::Arc { center, radius, .. } => {
                    pts.push(*center + Vec3::new(*radius, 0.0, 0.0));
                    pts.push(*center - Vec3::new(*radius, 0.0, 0.0));
                }
                SketchEntity::Rect { min, max } => {
                    pts.push(*min);
                    pts.push(*max);
                }
            }
        }
        pts
    }

    /// Контуры профиля: список замкнутых полилиний (2D, локальные координаты).
    /// Линии, соединённые в цепочки, объединяются; окружности — отдельные контуры.
    pub fn profiles(&self) -> Vec<Vec<Vec2>> {
        // Собираем сегменты.
        let mut segments: Vec<(Vec2, Vec2)> = Vec::new();
        let mut circles: Vec<Vec<Vec2>> = Vec::new();
        for e in &self.entities {
            match e {
                SketchEntity::Line { a, b } => {
                    segments.push((Vec2::new(a.x, a.y), Vec2::new(b.x, b.y)));
                }
                SketchEntity::Rect { min, max } => {
                    let (x0, y0, x1, y1) = (min.x, min.y, max.x, max.y);
                    segments.push((Vec2::new(x0, y0), Vec2::new(x1, y0)));
                    segments.push((Vec2::new(x1, y0), Vec2::new(x1, y1)));
                    segments.push((Vec2::new(x1, y1), Vec2::new(x0, y1)));
                    segments.push((Vec2::new(x0, y1), Vec2::new(x0, y0)));
                }
                SketchEntity::Circle { center, radius } => {
                    let n = 48;
                    let mut pts = Vec::with_capacity(n);
                    for i in 0..n {
                        let a = (i as f32 / n as f32) * TAU;
                        pts.push(Vec2::new(center.x + a.cos() * radius, center.y + a.sin() * radius));
                    }
                    circles.push(pts);
                }
                SketchEntity::Arc { center, radius, start_angle, end_angle } => {
                    let mut sweep = end_angle - start_angle;
                    while sweep <= 0.0 {
                        sweep += TAU;
                    }
                    let n = ((sweep / (5.0 * DEG2RAD)).ceil() as usize).clamp(4, 128);
                    let mut pts = Vec::with_capacity(n + 1);
                    for i in 0..=n {
                        let a = start_angle + sweep * (i as f32 / n as f32);
                        pts.push(Vec2::new(center.x + a.cos() * radius, center.y + a.sin() * radius));
                    }
                    // Дуга не замкнута — добавляем как сегменты цепочки.
                    for i in 0..pts.len() - 1 {
                        segments.push((pts[i], pts[i + 1]));
                    }
                }
            }
        }
        // Сшиваем сегменты в цепочки по совпадению концов.
        let mut chains: Vec<Vec<Vec2>> = Vec::new();
        let mut segs = segments;
        while let Some((a, b)) = segs.pop() {
            let mut chain = vec![a, b];
            let mut grew = true;
            while grew {
                grew = false;
                let mut i = 0;
                while i < segs.len() {
                    let (s, e) = segs[i];
                    if e.distance(*chain.last().unwrap()) < 1e-6 {
                        chain.push(s);
                        segs.remove(i);
                        grew = true;
                    } else if s.distance(*chain.last().unwrap()) < 1e-6 {
                        chain.push(e);
                        segs.remove(i);
                        grew = true;
                    } else if e.distance(*chain.first().unwrap()) < 1e-6 {
                        chain.insert(0, s);
                        segs.remove(i);
                        grew = true;
                    } else if s.distance(*chain.first().unwrap()) < 1e-6 {
                        chain.insert(0, e);
                        segs.remove(i);
                        grew = true;
                    } else {
                        i += 1;
                    }
                }
            }
            // Замкнутая цепочка: первый == последний.
            if chain.len() > 1 && chain.first().unwrap().distance(*chain.last().unwrap()) < 1e-6 {
                chain.pop();
            }
            if chain.len() >= 3 {
                chains.push(chain);
            }
        }
        chains.extend(circles);
        chains
    }

    /// Выдавливание профиля на `distance` вдоль нормали плоскости.
    /// Возвращает меш в мировых координатах.
    pub fn extrude(&self, distance: f32) -> Option<Mesh> {
        let profiles = self.profiles();
        if profiles.is_empty() {
            return None;
        }
        // Определяем внешний контур (наибольший по площади) и отверстия.
        let mut areas: Vec<(usize, f32)> = profiles
            .iter()
            .enumerate()
            .map(|(i, p)| (i, signed_area(p).abs()))
            .collect();
        areas.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let outer_idx = areas[0].0;
        let outer = &profiles[outer_idx];
        let holes: Vec<&Vec<Vec2>> = profiles
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != outer_idx)
            .map(|(_, p)| p)
            .collect();

        // Триангуляция профиля в 2D (ear clipping с мостами).
        let tris2d = triangulate_profile(outer, &holes)?;
        let (_, _, _, normal) = self.plane.frame();

        let mut mesh = Mesh::new();
        // Вершины крышек.
        let bottom_base = mesh.positions.len() as u32;
        for p in &tris2d.verts {
            mesh.add_vertex(self.plane.to_world(*p));
        }
        let top_base = mesh.positions.len() as u32;
        for p in &tris2d.verts {
            mesh.add_vertex(self.plane.to_world(*p) + normal * distance);
        }
        // Крышки.
        for t in &tris2d.tris {
            // Нижняя (нормаль -normal): обратный порядок.
            mesh.add_triangle(bottom_base + t[0], bottom_base + t[2], bottom_base + t[1]);
            // Верхняя.
            mesh.add_triangle(top_base + t[0], top_base + t[1], top_base + t[2]);
        }
        // Боковые стенки по всем контурам.
        let mut contours: Vec<(&Vec<Vec2>, bool)> = vec![(outer, false)];
        contours.extend(holes.iter().map(|h| (*h, true)));
        for (contour, is_hole) in contours {
            // Внешний контур должен идти CCW, отверстия — CW (стенки наружу от материала).
            let ccw = signed_area(contour) > 0.0;
            let flip = ccw == is_hole;
            let n = contour.len();
            for i in 0..n {
                let j = (i + 1) % n;
                let b0 = bottom_base + contour_index(&tris2d.verts, contour[i]);
                let b1 = bottom_base + contour_index(&tris2d.verts, contour[j]);
                let t0 = top_base + contour_index(&tris2d.verts, contour[i]);
                let t1 = top_base + contour_index(&tris2d.verts, contour[j]);
                if flip {
                    mesh.add_triangle(b0, t1, b1);
                    mesh.add_triangle(b0, t0, t1);
                } else {
                    mesh.add_triangle(b0, b1, t1);
                    mesh.add_triangle(b0, t1, t0);
                }
            }
        }
        mesh.weld();
        mesh.compute_smooth_normals();
        mesh.ensure_outward();
        Some(mesh)
    }
}

fn contour_index(verts: &[Vec2], p: Vec2) -> u32 {
    verts
        .iter()
        .position(|v| v.distance(p) < 1e-6)
        .unwrap_or(0) as u32
}

fn signed_area(poly: &[Vec2]) -> f32 {
    let mut s = 0.0;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        s += a.x * b.y - b.x * a.y;
    }
    s * 0.5
}

struct Triangulation {
    verts: Vec<Vec2>,
    tris: Vec<[u32; 3]>,
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

fn ear_clip(poly: &[Vec2]) -> Vec<usize> {
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
}
