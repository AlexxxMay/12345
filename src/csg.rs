//! Булевы операции над замкнутыми мешами (BSP-деревья, двойная точность).
//!
//! Алгоритм — классическая конструктивная блочная геометрия на BSP-деревьях:
//! многоугольники одного тела отсекаются деревом другого. Вычисления в f64,
//! допуск задаётся относительно габарита сцены. Рекурсия выполняется в потоке
//! с увеличенным стеком, поэтому глубокие деревья не переполняют стек.

use crate::math::Vec3;
use crate::mesh::Mesh;

#[derive(Clone, Copy, Debug, PartialEq)]
struct V {
    x: f64,
    y: f64,
    z: f64,
}

impl V {
    fn new(x: f64, y: f64, z: f64) -> V {
        V { x, y, z }
    }
    fn sub(self, o: V) -> V {
        V::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    fn add(self, o: V) -> V {
        V::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
    fn mul(self, k: f64) -> V {
        V::new(self.x * k, self.y * k, self.z * k)
    }
    fn dot(self, o: V) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    fn cross(self, o: V) -> V {
        V::new(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }
    fn len(self) -> f64 {
        self.dot(self).sqrt()
    }
    fn lerp(self, o: V, t: f64) -> V {
        self.add(o.sub(self).mul(t))
    }
}

#[derive(Clone, Copy, Debug)]
struct Plane {
    n: V,
    w: f64,
}

impl Plane {
    fn from_points(a: V, b: V, c: V) -> Option<Plane> {
        let n = b.sub(a).cross(c.sub(a));
        let l = n.len();
        if l < 1e-18 {
            return None;
        }
        let n = n.mul(1.0 / l);
        Some(Plane { n, w: n.dot(a) })
    }
    fn flip(&mut self) {
        self.n = self.n.mul(-1.0);
        self.w = -self.w;
    }
}

#[derive(Clone, Debug)]
struct Poly {
    v: Vec<V>,
    plane: Plane,
}

impl Poly {
    fn flip(&mut self) {
        self.v.reverse();
        self.plane.flip();
    }
}

const COPLANAR: u8 = 0;
const FRONT: u8 = 1;
const BACK: u8 = 2;
const SPANNING: u8 = 3;

fn split_polygon(pl: &Plane, poly: &Poly, eps: f64, cf: &mut Vec<Poly>, cb: &mut Vec<Poly>, f: &mut Vec<Poly>, b: &mut Vec<Poly>) {
    let mut ptype = 0u8;
    let mut types = Vec::with_capacity(poly.v.len());
    for v in &poly.v {
        let t = pl.n.dot(*v) - pl.w;
        let ty = if t < -eps { BACK } else if t > eps { FRONT } else { COPLANAR };
        ptype |= ty;
        types.push(ty);
    }
    match ptype {
        COPLANAR => {
            if pl.n.dot(poly.plane.n) > 0.0 {
                cf.push(poly.clone());
            } else {
                cb.push(poly.clone());
            }
        }
        FRONT => f.push(poly.clone()),
        BACK => b.push(poly.clone()),
        _ => {
            let mut fv = Vec::new();
            let mut bv = Vec::new();
            let n = poly.v.len();
            for i in 0..n {
                let j = (i + 1) % n;
                let (ti, tj) = (types[i], types[j]);
                let (vi, vj) = (poly.v[i], poly.v[j]);
                if ti != BACK {
                    fv.push(vi);
                }
                if ti != FRONT {
                    bv.push(vi);
                }
                if (ti | tj) == SPANNING {
                    let t = (pl.w - pl.n.dot(vi)) / pl.n.dot(vj.sub(vi));
                    let v = vi.lerp(vj, t);
                    fv.push(v);
                    bv.push(v);
                }
            }
            if fv.len() >= 3 {
                f.push(Poly { v: fv, plane: poly.plane });
            }
            if bv.len() >= 3 {
                b.push(Poly { v: bv, plane: poly.plane });
            }
        }
    }
}

#[derive(Default)]
struct Node {
    plane: Option<Plane>,
    front: Option<Box<Node>>,
    back: Option<Box<Node>>,
    polys: Vec<Poly>,
}

impl Node {
    fn new(polys: Vec<Poly>, eps: f64) -> Node {
        let mut n = Node::default();
        n.build(polys, eps);
        n
    }

    fn invert(&mut self) {
        for p in &mut self.polys {
            p.flip();
        }
        if let Some(pl) = &mut self.plane {
            pl.flip();
        }
        if let Some(f) = &mut self.front {
            f.invert();
        }
        if let Some(b) = &mut self.back {
            b.invert();
        }
        std::mem::swap(&mut self.front, &mut self.back);
    }

    fn clip_polygons(&self, polys: Vec<Poly>, eps: f64) -> Vec<Poly> {
        let Some(pl) = self.plane else { return polys };
        let mut f = Vec::new();
        let mut b = Vec::new();
        for p in &polys {
            let mut cf = Vec::new();
            let mut cb = Vec::new();
            split_polygon(&pl, p, eps, &mut cf, &mut cb, &mut f, &mut b);
            f.extend(cf);
            b.extend(cb);
        }
        let f = match &self.front {
            Some(n) => n.clip_polygons(f, eps),
            None => f,
        };
        let b = match &self.back {
            Some(n) => n.clip_polygons(b, eps),
            None => Vec::new(),
        };
        let mut out = f;
        out.extend(b);
        out
    }

    fn clip_to(&mut self, other: &Node, eps: f64) {
        let polys = std::mem::take(&mut self.polys);
        self.polys = other.clip_polygons(polys, eps);
        if let Some(f) = &mut self.front {
            f.clip_to(other, eps);
        }
        if let Some(b) = &mut self.back {
            b.clip_to(other, eps);
        }
    }

    fn all_polygons(&self, out: &mut Vec<Poly>) {
        out.extend(self.polys.iter().cloned());
        if let Some(f) = &self.front {
            f.all_polygons(out);
        }
        if let Some(b) = &self.back {
            b.all_polygons(out);
        }
    }

    fn build(&mut self, polys: Vec<Poly>, eps: f64) {
        if polys.is_empty() {
            return;
        }
        if self.plane.is_none() {
            // Выбор плоскости разбиения: из небольшой выборки — с минимумом разрезов.
            let mut best = 0;
            if polys.len() > 8 {
                let mut best_score = usize::MAX;
                let step = (polys.len() / 5).max(1);
                for cand in (0..polys.len()).step_by(step).take(5) {
                    let pl = polys[cand].plane;
                    let mut spans = 0;
                    let mut fr = 0i64;
                    for p in polys.iter().step_by((polys.len() / 64).max(1)) {
                        let mut ty = 0u8;
                        for v in &p.v {
                            let t = pl.n.dot(*v) - pl.w;
                            ty |= if t < -eps { BACK } else if t > eps { FRONT } else { COPLANAR };
                        }
                        if ty == SPANNING {
                            spans += 1;
                        } else if ty == FRONT {
                            fr += 1;
                        } else if ty == BACK {
                            fr -= 1;
                        }
                    }
                    let score = spans * 4 + fr.unsigned_abs() as usize;
                    if score < best_score {
                        best_score = score;
                        best = cand;
                    }
                }
            }
            self.plane = Some(polys[best].plane);
        }
        let pl = self.plane.unwrap();
        let mut f = Vec::new();
        let mut b = Vec::new();
        for p in &polys {
            let mut cf = Vec::new();
            let mut cb = Vec::new();
            split_polygon(&pl, p, eps, &mut cf, &mut cb, &mut f, &mut b);
            self.polys.extend(cf);
            self.polys.extend(cb);
        }
        if !f.is_empty() {
            self.front.get_or_insert_with(Default::default).build(f, eps);
        }
        if !b.is_empty() {
            self.back.get_or_insert_with(Default::default).build(b, eps);
        }
    }
}

/// Вид булевой операции.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    Union,
    Subtract,
    Intersect,
}

fn to_polys(m: &Mesh) -> Vec<Poly> {
    let mut out = Vec::with_capacity(m.triangle_count());
    for t in m.indices.chunks_exact(3) {
        let p = |i: u32| {
            let v = m.positions[i as usize];
            V::new(v.x as f64, v.y as f64, v.z as f64)
        };
        let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
        if let Some(plane) = Plane::from_points(a, b, c) {
            out.push(Poly { v: vec![a, b, c], plane });
        }
    }
    out
}

fn from_polys(polys: &[Poly]) -> Mesh {
    let mut m = Mesh::new();
    for p in polys {
        let v0 = p.v[0];
        for k in 1..p.v.len() - 1 {
            let (a, b, c) = (v0, p.v[k], p.v[k + 1]);
            // Пропуск вырожденных.
            if b.sub(a).cross(c.sub(a)).len() < 1e-14 {
                continue;
            }
            let f = |v: V| Vec3::new(v.x as f32, v.y as f32, v.z as f32);
            m.push_tri(f(a), f(b), f(c));
        }
    }
    m.weld();
    fix_t_junctions(&mut m);
    m.compute_smooth_normals();
    m
}

/// Устранение T-стыков: треугольник, на ребре которого лежит чужая вершина,
/// разбивается веером. После этого меш становится замкнутым (водонепроницаемым).
pub fn fix_t_junctions(m: &mut Mesh) {
    use std::collections::HashMap;
    let diag = m.aabb().diagonal().max(1e-6);
    let eps = diag * 1e-6;
    for _pass in 0..8 {
        let mut count: HashMap<(u32, u32), u32> = HashMap::new();
        for t in m.indices.chunks_exact(3) {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                *count.entry(if a < b { (a, b) } else { (b, a) }).or_insert(0) += 1;
            }
        }
        let mut bverts: Vec<u32> = Vec::new();
        for (&(a, b), &c) in &count {
            if c == 1 {
                bverts.push(a);
                bverts.push(b);
            }
        }
        if bverts.is_empty() || bverts.len() > 40_000 {
            return;
        }
        bverts.sort_unstable();
        bverts.dedup();
        let mut changed = false;
        let mut out: Vec<u32> = Vec::with_capacity(m.indices.len() + 64);
        let tris: Vec<[u32; 3]> = m.indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
        for t in tris {
            let mut done = false;
            for k in 0..3 {
                let (a, b, c) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
                let key = if a < b { (a, b) } else { (b, a) };
                if count.get(&key) != Some(&1) {
                    continue;
                }
                let pa = m.positions[a as usize];
                let pb = m.positions[b as usize];
                let d = pb - pa;
                let l2 = d.len2();
                if l2 <= eps * eps {
                    continue;
                }
                let mut on: Vec<(f32, u32)> = Vec::new();
                for &v in &bverts {
                    if v == a || v == b || v == c {
                        continue;
                    }
                    let p = m.positions[v as usize];
                    let tt = (p - pa).dot(d) / l2;
                    if tt <= 1e-5 || tt >= 1.0 - 1e-5 {
                        continue;
                    }
                    let q = pa + d * tt;
                    if q.distance(p) < eps * 10.0 {
                        on.push((tt, v));
                    }
                }
                if on.is_empty() {
                    continue;
                }
                on.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
                let mut prev = a;
                for &(_, v) in &on {
                    out.extend_from_slice(&[prev, v, c]);
                    prev = v;
                }
                out.extend_from_slice(&[prev, b, c]);
                done = true;
                changed = true;
                break;
            }
            if !done {
                out.extend_from_slice(&t);
            }
        }
        m.indices = out;
        if !changed {
            return;
        }
    }
}

fn run(a: &Mesh, b: &Mesh, op: BoolOp) -> Mesh {
    let mut bb = a.aabb();
    bb.union(&b.aabb());
    let eps = (bb.diagonal() as f64 * 1e-7).max(1e-9);
    let mut na = Node::new(to_polys(a), eps);
    let mut nb = Node::new(to_polys(b), eps);
    match op {
        BoolOp::Union => {
            na.clip_to(&nb, eps);
            nb.clip_to(&na, eps);
            nb.invert();
            nb.clip_to(&na, eps);
            nb.invert();
            let mut all = Vec::new();
            nb.all_polygons(&mut all);
            na.build(all, eps);
        }
        BoolOp::Subtract => {
            na.invert();
            na.clip_to(&nb, eps);
            nb.clip_to(&na, eps);
            nb.invert();
            nb.clip_to(&na, eps);
            nb.invert();
            let mut all = Vec::new();
            nb.all_polygons(&mut all);
            na.build(all, eps);
            na.invert();
        }
        BoolOp::Intersect => {
            na.invert();
            nb.clip_to(&na, eps);
            nb.invert();
            na.clip_to(&nb, eps);
            nb.clip_to(&na, eps);
            let mut all = Vec::new();
            nb.all_polygons(&mut all);
            na.build(all, eps);
            na.invert();
        }
    }
    let mut polys = Vec::new();
    na.all_polygons(&mut polys);
    from_polys(&polys)
}

/// Булева операция над двумя мешами в одной системе координат.
pub fn boolean(a: &Mesh, b: &Mesh, op: BoolOp) -> Result<Mesh, String> {
    if a.is_empty() || b.is_empty() {
        return Err("пустой меш".into());
    }
    let a = a.clone();
    let b = b.clone();
    let handle = std::thread::Builder::new()
        .name("csg".into())
        .stack_size(512 * 1024 * 1024)
        .spawn(move || run(&a, &b, op))
        .map_err(|e| format!("не удалось запустить поток: {e}"))?;
    handle.join().map_err(|_| "ошибка вычисления булевой операции".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::primitive_box;

    fn moved(mut m: Mesh, d: Vec3) -> Mesh {
        m.translate(d);
        m
    }

    #[test]
    fn box_boolean_volumes() {
        let a = primitive_box(2.0, 2.0, 2.0); // z ∈ [0,2], x,y ∈ [-1,1]
        let b = moved(primitive_box(2.0, 2.0, 2.0), Vec3::new(1.0, 0.0, 0.0));
        let u = boolean(&a, &b, BoolOp::Union).unwrap();
        let s = boolean(&a, &b, BoolOp::Subtract).unwrap();
        let i = boolean(&a, &b, BoolOp::Intersect).unwrap();
        assert!((u.volume() - 12.0).abs() < 1e-3, "union {}", u.volume());
        assert!((s.volume() - 4.0).abs() < 1e-3, "sub {}", s.volume());
        assert!((i.volume() - 4.0).abs() < 1e-3, "int {}", i.volume());
        assert!(u.is_closed() && s.is_closed() && i.is_closed());
    }

    #[test]
    fn hole_through_box() {
        let a = primitive_box(10.0, 10.0, 10.0);
        let mut c = crate::mesh::primitive_cylinder(2.0, 20.0, 32);
        c.translate(Vec3::new(0.0, 0.0, -5.0));
        let r = boolean(&a, &c, BoolOp::Subtract).unwrap();
        let hole = std::f32::consts::PI * 4.0 * 10.0;
        // Многоугольное приближение окружности чуть меньше круга.
        assert!((r.volume() - (1000.0 - hole)).abs() < 3.0, "vol={}", r.volume());
        assert!(r.is_closed());
    }
}
